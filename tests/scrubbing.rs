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
