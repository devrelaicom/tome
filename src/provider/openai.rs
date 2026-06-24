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
