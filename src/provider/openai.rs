//! OpenAI-compatible wire shapes (chat completions + embeddings).
//!
//! Phase 12 / US1 lands the **chat** path used by the remote summariser. The
//! response structs are LENIENT — they must NOT reject unknown fields (FR-021;
//! enforced by a file-scoped grep gate under `tests/harness_settings/`): a
//! provider adding a field must not break our parse. Auth
//! (`Authorization: Bearer`) is placed by [`crate::provider::http`], so this
//! module only shapes the body and extracts the text. Bodies are never logged
//! above DEBUG.

use serde::{Deserialize, Serialize};

use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};
use crate::provider::http;

/// One chat message (`{role, content}`). Used on the request side only.
#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// The chat-completions request body. `stream` is always `false` (we want the
/// whole completion, not a token stream — v1 has no streaming).
#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

/// The success-shape response. LENIENT — only the fields we read.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    /// An OpenAI-style error object can ride a 200 (some compatible servers do
    /// this). Present + no usable `choices` ⇒ `BadRequest`.
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChoiceMessage>,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

/// A provider-side error object (`{ "message": "...", ... }`). LENIENT.
#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
}

// ---------------------------------------------------------------------------
// Embeddings (single-text, one request per `embed()`). Shared with Voyage —
// the two providers use the identical `{ "data": [{ "embedding": [..] }] }`
// success shape (only Voyage RERANK differs, and that is US3, not here). The
// per-kind wrapper (`voyage::embed_one`) re-dispatches here.
// ---------------------------------------------------------------------------

/// The embeddings request body. `input` is ALWAYS a single-element array
/// (FR-011: one text per request → positional batch-misalignment is
/// structurally impossible). `dimensions` is emitted only when the caller
/// supplies an authoritative `[embedding] dimensions` (openai supports it; a
/// provider that ignores it simply returns its native dimension, which the
/// shared validator then asserts against the persisted value).
#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: [&'a str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

/// The embeddings success response. LENIENT — only `data[].embedding`, plus the
/// optional 200-with-error envelope some compatible gateways emit. `embedding`
/// is `Vec<f32>`: serde parses each JSON number into `f32` directly, so an
/// integer-typed value (Voyage int8/dtype surprise) round-trips to its `f32`
/// magnitude and is caught by the shared finite/dimension validator downstream.
#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<EmbeddingData>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    #[serde(default)]
    embedding: Vec<f32>,
}

/// Embed exactly one text against an OpenAI-compatible (or Voyage) `/embeddings`
/// endpoint. Returns the RAW vector — content validation (non-empty / finite /
/// dimension) is centralised in [`crate::embedding::remote::validate_embedding`]
/// so the SAME checks run at index-time AND query-time, CLI AND MCP. This
/// function asserts only the structural single-embedding contract (FR-011):
/// the response MUST carry exactly one embedding (a `data.len() != 1` is a
/// `MalformedResponse`/94, never a silent first-of-many).
///
/// `dimensions` is the OpenAI `dimensions` body field; the Voyage wrapper maps
/// it to `output_dimension` before delegating here.
pub fn embed_one(
    resolved: &ResolvedProvider,
    text: &str,
    dimensions: Option<u32>,
) -> Result<Vec<f32>, ProviderError> {
    embed_with_dimensions_field(resolved, text, dimensions, EmbeddingDimensionsField::Openai)
}

/// Which JSON field name carries the requested output dimension for this kind.
/// OpenAI uses `dimensions`; Voyage uses `output_dimension`. Kept here (rather
/// than in `voyage.rs`) so the single request-shaping + parse path is the SSOT.
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingDimensionsField {
    Openai,
    Voyage,
}

