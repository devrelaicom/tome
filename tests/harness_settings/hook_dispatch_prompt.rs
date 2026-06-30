//! US6.2 — prompt handler dispatch through the BYOM provider path.
//!
//! Uses the provider transport seam (`set_transport_override`) to inject canned
//! LLM responses without touching the network. Exercises the full
//! `dispatch_with_cfg` pipeline with a `Handler::Prompt` entry in the manifest.
//!
//! The internal Tome ↔ model I/O contract: Tome sends the hook's `prompt` text
//! as the system message and the CC event JSON as the user message; the model
//! replies with `{"ok":false,"reason":"…"}` to deny or `{"ok":true}` (or
//! anything else) to allow.
//!
//! ## Fail-open totality (all tests assert this)
//!
//! Any Tome-side fault — provider error, config error, unparsable reply —
//! degrades to a non-blocking allow at exit 0. Tome never blocks the agent
//! because of its own fault.

use tome::commands::harness::run_hook;
use tome::config::{Config, ProviderEntry, ProviderKind};
use tome::harness::hooks_ir::HookManifest;
use tome::provider::http::{RawResponse, TransportFailure, set_transport_override};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `HookManifest` with one `PreToolUse` prompt handler for `cursor`.
fn manifest_with_prompt(prompt: &str) -> HookManifest {
    let prompt_escaped = serde_json::to_string(prompt).expect("escape prompt");
    let json = format!(
        r#"{{
            "harness": "cursor",
            "events": {{
                "PreToolUse": [
                    {{
                        "plugin": "cat:guard",
                        "handler": {{ "type": "prompt", "prompt": {prompt_escaped} }}
                    }}
                ]
            }}
        }}"#
    );
    serde_json::from_str(&json).expect("parse manifest JSON")
}

/// Build a `Config` with one OpenAI-compatible provider pointed at by `[hooks]`.
fn cfg_with_openai_prompt() -> Config {
    let mut cfg = Config::default();
    cfg.providers.insert(
        "myprov".into(),
        ProviderEntry {
            kind: ProviderKind::Openai,
            base_url: None,
            api_key: None,
        },
    );
    cfg.hooks.prompt_provider = Some("myprov".into());
    cfg.hooks.prompt_model = Some("gpt-4o-mini".into());
    cfg
}

