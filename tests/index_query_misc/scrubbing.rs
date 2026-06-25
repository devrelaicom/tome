//! Table-driven coverage of the credential scrubber. The rules are documented
//! in research.md R-8; each row below maps to one rule.

use tome::catalog::git::{scrub_credentials, scrub_to_string};

fn check(input: &str, expected: &str) {
    let out = scrub_to_string(input.as_bytes());
    assert_eq!(out, expected, "scrubbing {:?}", input);
}

#[test]
fn https_url_with_userinfo_is_stripped() {
    check(
        "fatal: clone failed for https://alice:supersecret@github.com/o/r",
        "fatal: clone failed for https://github.com/o/r",
    );
}

#[test]
fn http_url_with_userinfo_is_stripped() {
    check(
        "cloning http://user:tok@example.com/x",
        "cloning http://example.com/x",
    );
}

#[test]
fn file_url_with_userinfo_is_stripped() {
    // `file://user:secret@/path` is unusual but git accepts it (silently
    // ignoring the userinfo). The scrub must still strip it before the
    // URL is persisted to `config.toml` or echoed on stdout.
    check("file://alice:supersecret@/tmp/repo", "file:///tmp/repo");
    check("ssh://bob:hunter2@host/path", "ssh://host/path");
}

#[test]
fn ssh_url_host_preserved_login_removed() {
    let out = scrub_to_string(b"failed: git@github.com:owner/repo.git is not reachable");
    assert!(out.contains("git@<host>:owner/repo"), "got: {}", out);
    assert!(!out.contains("git@github.com"), "got: {}", out);
}

#[test]
fn token_kv_redacted() {
    check(
        "Authorization: Bearer abc123xyz",
        "Authorization: <scrubbed>",
    );
    check(
        "password=hunter2 and other text",
        "password=<scrubbed> and other text",
    );
    check("api_key: deadbeef", "api_key: <scrubbed>");
    check("api-key=abcd", "api-key=<scrubbed>");
}

#[test]
fn long_hex_sequences_redacted_outside_kv_context() {
    let token = "a".repeat(48);
    let input = format!("see token {} for details", token);
    let out = scrub_to_string(input.as_bytes());
    assert!(!out.contains(&token), "long hex token leaked: {}", out);
    assert!(out.contains("<scrubbed>"), "no scrub marker: {}", out);
}

#[test]
fn sha1_in_colon_or_equals_context_survives() {
    let sha = "deadbeefcafebabedeadbeefcafebabedeadbeef"; // 40 hex chars
    let with_colon = format!("commit: {}", sha);
    let with_equals = format!("ref={}", sha);
    let out_colon = scrub_to_string(with_colon.as_bytes());
    let out_equals = scrub_to_string(with_equals.as_bytes());
    // Both inputs use a leading separator (`: ` and `=`) that places the hex
    // *after* a `:` or `=`. Per R-8 rule 4, these contexts are preserved.
    assert!(
        out_colon.contains(sha),
        "SHA in colon context was scrubbed: {}",
        out_colon
    );
    assert!(
        out_equals.contains(sha),
        "SHA in equals context was scrubbed: {}",
        out_equals
    );
}

#[test]
fn scrub_returns_bytes_unchanged_when_clean() {
    let clean = b"nothing to see here\n";
    let out = scrub_credentials(clean);
    assert_eq!(out, clean);
}

#[test]
fn ordering_url_then_token_both_applied() {
    let input = "remote: https://alice:tok@gh.example/path Authorization: Bearer s3cret";
    let out = scrub_to_string(input.as_bytes());
    assert!(!out.contains("alice"), "userinfo leaked: {}", out);
    assert!(!out.contains("s3cret"), "bearer leaked: {}", out);
    assert!(out.contains("https://gh.example/path"));
}

// ---------------------------------------------------------------------------
// Phase 2: model-download surfaces (T060/T061).
// ---------------------------------------------------------------------------

