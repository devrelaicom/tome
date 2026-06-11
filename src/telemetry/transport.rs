//! Delivery transport SCAFFOLDING (Phase 10, US Foundational).
//!
//! This slice lands only the endpoint resolver + the no-foreground-network
//! counter seam. The real `reqwest::blocking` POST (read queue → POST →
//! rewrite-after-2xx) lands in US3 and is the ONLY site permitted to touch the
//! network — it MUST call [`record_network_call`] so the integration tests can
//! assert zero network calls after a foreground command.

use std::sync::atomic::{AtomicU64, Ordering};

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
}
