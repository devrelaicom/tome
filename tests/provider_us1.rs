//! Phase 12 / US1 — provider chat (summarisation) integration tests.
//!
//! Two layers, both over the `set_transport_override` seam so no network is
//! touched:
//!
//! - **T020** — per-kind chat request shaping + response parsing
//!   (openai/anthropic/gemini): the override inspects the `RequestSpec` (URL
//!   path, body JSON, auth header/query) and returns a canned `RawResponse`;
//!   the test asserts the extracted text. Covers the gemini safety-block and
//!   200-with-error-envelope → `BadRequest` cases.
//! - **T021** — `RemoteSummariser` end-to-end over the seam: a non-empty
//!   short+long pair → `Ok`; empty provider content → `SummariserFailure`
//!   (exit 24, NOT 94); and the `build_summariser` selection logic (remote
//!   when `[summariser] provider` is set, bundled otherwise).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tempfile::TempDir;
use tome::config::{Config, ProviderEntry, ProviderKind, Secret};
use tome::error::TomeError;
use tome::paths::Paths;
use tome::provider::config::{Capability, ResolvedProvider, resolve};
use tome::provider::error::ProviderErrorKind;
use tome::provider::http::{RawResponse, RequestSpec, set_transport_override};
use tome::provider::{anthropic, gemini, openai};
use tome::summarise::{
    LONG_MAX_CHARS, PluginSummariesInput, PluginSummaryItem, RemoteSummariser, SkillSummaryItem,
    Summariser, build_summariser,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `ResolvedProvider` for the named kind through the real `resolve`
/// path (so the credential newtype is constructed legitimately). The inline
/// key seeds auth; the test then asserts auth placement off the `RequestSpec`.
fn resolved(kind: ProviderKind, key: Option<&str>) -> ResolvedProvider {
    let mut config = Config::default();
    config.providers.insert(
        "p".to_string(),
        ProviderEntry {
            kind,
            base_url: None,
            api_key: key.map(|k| Secret::from(k.to_string())),
        },
    );
    config.summariser.provider = Some("p".to_string());
    config.summariser.model = Some("test-model".to_string());
    resolve(&config, Capability::Summariser)
        .expect("resolve ok")
        .expect("provider referenced")
}

fn ok_json(value: serde_json::Value) -> RawResponse {
    RawResponse {
        status: 200,
        retry_after: None,
        body: serde_json::to_vec(&value).unwrap(),
    }
}

fn body_json(spec: &RequestSpec) -> serde_json::Value {
    serde_json::from_slice(&spec.body).expect("request body is valid JSON")
}

// ---------------------------------------------------------------------------
// T020 — openai chat
// ---------------------------------------------------------------------------

#[test]
fn openai_chat_shapes_request_and_parses_choices_content() {
    let _guard = set_transport_override(|spec| {
        // Path.
        assert!(
            spec.url.ends_with("/chat/completions"),
            "openai chat must POST /chat/completions: {}",
            spec.url
        );
        // Auth header (Bearer for openai).
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer sk-key"),
            "openai chat must carry a Bearer header"
        );
        // Body shape: model + a system+user message pair + stream:false.
        let body = body_json(spec);
        assert_eq!(body["model"], serde_json::json!("test-model"));
        assert_eq!(body["stream"], serde_json::json!(false));
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2, "system + user");
        assert_eq!(messages[0]["role"], serde_json::json!("system"));
        assert_eq!(messages[0]["content"], serde_json::json!("be terse"));
        assert_eq!(messages[1]["role"], serde_json::json!("user"));
        assert_eq!(messages[1]["content"], serde_json::json!("hello"));
        Ok(ok_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "the answer" } }]
        })))
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    let out = openai::chat(&r, Some("be terse"), "hello").unwrap();
    assert_eq!(out, "the answer");
}