#[test]
fn aws_signed_query_string_is_redacted() {
    // Typical S3 presigned URL — sha-flavoured signature + access key id +
    // session token in the query string. All three must be scrubbed.
    let signature = "a".repeat(64); // SHA-256 hex
    let credential = "AKIAIOSFODNN7EXAMPLE/20260512/us-east-1/s3/aws4_request";
    let session_token = "FwoGZXIvYXdzEXAMPLETOKEN";
    let url = format!(
        "fetching https://bucket.s3.amazonaws.com/model.onnx?\
         X-Amz-Algorithm=AWS4-HMAC-SHA256&\
         X-Amz-Credential={credential}&\
         X-Amz-Date=20260512T000000Z&\
         X-Amz-Expires=900&\
         X-Amz-Signature={signature}&\
         X-Amz-Security-Token={session_token}"
    );
    let out = scrub_to_string(url.as_bytes());

    assert!(!out.contains(&signature), "signature leaked: {out}");
    assert!(!out.contains(credential), "credential leaked: {out}");
    assert!(!out.contains(session_token), "session token leaked: {out}");
    // Innocuous bits stay so the operator can still see what was being fetched.
    assert!(
        out.contains("https://bucket.s3.amazonaws.com/model.onnx"),
        "host/path was over-scrubbed: {out}"
    );
}

#[test]
fn generic_signature_query_param_is_redacted() {
    // Hugging Face presigned URLs use a plain `signature=` param.
    let sig = "deadbeef".repeat(8);
    let input =
        format!("GET https://cdn-lfs.huggingface.co/repos/foo?signature={sig}&expires=12345");
    let out = scrub_to_string(input.as_bytes());
    assert!(!out.contains(&sig), "HF signature leaked: {out}");
    // Expiry timestamps are not sensitive and stay visible.
    assert!(
        out.contains("expires=12345"),
        "expires field over-scrubbed: {out}"
    );
}

#[test]
fn reqwest_style_error_with_url_credentials_is_redacted() {
    // Approximates what `reqwest::Error::Display` produces for a failed
    // request against a userinfo-bearing URL.
    let input = "HTTP get failed: error sending request for url \
                 (https://user:supersecret@cdn.example/bucket/model.onnx): \
                 dns error: failed to lookup address";
    let out = scrub_to_string(input.as_bytes());
    assert!(!out.contains("supersecret"), "userinfo leaked: {out}");
    assert!(!out.contains("user:"), "userinfo prefix leaked: {out}");
    assert!(
        out.contains("https://cdn.example/bucket/model.onnx"),
        "host/path was over-scrubbed: {out}"
    );
    assert!(
        out.contains("dns error"),
        "diagnostic suffix was over-scrubbed: {out}"
    );
}

// ---------------------------------------------------------------------------
// Phase 4 / Polish PR-D / T-M10 — scrubbing extensions for Phase 4
// surfaces. Every download URL Tome handles should round-trip through
// `scrub_to_string` idempotently; every harness-MCP-config error chain
// path should preserve verbatim (no scrub-eligible content per
// `contracts/paths-and-layout-p4.md`).
// ---------------------------------------------------------------------------

#[test]
fn scrub_summariser_download_url_is_idempotent_and_preserves_host_path() {
    // The HuggingFace summariser URL carries no credentials; assert the
    // scrubber leaves it byte-for-byte stable and keeps host + path
    // intact so an operator reading a log line can still tell what was
    // being downloaded.
    use tome::summarise::registry::SUMMARISER_SOURCE_URL;

    let once = scrub_to_string(SUMMARISER_SOURCE_URL.as_bytes());
    assert_eq!(
        once, SUMMARISER_SOURCE_URL,
        "first-pass scrub mutated a clean HF URL",
    );

    // Idempotence: scrubbing the scrubbed value MUST return the same
    // value. Documents that the discipline survives repeated passes
    // through the boundary.
    let twice = scrub_to_string(once.as_bytes());
    assert_eq!(
        twice, once,
        "scrub_to_string is not idempotent on clean URL"
    );

    // Preserves host + path explicitly.
    assert!(
        once.contains("huggingface.co"),
        "host stripped from clean URL: {once}",
    );
    assert!(
        once.contains("qwen2.5-0.5b-instruct-q4_k_m.gguf"),
        "path stripped from clean URL: {once}",
    );
}

