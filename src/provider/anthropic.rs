//! Anthropic Messages-API wire shapes (summarisation only).
//!
//! Phase 12 / US1. Response structs are LENIENT — they must NOT reject unknown
//! fields (FR-021; enforced by a file-scoped grep gate). Auth (`x-api-key` +
//! `anthropic-version`) is placed by [`crate::provider::http`] for
//! kind=anthropic. Bodies are never logged above DEBUG.

use serde::{Deserialize, Serialize};

use crate::provider::config::ResolvedProvider;
use crate::provider::error::{ProviderError, ProviderErrorKind};
use crate::provider::http;

/// A generous default for the Messages API `max_tokens` (required). The
/// summariser's two passes are short; this is comfortable headroom and the
/// provider stops at the model's stop sequence well before the cap.
const MAX_TOKENS: u32 = 1024;

#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    /// `system` is a TOP-LEVEL param on the Messages API (not a message).
    /// Omitted on the wire when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<Message<'a>>,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

/// The success response. `content` is a list of content blocks; we read the
/// first `text` block's `text`. LENIENT.
#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    /// Anthropic can return `{"type":"error", "error":{...}}` — surfaced as a
    /// BadRequest when there is no usable text.
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
}

/// One Messages round-trip. Returns the first content block's text
/// (`content[0].text`).
pub fn chat(
    resolved: &ResolvedProvider,
    system: Option<&str>,
    user: &str,
) -> Result<String, ProviderError> {
    let body = serde_json::to_value(MessagesRequest {
        model: &resolved.model,
        max_tokens: MAX_TOKENS,
        system,
        messages: vec![Message {
            role: "user",
            content: user,
        }],
    })
    .map_err(|e| {
        ProviderError::new(
            &resolved.name,
            ProviderErrorKind::BadRequest,
            false,
            format!("failed to serialise messages request: {e}"),
        )
    })?;

    let value = http::request_with_retry(resolved, "/v1/messages", &body)?;
    extract_message_text(&resolved.name, value)
}

fn extract_message_text(provider: &str, value: serde_json::Value) -> Result<String, ProviderError> {
    let response: MessagesResponse = serde_json::from_value(value).map_err(|e| {
        ProviderError::new(
            provider,
            ProviderErrorKind::MalformedResponse,
            false,
            format!("messages response did not match the expected shape: {e}"),
        )
    })?;

    if let Some(text) = response.content.into_iter().find_map(|b| b.text) {
        return Ok(text);
    }

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
        "messages response had no content[].text",
    ))
}