/// Construct an OpenAI-shaped `RawResponse` with `content` as the assistant
/// message text. The transport seam returns this as the raw bytes that
/// `openai::chat` will parse.
fn openai_ok_response(content: &str) -> RawResponse {
    let body = format!(
        r#"{{"choices":[{{"message":{{"content":{content_json}}}}}]}}"#,
        content_json = serde_json::to_string(content).expect("escape content")
    );
    RawResponse {
        status: 200,
        retry_after: None,
        body: body.into_bytes(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A BYOM model that replies `{"ok":false,"reason":"unsafe tool"}` → the
/// dispatcher maps it to a Cursor deny at exit 0.
///
/// Cursor emits deny as a JSON body at exit 0 (never exit 2).
#[test]
fn byom_block_reply_denies() {
    let _guard = set_transport_override(|_spec| {
        Ok(openai_ok_response(r#"{"ok":false,"reason":"unsafe tool"}"#))
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    // Cursor emits deny as JSON at exit 0 (not exit 2).
    assert_eq!(out.exit_code, 0, "Cursor deny must be exit 0, not exit 2");
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "expected Cursor deny JSON with permission:deny, got: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("unsafe tool"),
        "deny reason must be forwarded from the model reply: {}",
        out.stdout
    );
}

/// A BYOM model that replies `{"ok":true}` → non-blocking allow (empty stdout +
/// exit 0, the standard fail-open shape).
#[test]
fn byom_allow_reply_is_allow() {
    let _guard = set_transport_override(|_spec| Ok(openai_ok_response(r#"{"ok":true}"#)));

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "ok:true must produce an empty allow, got: {}",
        out.stdout
    );
}

/// A transport error (connection refused, DNS failure, …) → fail-open allow.
/// Tome NEVER blocks the agent because of its own provider fault.
#[test]
fn provider_transport_error_fails_open() {
    let _guard = set_transport_override(|_spec| {
        Err(TransportFailure::Other {
            detail: "connection refused".into(),
        })
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0, "transport error must fail open at exit 0");
    assert!(
        out.stdout.is_empty(),
        "transport error must produce an empty allow (no block): {}",
        out.stdout
    );
}

/// A provider timeout → fail-open allow.
#[test]
fn provider_timeout_fails_open() {
    let _guard = set_transport_override(|_spec| Err(TransportFailure::Timeout));

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0, "timeout must fail open at exit 0");
    assert!(
        out.stdout.is_empty(),
        "timeout must produce an empty allow: {}",
        out.stdout
    );
}

/// When no provider is configured (`Config::default()`), the prompt handler in
/// the manifest is a non-blocking fail-open allow — same as if no handler were
/// present. This is also the path exercised by `dispatch_core` (which passes
/// `Config::default()` internally).
#[test]
fn prompt_handler_unconfigured_provider_fails_open() {
    // No transport needed — the provider path is never reached when unconfigured.
    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = Config::default(); // no prompt_provider/model set
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(
        out.exit_code, 0,
        "unconfigured prompt handler must fail open"
    );
    assert!(
        out.stdout.is_empty(),
        "unconfigured prompt handler must produce empty allow: {}",
        out.stdout
    );
}

/// A model reply that is not valid JSON → fail-open allow (lenient parse).
#[test]
fn unparsable_model_reply_fails_open() {
    let _guard = set_transport_override(|_spec| {
        Ok(openai_ok_response(
            "I have reviewed this and it seems fine.",
        ))
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "non-JSON reply must fail open to allow: {}",
        out.stdout
    );
}

/// `dispatch_core` (the backward-compatible entry point used by all existing
/// tests) continues to fail open for prompt handlers — it internally passes
/// `Config::default()` which has no prompt provider configured.
#[test]
fn dispatch_core_prompt_handler_is_still_fail_open() {
    // Confirm the pre-US6 behavior is preserved for callers that use `dispatch_core`.
    let m = manifest_with_prompt("Is this tool use safe?");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "dispatch_core prompt handler must fail open (Config::default()): {}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// Fix 1b: parser hardening — integration tests through the full transport seam
// ---------------------------------------------------------------------------

/// Fix 1b: a model that wraps its JSON reply in a markdown code fence
/// (`\`\`\`json … \`\`\``) is now parsed correctly → Deny.
///
/// Before Fix 1b the fenced reply fell through as non-JSON and the handler
/// failed open (silently allowing a request the plugin author meant to block).
#[test]
fn byom_fenced_json_deny_is_parsed() {
    let _guard = set_transport_override(|_spec| {
        Ok(openai_ok_response(
            "```json\n{\"ok\":false,\"reason\":\"fenced deny\"}\n```",
        ))
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0, "Cursor deny must be exit 0");
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "fenced deny must map to Cursor deny: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("fenced deny"),
        "deny reason must be forwarded: {}",
        out.stdout
    );
}

/// Fix 1b: a model that embeds the JSON object mid-prose is now parsed → Deny.
/// The extractor finds the first balanced `{…}` and parses it.
#[test]
fn byom_prose_with_embedded_deny_is_parsed() {
    let _guard = set_transport_override(|_spec| {
        Ok(openai_ok_response(
            "After careful review I think this is unsafe. \
             {\"ok\":false,\"reason\":\"embedded deny\"} Please reconsider.",
        ))
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "prose-embedded deny must map to Cursor deny: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("embedded deny"),
        "deny reason must be forwarded: {}",
        out.stdout
    );
}

/// Fix 1b: a model that embeds `{"ok":true}` mid-prose → fail-open allow.
/// The extractor finds the object but `ok=true` is not a deny.
#[test]
fn byom_prose_with_embedded_allow_fails_open() {
    let _guard = set_transport_override(|_spec| {
        Ok(openai_ok_response(
            "This looks fine to me. {\"ok\":true} Go ahead.",
        ))
    });

    let m = manifest_with_prompt("Is this tool use safe?");
    let cfg = cfg_with_openai_prompt();
    let out = run_hook::dispatch_with_cfg("cursor", "PreToolUse", "{}", Some(&m), &cfg);

    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "prose ok:true must produce an empty allow: {}",
        out.stdout
    );
}