#[test]
fn scrub_to_string_handles_harness_mcp_config_error_chain_paths() {
    // Harness MCP config paths (e.g. `~/.codex/config.toml`,
    // `~/.cursor/mcp.json`) carry no scrub-eligible content per the
    // `paths-and-layout-p4.md` "What is NOT a credential" table. Assert
    // the scrubber preserves them verbatim so an operator debugging a
    // sync failure can see exactly which file refused to parse.
    let input = "failed to parse harness MCP config at \
                 /home/user/.codex/config.toml: \
                 invalid TOML at line 3: expected `}` after table";
    let out = scrub_to_string(input.as_bytes());
    assert_eq!(
        out, input,
        "harness MCP config path / error chain mutated by scrub: {out}",
    );

    // Same discipline for the other four harnesses' typical paths.
    for path in [
        "/home/user/.claude/settings.json",
        "/home/user/.cursor/mcp.json",
        "/home/user/.gemini/config.json",
        "/home/user/.opencode/config.toml",
    ] {
        let err = format!("failed at {path}: io error: permission denied");
        let scrubbed = scrub_to_string(err.as_bytes());
        assert_eq!(
            scrubbed, err,
            "harness path was mutated by scrubber: {path}",
        );
    }
}

#[test]
fn signed_url_keys_in_colon_form_also_redact() {
    // Some loggers pretty-print query strings as colon-separated KV pairs
    // (e.g. tracing field rendering). Make sure that form is also caught.
    let input = "X-Amz-Signature: deadbeef0123456789, X-Amz-Credential: AKIASOMETHING";
    let out = scrub_to_string(input.as_bytes());
    assert!(
        !out.contains("deadbeef"),
        "colon-form signature leaked: {out}"
    );
    assert!(
        !out.contains("AKIASOMETHING"),
        "colon-form credential leaked: {out}"
    );
}

// ---------------------------------------------------------------------------
// Phase 12 — BYOK/BYOM provider key formats (FR-014a / SC-006).
//
// The SSOT scrubber is extended to redact each supported provider key format
// wherever it could surface — bare in a response body AND in the per-kind auth
// contexts (Bearer header, `x-api-key` header, `?key=` query). Real provider
// keys exceed the format length bounds (`sk-`/`pa-` ≥16, `AIza` ≥20), so the
// bare-token fallback always catches a reflected key; the KV/header contexts
// catch the value regardless of length.
// ---------------------------------------------------------------------------

#[test]
fn bare_provider_keys_in_a_response_body_are_redacted() {
    // A provider that reflects the request (including the key) in its error
    // body is the pre-mortem's load-bearing leak case.
    for key in [
        "sk-ABCDEFGHIJKLMNOPQRSTUVWX",            // OpenAI legacy
        "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV",    // Anthropic
        "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX",       // OpenAI project
        "pa-ABCDEFGHIJKLMNOPQRSTUVWXYZ01",        // Voyage
        "AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345", // Google
    ] {
        let body = format!("{{\"error\":\"invalid api key {key} supplied\"}}");
        let out = scrub_to_string(body.as_bytes());
        assert!(
            !out.contains(key),
            "bare provider key leaked in body: key={key} out={out}",
        );
    }
}

#[test]
fn provider_keys_in_bearer_context_are_redacted() {
    let input = "request failed: Authorization: Bearer sk-ABCDEFGHIJKLMNOPQRSTUVWX";
    let out = scrub_to_string(input.as_bytes());
    assert!(
        !out.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"),
        "bearer-context key leaked: {out}",
    );
}

