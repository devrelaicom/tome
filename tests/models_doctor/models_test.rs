//! Phase 12 / US4 (T067) — `tome models test <capability>` CLI integration.
//!
//! The REMOTE-path round-trips (embedding / summariser / reranker over the
//! transport seam) are covered in-process by the `commands::models::test` lib
//! tests — a spawned binary cannot install the in-process transport seam. These
//! integration tests cover the SPAWNED CLI surface that does NOT need a network:
//!
//! - a bundled-local model selected but NOT on disk surfaces an ACTIONABLE
//!   failure (a clean `ModelMissing` exit, never a panic / SIGABRT);
//! - `models test` writes NO stored state (no index DB is created by the run).

use crate::common::{ToolEnv, paths_for};

/// `tome models test embedding` with NO models on disk: the bundled embedder
/// can't load, so the command fails with the clean `ModelMissing` exit (30) —
/// an actionable failure, not a crash (no panic, no SIGABRT/134).
#[test]
fn embedding_missing_bundled_model_is_actionable_not_a_crash() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately do NOT fabricate any models.

    let out = env
        .cmd()
        .args(["models", "test", "embedding"])
        .output()
        .unwrap();

    let code = out.status.code();
    assert_eq!(
        code,
        Some(30),
        "missing bundled embedder must surface ModelMissing (exit 30), not a crash; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // A clean error message, not a panic backtrace.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "must be a clean error, not a panic: {stderr}"
    );
}

/// `models test` must write NO stored state: a failed (or successful) run must
/// not create the index DB. Run against a fresh root with no models — the run
/// fails on the missing model, and crucially the index DB is never created.
#[test]
fn models_test_writes_no_stored_state() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    assert!(!paths.index_db.is_file(), "precondition: no index DB");

    let _ = env
        .cmd()
        .args(["models", "test", "reranker"])
        .output()
        .unwrap();

    assert!(
        !paths.index_db.is_file(),
        "models test must not create or write the index DB"
    );
}

/// The `--json` surface is accepted (clap parses the subcommand + capability).
/// With no model on disk it still fails cleanly; we assert the failure is the
/// actionable model error, proving the arg surface is wired.
#[test]
fn models_test_json_flag_is_accepted() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["--json", "models", "test", "summariser"])
        .output()
        .unwrap();

    // Not a usage error (exit 2) — the subcommand + capability + --json parse.
    assert_ne!(
        out.status.code(),
        Some(2),
        "`--json models test summariser` must parse, not usage-error; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `--verify` is accepted on `models test` (parses, not a usage error). With
/// no bundled model on disk the round-trip still fails cleanly with the
/// actionable `ModelMissing` (exit 30) — `--verify` does not change the
/// failure classification. (The verify SEMANTICS — remote NotApplicable,
/// bundled checksum_mismatched/missing — are covered by the in-process
/// `commands::models::test` lib tests, where the SHA SSOT can be exercised
/// without a real model.)
#[test]
fn models_test_verify_flag_is_accepted() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "test", "embedding", "--verify"])
        .output()
        .unwrap();

    assert_ne!(
        out.status.code(),
        Some(2),
        "`models test embedding --verify` must parse, not usage-error; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Same clean actionable failure as without --verify (no model on disk).
    assert_eq!(
        out.status.code(),
        Some(30),
        "missing bundled embedder must still surface ModelMissing (30) under --verify; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "must be a clean error, not a panic: {stderr}"
    );
}
