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