#[test]
fn provider_keys_in_x_api_key_header_context_are_redacted() {
    // Anthropic uses `x-api-key`; the value must be scrubbed, header name kept.
    let input = "headers: x-api-key: sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV";
    let out = scrub_to_string(input.as_bytes());
    assert!(
        !out.contains("sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV"),
        "x-api-key value leaked: {out}",
    );
    assert!(
        out.contains("x-api-key"),
        "x-api-key header name should be preserved: {out}",
    );
}

#[test]
fn google_key_in_query_string_is_redacted() {
    // Gemini places the credential as `?key=<k>`.
    let input = "GET https://generativelanguage.googleapis.com/v1beta/models/x:generateContent?key=AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
    let out = scrub_to_string(input.as_bytes());
    assert!(
        !out.contains("AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345"),
        "gemini query key leaked: {out}",
    );
}

#[test]
fn provider_scrub_preserves_existing_behaviour_on_clean_text() {
    // No false positives on ordinary prose / identifiers that merely contain a
    // short `sk`/`pa` fragment without the key shape.
    let clean = "the skill `pa11y-audit` ran ok; see sk for details";
    let out = scrub_to_string(clean.as_bytes());
    assert_eq!(
        out, clean,
        "clean text mutated by provider-key scrub: {out}"
    );
}

// ---------------------------------------------------------------------------
// T080 — SC-006: no supported provider key format survives ERROR FORMATTING at
// any log level, asserted through the ACTUAL production error path (not the raw
// scrubber): `ProviderError::new` (scrubs `raw_detail` at construction) →
// `into_tome_error()` (carries the scrubbed `Display` into the `TomeError`
// `detail`) → the three surfaces an operator/log can observe:
//   (a) `ProviderError::Display`,
//   (b) the mapped `TomeError::Display`,
//   (c) the `--json` error envelope (`ErrorRecord` → `category`/`exit_code`/
//       `message`, where `message = format!("{}", err)`).
//
// Covered formats: OpenAI `sk-…`, Anthropic `sk-ant-…`, OpenAI project
// `sk-proj-…`, Voyage `pa-…`, Google `AIza…`. Each is asserted in BOTH a
// header-echo context (`Authorization: Bearer <k>` / `x-api-key: <k>`) and a
// bare JSON-body-reflection context — the two real shapes a provider that
// echoes the request can leak a key in.
// ---------------------------------------------------------------------------

