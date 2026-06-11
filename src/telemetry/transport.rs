//! Delivery transport (Phase 10, US3).
//!
//! The endpoint resolver + the no-foreground-network counter seam landed in the
//! Foundational slice; this slice lands [`post_batch`], the ONE site permitted
//! to touch the network. It MUST call [`record_network_call`] so the
//! integration tests can assert zero network calls after a foreground command.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::TomeError;

/// Per-request timeout for the single POST attempt (FR-041b/043). No retry: one
/// connect+send+receive must complete within this window or the batch is left
/// unsent for the next drain.
const POST_TIMEOUT: Duration = Duration::from_secs(5);

/// Max events per batch (FR-041b). The flusher groups queue-line *indices* into
/// batches each ≤ this many lines AND ≤ [`MAX_BATCH_BYTES`] of wire bytes.
pub const MAX_BATCH_LINES: usize = 100;

/// Max wire bytes per batch (FR-041b): the NDJSON body (lines joined by `\n`
/// plus a trailing `\n`) must not exceed 256 KiB. A single line can never reach
/// this on its own (the queue's 4096 B per-line cap), so no oversize-single
/// special case is needed.
pub const MAX_BATCH_BYTES: usize = 256 * 1024;

/// The production collector endpoint, compiled in as a single `const`.
///
/// The receiving endpoint is **operational scope** (PRD §Non-goals, line 71) —
/// it is not in the spec/plan to stand up, only to point at. Pinning it as a
/// one-line const (the research §R-16 precedent) keeps "where do events go?"
/// auditable and confirm-at-deploy: change one line, not a config schema. It is
/// overridable via `TOME_TELEMETRY_ENDPOINT` and always read through
/// [`resolve_endpoint`] so it is scrubbed before it can reach any user surface.
const DEFAULT_ENDPOINT: &str = "https://telemetry.tome-mcp.app/v1/events";

/// The endpoint the flusher POSTs to: the `TOME_TELEMETRY_ENDPOINT` override if
/// set and non-empty, else [`DEFAULT_ENDPOINT`] — then ALWAYS scrubbed through
/// the shared credential scrubber.
///
/// This is THE accessor every status/log/error surface uses, so a
/// credential-bearing override (`https://user:pass@host/...`) can never be shown
/// unscrubbed. Returning a `String` (not `&str`) is deliberate: the scrubbed
/// form is freshly allocated.
pub fn resolve_endpoint() -> String {
    let raw = match std::env::var("TOME_TELEMETRY_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => DEFAULT_ENDPOINT.to_string(),
    };
    let scrubbed = crate::catalog::git::scrub_credentials(raw.as_bytes());
    String::from_utf8_lossy(&scrubbed).into_owned()
}

/// Counts every POST the flusher makes — the structural seam guarding the
/// no-foreground-network invariant (research §R-10, NFR-001).
///
/// The US3 POST (the only network site) increments this via
/// [`record_network_call`]. Integration tests assert [`network_call_count`] is
/// `0` after a foreground CLI command / in-process MCP tool call, proving the
/// foreground path never reached the network.
#[doc(hidden)]
pub static NETWORK_CALLS: AtomicU64 = AtomicU64::new(0);

/// Record that a network call was made. Called ONLY by the US3 POST site.
pub fn record_network_call() {
    NETWORK_CALLS.fetch_add(1, Ordering::Relaxed);
}

/// The number of network calls made so far this process. Test-facing.
#[doc(hidden)]
pub fn network_call_count() -> u64 {
    NETWORK_CALLS.load(Ordering::Relaxed)
}

/// Group line indices into delivery batches each bounded by BOTH
/// [`MAX_BATCH_LINES`] lines AND [`MAX_BATCH_BYTES`] of wire bytes (FR-041b).
///
/// The wire size of a batch is the NDJSON body the flusher will POST: each
/// line's bytes plus one `\n` (a `\n` after the final line too — the
/// `lines.join("\n") + "\n"` shape `post_batch` consumes). We greedily fill a
/// batch and close it the moment adding the next line would cross *either* cap.
///
/// A single line cannot exceed [`MAX_BATCH_BYTES`] on its own (the queue caps
/// each line at 4096 B including its newline — FR-036), so a one-line batch is
/// always deliverable; no oversize-single special case is needed.
pub fn split_batches(lines: &[String]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_bytes = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        // Each line contributes its bytes + one trailing '\n' to the body.
        let line_wire = line.len() + 1;
        let would_overflow_bytes =
            !current.is_empty() && current_bytes + line_wire > MAX_BATCH_BYTES;
        let would_overflow_lines = current.len() >= MAX_BATCH_LINES;

        if would_overflow_bytes || would_overflow_lines {
            batches.push(std::mem::take(&mut current));
            current_bytes = 0;
        }

        current.push(idx);
        current_bytes += line_wire;
    }

    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// POST one NDJSON batch for `stream` to the resolved collector endpoint — THE