#[test]
fn openai_chat_omits_system_when_none() {
    let _guard = set_transport_override(|spec| {
        let body = body_json(spec);
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1, "user only when system is None");
        assert_eq!(messages[0]["role"], serde_json::json!("user"));
        Ok(ok_json(serde_json::json!({
            "choices": [{ "message": { "content": "x" } }]
        })))
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    assert_eq!(openai::chat(&r, None, "hi").unwrap(), "x");
}

#[test]
fn openai_chat_200_error_envelope_is_bad_request() {
    // A 200 carrying an error object and NO choices → BadRequest (not empty
    // success).
    let _guard = set_transport_override(|_spec| {
        Ok(ok_json(serde_json::json!({
            "error": { "message": "context length exceeded", "type": "invalid_request_error" }
        })))
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    let err = openai::chat(&r, None, "hi").unwrap_err();
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(!err.retryable);
    // The provider's error message rides along in the redacted detail.
    assert!(
        err.redacted_detail.contains("context length exceeded"),
        "detail should carry the provider error message: {}",
        err.redacted_detail
    );
}

// ---------------------------------------------------------------------------
// T020 — anthropic chat
// ---------------------------------------------------------------------------

#[test]
fn anthropic_chat_shapes_request_and_parses_content_text() {
    let _guard = set_transport_override(|spec| {
        assert!(
            spec.url.ends_with("/v1/messages"),
            "anthropic chat must POST /v1/messages: {}",
            spec.url
        );
        // anthropic auth: x-api-key + anthropic-version (placed by http.rs).
        assert!(
            spec.headers
                .iter()
                .any(|(k, v)| k == "x-api-key" && v == "ant-key"),
            "anthropic chat must carry x-api-key"
        );
        assert!(
            spec.headers.iter().any(|(k, _)| k == "anthropic-version"),
            "anthropic chat must carry anthropic-version"
        );
        // No Bearer for anthropic.
        assert!(!spec.headers.iter().any(|(k, _)| k == "Authorization"));
        let body = body_json(spec);
        assert_eq!(body["model"], serde_json::json!("test-model"));
        assert!(
            body["max_tokens"].is_number(),
            "anthropic body must carry max_tokens"
        );
        // system is a top-level param (not a message).
        assert_eq!(body["system"], serde_json::json!("be terse"));
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], serde_json::json!("user"));
        assert_eq!(messages[0]["content"], serde_json::json!("hello"));
        Ok(ok_json(serde_json::json!({
            "content": [{ "type": "text", "text": "anthropic says hi" }]
        })))
    });
    let r = resolved(ProviderKind::Anthropic, Some("ant-key"));
    let out = anthropic::chat(&r, Some("be terse"), "hello").unwrap();
    assert_eq!(out, "anthropic says hi");
}

#[test]
fn anthropic_chat_omits_system_param_when_none() {
    let _guard = set_transport_override(|spec| {
        let body = body_json(spec);
        assert!(
            body.get("system").is_none() || body["system"].is_null(),
            "system param must be omitted when None"
        );
        Ok(ok_json(serde_json::json!({
            "content": [{ "type": "text", "text": "ok" }]
        })))
    });
    let r = resolved(ProviderKind::Anthropic, Some("ant-key"));
    assert_eq!(anthropic::chat(&r, None, "hi").unwrap(), "ok");
}

// ---------------------------------------------------------------------------
// T020 — gemini chat
// ---------------------------------------------------------------------------

#[test]
fn gemini_chat_shapes_request_and_parses_candidates_text() {
    let _guard = set_transport_override(|spec| {
        // Path: …/models/<model>:generateContent ; key appended by http.rs.
        assert!(
            spec.url
                .contains("/v1beta/models/test-model:generateContent"),
            "gemini chat path must target the model's generateContent: {}",
            spec.url
        );
        assert!(
            spec.url.contains("key=g-key"),
            "gemini chat must carry the ?key= query: {}",
            spec.url
        );
        // No auth header for gemini.
        assert!(!spec.headers.iter().any(|(k, _)| k == "Authorization"));
        assert!(!spec.headers.iter().any(|(k, _)| k == "x-api-key"));
        let body = body_json(spec);
        assert_eq!(
            body["contents"][0]["parts"][0]["text"],
            serde_json::json!("hello")
        );
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            serde_json::json!("be terse")
        );
        Ok(ok_json(serde_json::json!({
            "candidates": [{ "content": { "parts": [{ "text": "gemini reply" }] } }]
        })))
    });
    let r = resolved(ProviderKind::Gemini, Some("g-key"));
    let out = gemini::chat(&r, Some("be terse"), "hello").unwrap();
    assert_eq!(out, "gemini reply");
}

#[test]
fn gemini_chat_omits_system_instruction_when_none() {
    let _guard = set_transport_override(|spec| {
        let body = body_json(spec);
        assert!(
            body.get("systemInstruction").is_none() || body["systemInstruction"].is_null(),
            "systemInstruction must be omitted when None"
        );
        Ok(ok_json(serde_json::json!({
            "candidates": [{ "content": { "parts": [{ "text": "ok" }] } }]
        })))
    });
    let r = resolved(ProviderKind::Gemini, Some("g-key"));
    assert_eq!(gemini::chat(&r, None, "hi").unwrap(), "ok");
}

#[test]
fn gemini_chat_safety_block_no_candidates_is_bad_request() {
    // A safety-blocked response carries NO candidates (only promptFeedback). The
    // contract requires this be a BadRequest, never an empty success.
    let _guard = set_transport_override(|_spec| {
        Ok(ok_json(serde_json::json!({
            "promptFeedback": { "blockReason": "SAFETY" }
        })))
    });
    let r = resolved(ProviderKind::Gemini, Some("g-key"));
    let err = gemini::chat(&r, None, "hi").unwrap_err();
    assert_eq!(err.kind, ProviderErrorKind::BadRequest);
    assert!(!err.retryable);
}

// ---------------------------------------------------------------------------
// T021 — RemoteSummariser
// ---------------------------------------------------------------------------

/// A small two-skill input that renders to a non-empty description block.
fn sample_input() -> PluginSummariesInput {
    PluginSummariesInput {
        plugins: vec![PluginSummaryItem {
            catalog: "core".to_string(),
            plugin: "alpha".to_string(),
            description: String::new(),
            skills: vec![SkillSummaryItem {
                name: "skill-one".to_string(),
                description: "describes one".to_string(),
            }],
        }],
    }
}

