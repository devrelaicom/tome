//! `tome mcp` lifecycle exit-code coverage.
//!
//! T079 calls out six failure modes; this file covers the ones we can
//! verify deterministically against the CLI binary without spawning a
//! real `rmcp` handshake (which needs a populated index + a real BGE
//! embedder load — out of scope for US1.a's scaffolding slice). Each
//! case below makes the pre-flight fail BEFORE the server binds stdio,
//! so the process exits with the contract's specific code.
//!
//! Covered here (US1.a):
//! Phase 4 / F10: `--global` is gone; the `--workspace` + `--global`
//! conflict is no longer expressible. The `WorkspaceConflict` exit
//! (code 72) is reserved-but-unused and the test is deleted below.
//! - missing index DB → exit 51 (`IndexIntegrityCheckFailure`,
//!   specific-over-generic over the residual `McpStartupFailed` 60).
//!   (The `exit-codes-p3.md` contract names the right variant but mis-
//!   types the number as 35; the closed-enum mapping in `src/error.rs`
//!   is the authority — 35 is `VectorExtensionInitFailure`.
//!   Contract reconciliation: PR-H.)
//! - schema-too-new → exit 73 (`SchemaVersionTooNew`).
//! - missing embedder file → exit 30 (`ModelMissing`).
//!
//! Deferred to T088 manual SC-001 / SC-002 verification (need a
//! populated index + working embedder load, which require either real
//! ONNX models or a stub injection point that does not exist on the
//! MCP read path yet):
//! - "startup ok" + graceful SIGINT/SIGTERM shutdown → exit 8 (T095).
//! - embedder identity mismatch (drift) → exit 41 (no integration
//!   coverage; drift CLASSIFICATION is exercised at library-API level
//!   in `tests/doctor.rs::embedder_name_drift_classifies_unhealthy`,
//!   but the MCP preflight refusal still needs T088).
//! - "index integrity check fails" → exit 35 (no integration coverage
//!   yet; the missing-file path is exercised by
//!   `mcp_preflight_index_missing_exits_35` below).

use std::io::Write;

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tome::index::{MetaSeed, OpenOptions, SCHEMA_VERSION, open};

/// Seed `meta` with the DEFAULT profile's embedder + reranker. Preflight (B4)
/// resolves the active profile's models via `active_embedder`/`active_reranker`
/// — the default is Medium (`bge-base-en-v1.5`) — so the seed must match or the
/// embedder-drift check (41) would shadow the artefact checks (30/32) these
/// tests target. The summariser keeps the registry's single summariser entry.
fn open_opts() -> OpenOptions {
    use tome::embedding::profile::{Profile, embedder_for, reranker_for};
    let e = embedder_for(Profile::DEFAULT);
    let r = reranker_for(Profile::DEFAULT);
    let s = tome::summarise::registry::summariser_entry();
    let seed = |name: &str, version: &str| MetaSeed {
        name: name.into(),
        version: version.into(),
    };
    OpenOptions {
        embedder: seed(e.name, e.version),
        reranker: seed(r.name, r.version),
        summariser: seed(s.name, s.version),
    }
}

#[test]
fn mcp_preflight_index_missing_exits_51() {
    // Fresh install: no index DB on disk. Pre-flight should fail with
    // `IndexIntegrityCheckFailure` (51) — specific-over-generic over
    // the residual `McpStartupFailed` (60) per
    // `contracts/exit-codes-p3.md` §"Specific-over-generic preference".
    // (The contract names "35" but the closed-enum mapping is 51 —
    // 35 is `VectorExtensionInitFailure`. Contract is the typo source;
    // reconciliation in PR-H.) Models are also missing in this
    // scenario, but the contract walks them in order: scope → index →
    // schema → drift → models. Index is the first to trip.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // Ensure data dir exists but NO index.db. Models dir not needed for
    // this assertion — preflight aborts before it gets to model checks.
    std::fs::create_dir_all(&paths.root).unwrap();

    // EOF on stdin keeps the subprocess from waiting on protocol bytes
    // — pre-flight runs synchronously before stdin is touched anyway,
    // but the close keeps the test deterministic across platforms.
    let mut child = env
        .cmd()
        .args(["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tome mcp");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait for tome mcp");

    assert_eq!(
        out.status.code(),
        Some(51),
        "expected exit 51 IndexIntegrityCheckFailure(index_missing), got {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn mcp_schema_too_new_returns_73() {
    // Bootstrap an index then stamp the schema version higher than the
    // compiled `SCHEMA_VERSION`. MCP pre-flight re-gates schema via
    // `SchemaVersionTooNew` (73) per `contracts/mcp-server.md` — a
    // deliberate split from the legacy `SchemaTooNew` (52) that
    // `open_read_only` still emits for the CLI read paths.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    {
        let conn = open(&paths.index_db, &open_opts()).expect("bootstrap");
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![(SCHEMA_VERSION + 1).to_string()],
        )
        .expect("stamp future version");
    }

    let mut child = env
        .cmd()
        .args(["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tome mcp");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait for tome mcp");

    assert_eq!(
        out.status.code(),
        Some(73),
        "expected exit 73 SchemaVersionTooNew, got {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn mcp_missing_embedder_file_returns_30() {
    // Index bootstrap completes; models directory is empty. Pre-flight
    // reaches step 5 (verify embedder artefacts) and returns
    // `ModelMissing` (30) per `contracts/exit-codes-p3.md`
    // §"Specific-over-generic".
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _ = open(&paths.index_db, &open_opts()).expect("bootstrap");

    let mut child = env
        .cmd()
        .args(["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tome mcp");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait for tome mcp");

    assert_eq!(
        out.status.code(),
        Some(30),
        "expected exit 30 ModelMissing, got {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn mcp_checksum_mismatch_returns_32() {
    // Bootstrap the index and fabricate sparse, all-zero model files.
    // The artefact's SHA-256 will not match the registry's pinned
    // digest, so preflight step 5 returns `ModelChecksumMismatch` (32).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _ = open(&paths.index_db, &open_opts()).expect("bootstrap");
    fabricate_all_registry_models(&paths);

    let mut child = env
        .cmd()
        .args(["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tome mcp");
    // Keep stdin alive briefly so a slow process doesn't lose the
    // pre-flight to the EOF race; pre-flight runs before we even touch
    // stdin, but having the pipe open is closer to a real harness.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"");
    }
    let out = child.wait_with_output().expect("wait for tome mcp");

    assert_eq!(
        out.status.code(),
        Some(32),
        "expected exit 32 ModelChecksumMismatch, got {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}