/// single network site (FR-043).
///
/// - Resolves the endpoint via [`resolve_endpoint`] (already credential-scrubbed)
///   and uses that scrubbed form as BOTH the POST target and any error display,
///   so a credential can never reach the wire or a user surface. Telemetry
///   endpoints carry no URL credentials, so the scrubbed form IS the target.
/// - **HTTPS only** (FR-043): a non-`https://` endpoint fails CLOSED with
///   [`TelemetryEndpointUnreachable`](TomeError::TelemetryEndpointUnreachable) —
///   we NEVER POST telemetry (carrying the install UUID) over plaintext.
/// - One attempt, [`POST_TIMEOUT`] (5 s), **no retry**, `Content-Type:
///   application/x-ndjson`. A `?stream=<stream>` query is appended (`&` if the
///   endpoint already has a query).
/// - Increments [`record_network_call`] right before the request — this is the
///   load-bearing increment behind the foreground-counter==0 proof.
///
/// Returns the response status as a `u16` on a COMPLETED request (the caller
/// decides 2xx vs not); a transport error (connect/timeout/TLS) or a non-https /
/// malformed endpoint is `Err(TelemetryEndpointUnreachable { endpoint })` with
/// the scrubbed endpoint. Never panics.
pub fn post_batch(stream: &str, ndjson_body: &[u8]) -> Result<u16, TomeError> {
    let endpoint = resolve_endpoint();

    // HTTPS-only fail-closed: never POST telemetry over plaintext (FR-043).
    if !endpoint.starts_with("https://") {
        return Err(TomeError::TelemetryEndpointUnreachable { endpoint });
    }

    // Append `?stream=<stream>` (or `&stream=<stream>` if a query already
    // exists). The scrubbed endpoint is the SSOT, so this stays scrubbed.
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    let url = format!("{endpoint}{separator}stream={stream}");

    let client = match reqwest::blocking::Client::builder()
        .timeout(POST_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        // A builder failure (TLS backend init) is unreachable in practice but
        // must fail closed, not panic — report the scrubbed endpoint.
        Err(_) => return Err(TomeError::TelemetryEndpointUnreachable { endpoint }),
    };

    // THE single network site. Increment BEFORE the request so the seam counts
    // the attempt even if it errors out (the proof is "did the foreground path
    // reach the network at all", not "did it succeed").
    record_network_call();

    match client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-ndjson")
        .body(ndjson_body.to_vec())
        .send()
    {
        Ok(resp) => Ok(resp.status().as_u16()),
        // connect / timeout / TLS — surface the SCRUBBED endpoint, never the
        // reqwest error display (which can reproduce the URL).
        Err(_) => Err(TomeError::TelemetryEndpointUnreachable { endpoint }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `TOME_TELEMETRY_ENDPOINT` is process-global; serialise the tests that
    /// mutate it.
    static ENDPOINT_ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EndpointEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prior: Option<std::ffi::OsString>,
    }

    impl EndpointEnvGuard {
        fn new() -> Self {
            let lock = ENDPOINT_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let prior = std::env::var_os("TOME_TELEMETRY_ENDPOINT");
            // SAFETY: guarded by ENDPOINT_ENV_MUTEX for the guard's lifetime.
            unsafe { std::env::remove_var("TOME_TELEMETRY_ENDPOINT") };
            EndpointEnvGuard { _lock: lock, prior }
        }
        fn set(&self, v: &str) {
            // SAFETY: guarded by ENDPOINT_ENV_MUTEX (held via `_lock`).
            unsafe { std::env::set_var("TOME_TELEMETRY_ENDPOINT", v) };
        }
    }

    impl Drop for EndpointEnvGuard {
        fn drop(&mut self) {
            // SAFETY: still holding the mutex.
            match &self.prior {
                Some(v) => unsafe { std::env::set_var("TOME_TELEMETRY_ENDPOINT", v) },
                None => unsafe { std::env::remove_var("TOME_TELEMETRY_ENDPOINT") },
            }
        }
    }

    #[test]
    fn default_when_unset() {
        let _g = EndpointEnvGuard::new();
        assert_eq!(resolve_endpoint(), DEFAULT_ENDPOINT);
    }

    #[test]
    fn override_is_honoured() {
        let g = EndpointEnvGuard::new();
        g.set("https://collector.example/v1/events");
        assert_eq!(resolve_endpoint(), "https://collector.example/v1/events");
    }

    #[test]
    fn empty_override_falls_back_to_default() {
        let g = EndpointEnvGuard::new();
        g.set("");
        assert_eq!(resolve_endpoint(), DEFAULT_ENDPOINT);
    }

    #[test]
    fn credential_bearing_override_is_scrubbed() {
        let g = EndpointEnvGuard::new();
        g.set("https://user:secret@collector.example/v1/events");
        let out = resolve_endpoint();
        assert!(
            !out.contains("secret"),
            "credentials must be scrubbed: {out}"
        );
        assert!(!out.contains("user:"), "userinfo must be scrubbed: {out}");
        assert!(out.contains("collector.example"));
    }

    #[test]
    fn network_counter_records() {
        let before = network_call_count();
        record_network_call();
        assert_eq!(network_call_count(), before + 1);
    }

    #[test]
    fn post_batch_rejects_plaintext_http_fail_closed() {
        // FR-043: a non-https endpoint must fail CLOSED (exit 90) and NEVER POST.
        let g = EndpointEnvGuard::new();
        g.set("http://127.0.0.1:1/v1/events");
        let before = network_call_count();
        let err = post_batch("anonymous", b"{}\n").unwrap_err();
        match err {
            TomeError::TelemetryEndpointUnreachable { endpoint } => {
                assert!(
                    endpoint.starts_with("http://"),
                    "scrubbed endpoint: {endpoint}"
                );
            }
            other => panic!("expected TelemetryEndpointUnreachable, got {other:?}"),
        }
        // It fails closed BEFORE the network — the counter must not move.
        assert_eq!(
            network_call_count(),
            before,
            "a plaintext-rejected POST must not reach the network"
        );
    }

    #[test]
    fn post_batch_error_endpoint_is_scrubbed_and_records_attempt() {
        // Point at a non-routable https addr (TEST-NET-1, RFC 5737) carrying
        // credentials in the override; the transport error endpoint must be
        // scrubbed, and `record_network_call` must have fired for the attempt.
        let g = EndpointEnvGuard::new();
        g.set("https://user:secret@192.0.2.1:1/v1/events");
        let before = network_call_count();
        let err = post_batch("anonymous", b"{}\n").unwrap_err();
        match err {
            TomeError::TelemetryEndpointUnreachable { endpoint } => {
                assert!(!endpoint.contains("secret"), "creds scrubbed: {endpoint}");
                assert!(!endpoint.contains("user:"), "userinfo scrubbed: {endpoint}");
            }
            other => panic!("expected TelemetryEndpointUnreachable, got {other:?}"),
        }
        // A real https attempt (it times out / refuses) DID reach the network
        // site, so the counter moved.
        assert_eq!(
            network_call_count(),
            before + 1,
            "an https attempt records exactly one network call"
        );
    }

    #[test]
    fn split_batches_groups_by_line_count() {
        // 250 small lines ⇒ 3 batches (100, 100, 50) at the line-count cap.
        let lines: Vec<String> = (0..250).map(|i| format!("{{\"n\":{i}}}")).collect();
        let batches = split_batches(&lines);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), MAX_BATCH_LINES);
        assert_eq!(batches[1].len(), MAX_BATCH_LINES);
        assert_eq!(batches[2].len(), 50);
        // Indices are contiguous and complete.
        assert_eq!(batches[0][0], 0);
        assert_eq!(*batches[2].last().unwrap(), 249);
    }

    #[test]
    fn split_batches_splits_on_byte_budget() {
        // Lines ~2 KiB each: 256 KiB / ~2 KiB ≈ 128 lines per batch, well under
        // the 100-line cap on the byte side — so the BYTE cap must bind first.
        // Use ~3 KiB lines so the byte cap (256 KiB ⇒ ~85 lines) trips before
        // the 100-line cap.
        let big = "z".repeat(3000);
        let lines: Vec<String> = (0..90).map(|_| big.clone()).collect();
        let batches = split_batches(&lines);
        assert!(
            batches.len() >= 2,
            "oversize-by-bytes must split: {}",
            batches.len()
        );
        // Every batch's wire size is within the byte cap.
        for batch in &batches {
            let wire: usize = batch.iter().map(|&i| lines[i].len() + 1).sum();
            assert!(wire <= MAX_BATCH_BYTES, "batch wire {wire} within cap");
        }
    }

    #[test]
    fn split_batches_empty_is_empty() {
        assert!(split_batches(&[]).is_empty());
    }
}