#[test]
fn remote_summariser_two_passes_returns_non_empty_pair() {
    // Both passes return content. The override returns a per-call canned reply;
    // the second pass (long) cascades off the short, but the content the
    // provider returns is what's asserted.
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let _guard = set_transport_override(move |spec| {
        let n = c.fetch_add(1, Ordering::SeqCst);
        // Both passes carry a non-empty user message (the prompt).
        let body = body_json(spec);
        let user = body["messages"][1]["content"]
            .as_str()
            .or_else(|| body["messages"][0]["content"].as_str())
            .unwrap_or("");
        assert!(!user.is_empty(), "prompt must be non-empty");
        let text = if n == 0 {
            "short topics"
        } else {
            "the long rules section"
        };
        Ok(ok_json(serde_json::json!({
            "choices": [{ "message": { "content": text } }]
        })))
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    let summariser = RemoteSummariser::new(r);
    let out = summariser
        .summarise(&sample_input(), LONG_MAX_CHARS)
        .expect("summarise ok");
    assert_eq!(out.short, "short topics");
    assert_eq!(out.long, "the long rules section");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "exactly two passes");
}

#[test]
fn remote_summariser_empty_content_is_summariser_failure_exit_24() {
    // The provider returns an empty string → after trim, OutputEmpty(Short),
    // which is exit 24 (a content failure), NOT 94 (a provider request error).
    let _guard = set_transport_override(|_spec| {
        Ok(ok_json(serde_json::json!({
            "choices": [{ "message": { "content": "   " } }]
        })))
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    let summariser = RemoteSummariser::new(r);
    let err = summariser
        .summarise(&sample_input(), LONG_MAX_CHARS)
        .expect_err("empty content must fail");
    assert_eq!(
        err.exit_code(),
        24,
        "empty content is a SummariserFailure (24), not ProviderRequestFailed (94): {err:?}"
    );
    assert!(
        matches!(err, TomeError::SummariserFailure { .. }),
        "{err:?}"
    );
}

#[test]
fn remote_summariser_provider_error_maps_to_94() {
    // A transport-level provider failure (here: a 401) surfaces as
    // ProviderRequestFailed (94), distinct from the empty-content 24.
    let _guard = set_transport_override(|_spec| {
        Ok(RawResponse {
            status: 401,
            retry_after: None,
            body: b"{\"error\":\"bad key\"}".to_vec(),
        })
    });
    let r = resolved(ProviderKind::Openai, Some("sk-key"));
    let summariser = RemoteSummariser::new(r);
    let err = summariser
        .summarise(&sample_input(), LONG_MAX_CHARS)
        .expect_err("401 must fail");
    assert_eq!(err.exit_code(), 94, "provider 401 maps to 94: {err:?}");
}

// ---------------------------------------------------------------------------
// T021 — build_summariser selection logic
// ---------------------------------------------------------------------------

fn paths_in(dir: &TempDir) -> Paths {
    Paths::from_root(dir.path().to_path_buf())
}

#[test]
fn build_summariser_selects_remote_when_provider_configured() {
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    let mut config = Config::default();
    config.providers.insert(
        "p".to_string(),
        ProviderEntry {
            kind: ProviderKind::Openai,
            base_url: None,
            api_key: Some(Secret::from("sk-key".to_string())),
        },
    );
    config.summariser.provider = Some("p".to_string());
    config.summariser.model = Some("gpt-4o-mini".to_string());

    // With a provider configured, build_summariser must produce a remote
    // summariser. We assert that by driving it over the transport seam: a
    // bundled LlamaSummariser would never reach the seam.
    let _guard = set_transport_override(|_spec| {
        Ok(ok_json(serde_json::json!({
            "choices": [{ "message": { "content": "remote text" } }]
        })))
    });
    let summariser = build_summariser(&config, &paths, false).expect("build remote");
    let out = summariser
        .summarise(&sample_input(), LONG_MAX_CHARS)
        .expect("remote summarise");
    assert_eq!(out.short, "remote text");
}

#[test]
fn build_summariser_uses_bundled_when_no_provider() {
    // No provider configured → resolve() returns Ok(None) → build_summariser
    // constructs the bundled LlamaSummariser. The model GGUF is absent in this
    // temp HOME, so LlamaSummariser::new returns ModelMissing — which proves the
    // BUNDLED branch was taken (a remote build would have succeeded with no
    // model on disk).
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    let config = Config::default();
    // `Box<dyn Summariser>` isn't Debug, so `expect_err` won't compile; match.
    match build_summariser(&config, &paths, false) {
        Ok(_) => panic!("no-provider build should take the bundled path and fail (model absent)"),
        Err(TomeError::SummariserFailure {
            kind: tome::error::SummariserFailureKind::ModelMissing,
        }) => {}
        Err(other) => {
            panic!("no-provider build must take the bundled path (ModelMissing here): {other:?}")
        }
    }
}
