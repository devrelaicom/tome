//! CONSENT MATRIX — the kernel resolves consent (CI auto-off, opt-out, the global
//! kill switch) and a disabled handle is a pure no-op. These build local
//! `gauge_telemetry::Telemetry` handles directly via the builder (NOT the global
//! `OnceLock`) so each case is deterministic and isolated, then assert the
//! resolved `is_enabled()` and the no-op emit behaviour.
//!
//! Endpoint validation is also covered: a non-https/non-loopback endpoint is
//! rejected at build (a misconfigured enabled build never silently fills the
//! queue), while a loopback/https endpoint is accepted.

use gauge_telemetry::{BuildError, Telemetry};
use tempfile::TempDir;

/// A builder pre-populated with the required fields and forced ENABLED inputs
/// (`config_enabled(true)`, `runtime_enabled(true)`), CI forced OFF by default so
/// the case under test is the only variable. Each test overrides exactly the one
/// consent input it exercises.
fn enabled_builder(dir: &TempDir) -> gauge_telemetry::Builder {
    Telemetry::builder()
        .app("tome")
        .app_version("0.7.6")
        .endpoint("http://127.0.0.1:1")
        .install_id_path(dir.path().join("id"))
        .config_enabled(true)
        .runtime_enabled(true)
        .ci(false)
}

// ---------------------------------------------------------------------------
// CI auto-off.
// ---------------------------------------------------------------------------

#[test]
fn ci_env_disables_handle() {
    // The kernel auto-disables under CI. A handle built with `ci(true)` is a
    // no-op, and a disabled build never even mints the install id (no FS work).
    let dir = TempDir::new().unwrap();
    let t = enabled_builder(&dir).ci(true).build().unwrap();
    assert!(!t.is_enabled(), "CI must auto-disable telemetry");
    assert!(
        !dir.path().join("id").exists(),
        "a disabled (CI) build must not mint an install id",
    );
}

// ---------------------------------------------------------------------------
// Opt-out: config off, runtime off.
// ---------------------------------------------------------------------------

#[test]
fn config_disabled_is_a_noop() {
    let dir = TempDir::new().unwrap();
    let t = enabled_builder(&dir).config_enabled(false).build().unwrap();
    assert!(!t.is_enabled(), "config opt-out disables telemetry");
    assert!(!dir.path().join("id").exists(), "disabled ⇒ no id minted");
}

#[test]
fn runtime_disabled_is_a_noop() {
    let dir = TempDir::new().unwrap();
    let t = enabled_builder(&dir)
        .runtime_enabled(false)
        .build()
        .unwrap();
    assert!(!t.is_enabled(), "runtime opt-out disables telemetry");
    assert!(!dir.path().join("id").exists(), "disabled ⇒ no id minted");
}

// ---------------------------------------------------------------------------
// Enabled: a clean (non-CI, opted-in) build is enabled and mints an id.
// ---------------------------------------------------------------------------

#[test]
fn opted_in_non_ci_build_is_enabled_and_mints_id() {
    let dir = TempDir::new().unwrap();
    let t = enabled_builder(&dir).build().unwrap();
    assert!(t.is_enabled(), "an opted-in non-CI build is enabled");
    assert!(
        dir.path().join("id").exists(),
        "an enabled build mints the install id at build time",
    );
}

// ---------------------------------------------------------------------------
// Disabled handle is a pure no-op on emit: nothing is appended.
// ---------------------------------------------------------------------------

#[test]
fn disabled_handle_emit_appends_nothing() {
    use tome::telemetry::event::{PluginAction, PluginActionEvent};

    let dir = TempDir::new().unwrap();
    let queue = dir.path().join("id.queue.jsonl");
    let t = enabled_builder(&dir).ci(true).build().unwrap();
    assert!(!t.is_enabled());

    // Emitting on a disabled handle is a no-op — it never even creates the queue.
    t.emit(&PluginActionEvent {
        action: PluginAction::Enabled,
    });
    assert!(
        !queue.exists(),
        "a disabled handle's emit must append nothing (no queue file created)",
    );
}

// ---------------------------------------------------------------------------
// Endpoint validation: insecure endpoint rejected; loopback/https accepted.
// ---------------------------------------------------------------------------

#[test]
fn insecure_endpoint_is_rejected_at_build() {
    let dir = TempDir::new().unwrap();
    let result = Telemetry::builder()
        .app("tome")
        .app_version("0.7.6")
        .endpoint("http://telemetry.internal") // plain http, non-loopback
        .install_id_path(dir.path().join("id"))
        .config_enabled(true)
        .runtime_enabled(true)
        .ci(false)
        .build();
    match result {
        Err(BuildError::InsecureEndpoint(_)) => {}
        Err(other) => panic!("expected InsecureEndpoint, got {other:?}"),
        Ok(_) => panic!("expected InsecureEndpoint, got Ok"),
    }
    // The enabled-but-misconfigured build must not have minted an id.
    assert!(!dir.path().join("id").exists());
}

#[test]
fn loopback_and_https_endpoints_are_accepted() {
    // Loopback (used by every emit-assertion test) and https both pass
    // `endpoint_allowed`, so an enabled handle builds.
    for endpoint in ["http://127.0.0.1:1", "https://gauge-telemetry.fly.dev"] {
        let dir = TempDir::new().unwrap();
        let t = Telemetry::builder()
            .app("tome")
            .app_version("0.7.6")
            .endpoint(endpoint)
            .install_id_path(dir.path().join("id"))
            .config_enabled(true)
            .runtime_enabled(true)
            .ci(false)
            .build()
            .unwrap_or_else(|e| panic!("endpoint {endpoint} should build: {e}"));
        assert!(t.is_enabled(), "endpoint {endpoint} ⇒ enabled handle");
    }
}