#[test]
fn sc006_no_provider_key_survives_error_formatting() {
    use tome::error::TomeError;
    use tome::output::ErrorRecord;
    use tome::provider::error::{ProviderError, ProviderErrorKind};

    // Real provider keys exceed the format length bounds the SSOT scrubber
    // enforces (`sk-`/`pa-` ≥16 url-safe, `AIza` ≥20), so the bare-token
    // fallback always catches a reflected key even with no `key=` context.
    let keys = [
        "sk-ABCDEFGHIJKLMNOPQRSTUVWX",            // OpenAI legacy
        "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV",    // Anthropic
        "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX",       // OpenAI project
        "pa-ABCDEFGHIJKLMNOPQRSTUVWXYZ01",        // Voyage
        "AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ012345", // Google
    ];

    for key in keys {
        // The two real leak shapes for each key:
        //  (a) a header echo (Bearer / x-api-key — the openai/voyage and
        //      anthropic auth placements), and
        //  (b) a bare JSON-body reflection (the pre-mortem's load-bearing
        //      case: a provider that mirrors the request — including the key —
        //      back in its error body).
        let header_echo =
            format!("HTTP 401: upstream rejected request headers: Authorization: Bearer {key}");
        let x_api_key_echo = format!("HTTP 401: rejected headers: x-api-key: {key}");
        let body_reflection = format!("HTTP 400: {{\"error\":\"invalid api key {key} supplied\"}}");

        for raw_detail in [&header_echo, &x_api_key_echo, &body_reflection] {
            // Route through the REAL error path — the constructor scrubs.
            let provider_err =
                ProviderError::new("acme", ProviderErrorKind::Auth, false, raw_detail);

            // (a) ProviderError::Display.
            let pe_display = provider_err.to_string();
            assert!(
                !pe_display.contains(key),
                "key `{key}` leaked in ProviderError::Display\n  raw={raw_detail}\n  out={pe_display}",
            );

            // Map onto the closed TomeError set (the single mapping point).
            let tome_err: TomeError = provider_err.into_tome_error();

            // (b) TomeError::Display (what `write_error` prints in Human mode).
            let te_display = tome_err.to_string();
            assert!(
                !te_display.contains(key),
                "key `{key}` leaked in TomeError::Display\n  raw={raw_detail}\n  out={te_display}",
            );

            // (c) The `--json` error envelope: the serialised `ErrorRecord`
            //     (category + exit_code + message). `message` is the TomeError
            //     Display, which is already scrubbed — assert on the actual
            //     serialised JSON bytes so a future schema change can't silently
            //     reintroduce the key.
            let record = ErrorRecord::from_error(&tome_err);
            let json = serde_json::to_string(&record).expect("ErrorRecord serialises");
            assert!(
                !json.contains(key),
                "key `{key}` leaked in the --json error record\n  raw={raw_detail}\n  json={json}",
            );

            // Sanity: the surfaces are still useful — the redaction marker is
            // present and the structured fields survived (we scrubbed the key,
            // not the whole message).
            assert!(
                pe_display.contains("<scrubbed>"),
                "expected the scrub marker in: {pe_display}",
            );
            assert!(
                json.contains("\"category\":\"provider_request_failed\"")
                    && json.contains("\"exit_code\":94"),
                "expected the structured --json fields to survive: {json}",
            );
        }
    }
}

#[test]
fn sc006_no_provider_key_survives_in_tracing_error_field() {
    // Defence-in-depth: a `tracing::warn!(error = %e)` / `debug!` renders the
    // error's Display, which we already proved scrubbed above. Capture a tracing
    // event at WARN and DEBUG and assert the key is absent at BOTH levels — the
    // "at any log level" clause of SC-006. We capture via a custom Layer that
    // records the formatted `error` field (no global subscriber install — this
    // is a scoped `with_default` so it never collides with parallel tests).
    use std::sync::{Arc, Mutex};

    use tome::provider::error::{ProviderError, ProviderErrorKind};
    use tracing::Level;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};

    /// A minimal Layer that appends every event's `Debug`-formatted field set to
    /// a shared buffer, so the test can assert on what a real subscriber would
    /// have written — at whatever level the event was emitted.
    struct CaptureLayer(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            struct Visitor<'a>(&'a mut String);
            impl tracing::field::Visit for Visitor<'_> {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    use std::fmt::Write;
                    let _ = write!(self.0, "{}={value:?} ", field.name());
                }
            }
            let mut line = String::new();
            event.record(&mut Visitor(&mut line));
            self.0.lock().expect("capture buffer").push(line);
        }
    }

    let key = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUV";
    let raw_detail = format!("HTTP 401: headers echoed: x-api-key: {key}");
    let err = ProviderError::new("acme", ProviderErrorKind::Auth, false, &raw_detail);

    let buf = Arc::new(Mutex::new(Vec::<String>::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(Arc::clone(&buf)));

    tracing::subscriber::with_default(subscriber, || {
        tracing::event!(Level::WARN, error = %err, "remote provider call failed");
        tracing::event!(Level::DEBUG, error = %err, "remote provider call failed");
    });

    let captured = buf.lock().expect("capture buffer").join("\n");
    assert!(
        captured.contains("error="),
        "precondition: the error field was captured: {captured}",
    );
    assert!(
        !captured.contains(key),
        "key `{key}` leaked into a tracing error field: {captured}",
    );
}
