//! Google Gemini generateContent wire shapes (summarisation only).
//!
//! Phase 12 / US1. Response structs are LENIENT — they must NOT reject unknown
//! fields (FR-021; enforced by a file-scoped grep gate). The `?key=<k>` query is
//! appended by [`crate::provider::http`] for kind=gemini. A safety block (a 2xx
//! with NO `candidates`) is a `BadRequest`, never an empty success. Bodies are
//! never logged above DEBUG.

use serde::{Deserialize, Serialize};

use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};
use crate::provider::http;

#[derive(Debug, Serialize)]
struct GenerateContentRequest<'a> {
    contents: Vec<Content<'a>>,
    /// `systemInstruction` is a sibling of `contents`. Omitted on the wire when
    /// `None`.
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content<'a>>,
}

#[derive(Debug, Serialize)]
struct Content<'a> {
    parts: Vec<Part<'a>>,
}

#[derive(Debug, Serialize)]
struct Part<'a> {
    text: &'a str,
}

/// The success response. `candidates[0].content.parts[0].text` carries the
/// reply. LENIENT.
#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    /// A top-level `error` object can ride a 2xx on some compatible servers.
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<RespContent>,
}

#[derive(Debug, Deserialize)]
struct RespContent {
    #[serde(default)]
    parts: Vec<RespPart>,
}

#[derive(Debug, Deserialize)]
struct RespPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
}

/// One generateContent round-trip. Returns
/// `candidates[0].content.parts[0].text`. A safety block (no candidates) →
/// `BadRequest`/94.
pub fn chat(
    resolved: &ResolvedProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, ProviderError> {
    let body = serde_json::to_value(GenerateContentRequest {
        contents: vec![Content {
            parts: vec![Part { text: user }],
        }],
        system_instruction: system.map(|s| Content {
            parts: vec![Part { text: s }],
        }),
    })
    .map_err(|e| {
        ProviderError::new(
            &resolved.name,
            ProviderErrorKind::BadRequest,
            false,
            format!("failed to serialise generateContent request: {e}"),
        )
    })?;

    // The model id is part of the PATH for Gemini; the `?key=` is appended by
    // http.rs.
    let path = format!("/v1beta/models/{}:generateContent", resolved.model);
    let value = http::request_with_retry(resolved, &path, &body)?;
    extract_candidate_text(&resolved.name, value)
}

fn extract_candidate_text(
    provider: &str,
    value: serde_json::Value,
) -> Result<String, ProviderError> {
    let response: GenerateContentResponse = serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("generateContent response did not match the expected shape: {e}"),
        )
    })?;

    if let Some(text) = response
        .candidates
        .into_iter()
        .find_map(|c| c.content)
        .and_then(|content| content.parts.into_iter().find_map(|p| p.text))
    {
        return Ok(text);
    }

    // No candidate text. A surfaced error object wins its message; otherwise
    // this is a safety block (no candidates) — both are BadRequest, never an
    // empty success (per the contract).
    let detail = response
        .error
        .and_then(|e| e.message)
        .unwrap_or_else(|| "no candidates returned (safety block or empty response)".to_string());
    Err(ProviderError::new(
        provider,
        ProviderErrorKind::BadRequest,
        false,
        detail,
    ))
}