/// Shared embeddings round-trip. Voyage's wrapper passes
/// [`EmbeddingDimensionsField::Voyage`] so the dimension knob serialises as
/// `output_dimension`; everything else (path, auth via `http`, single-element
/// input, single-embedding assertion) is identical.
pub fn embed_with_dimensions_field(
    resolved: &ResolvedProvider,
    text: &str,
    dimensions: Option<u32>,
    field: EmbeddingDimensionsField,
) -> Result<Vec<f32>, ProviderError> {
    // Build the body. For openai the `dimensions` key lives on the typed
    // struct; for voyage we rename it to `output_dimension` after serialising
    // (the request struct is a private detail — a post-serialise key swap keeps
    // one struct rather than two near-identical ones).
    let mut body = serde_json::to_value(EmbeddingRequest {
        model: &resolved.model,
        input: [text],
        dimensions,
    })
    .map_err(|e| {
        ProviderError::new(
            &resolved.name,
            ProviderErrorKind::BadRequest,
            false,
            format!("failed to serialise embeddings request: {e}"),
        )
    })?;
    if let (EmbeddingDimensionsField::Voyage, Some(d)) = (field, dimensions)
        && let Some(obj) = body.as_object_mut()
    {
        obj.remove("dimensions");
        obj.insert("output_dimension".to_string(), serde_json::json!(d));
    }

    let value = http::request_with_retry(resolved, "/embeddings", &body)?;
    extract_one_embedding(&resolved.name, value)
}

/// Extract exactly one embedding vector from a parsed embeddings response,
/// detecting the 200-with-error envelope and enforcing the single-embedding
/// structural contract (FR-011). Returns the RAW vector; content validation is
/// the caller's (shared validator's) job.
fn extract_one_embedding(
    provider: &str,
    value: serde_json::Value,
) -> Result<Vec<f32>, ProviderError> {
    let response: EmbeddingResponse = serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("embeddings response did not match the expected shape: {e}"),
        )
    })?;

    // A 200-with-error envelope and no usable data ⇒ BadRequest (per-kind
    // detection, FR-013b) — never a silent empty success.
    if response.data.is_empty()
        && let Some(err) = response.error
    {
        let detail = err
            .message
            .unwrap_or_else(|| "provider returned an error object on a 2xx".to_string());
        return Err(ProviderError::new(
            provider,
            ProviderErrorKind::BadRequest,
            false,
            detail,
        ));
    }

    // FR-011: the single-text request MUST yield exactly one embedding. A
    // `data.len() != 1` is a malformed response — refuse rather than pick the
    // first of many (which is the batch-misalignment hazard the single-text
    // design exists to make structurally impossible).
    if response.data.len() != 1 {
        return Err(ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!(
                "expected exactly one embedding for a single-text request, got {}",
                response.data.len()
            ),
        ));
    }

    Ok(response.data.into_iter().next().unwrap().embedding)
}

/// One chat round-trip. `system` is an optional system message; `user` is the
/// user turn. Returns the assistant text (`choices[0].message.content`).
///
/// A 200 carrying an `error` object and no usable `choices` is a
/// `BadRequest`/94 (never an empty success).
pub fn chat(
    resolved: &ResolvedProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, ProviderError> {
    let mut messages = Vec::with_capacity(2);
    if let Some(sys) = system {
        messages.push(ChatMessage {
            role: "system",
            content: sys,
        });
    }
    messages.push(ChatMessage {
        role: "user",
        content: user,
    });
    let body = serde_json::to_value(ChatRequest {
        model: &resolved.model,
        messages,
        stream: false,
    })
    .map_err(|e| {
        ProviderError::new(
            &resolved.name,
            ProviderErrorKind::BadRequest,
            false,
            format!("failed to serialise chat request: {e}"),
        )
    })?;

    let value = http::request_with_retry(resolved, "/chat/completions", &body)?;
    extract_chat_text(&resolved.name, value)
}

/// Extract the assistant text from a parsed chat response, detecting the
/// 200-with-error envelope.
fn extract_chat_text(provider: &str, value: serde_json::Value) -> Result<String, ProviderError> {
    let response: ChatResponse = serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("chat response did not match the expected shape: {e}"),
        )
    })?;

    // First content found wins; this also lets the error-envelope detection
    // below fire only when there's genuinely no usable content.
    if let Some(content) = response
        .choices
        .into_iter()
        .find_map(|c| c.message.and_then(|m| m.content))
    {
        return Ok(content);
    }

    // No usable content. If a 200-with-error envelope is present, surface the
    // provider message as a BadRequest; else a generic malformed response.
    if let Some(err) = response.error {
        let detail = err
            .message
            .unwrap_or_else(|| "provider returned an error object on a 2xx".to_string());
        return Err(ProviderError::new(
            provider,
            ProviderErrorKind::BadRequest,
            false,
            detail,
        ));
    }
    Err(ProviderError::new(
        provider,
        ProviderErrorKind::MalformedResponse,
        false,
        "chat response had no choices[].message.content",
    ))
}
