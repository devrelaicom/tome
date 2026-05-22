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
//! - `--workspace` + `--global` → exit 72 (`WorkspaceConflict`).
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

mod common;

use std::io::Write;

use common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::embedding::registry::{MODEL_REGISTRY, ModelKind};
use tome::index::{MetaSeed, OpenOptions, SCHEMA_VERSION, open};

fn open_opts() -> OpenOptions {
    OpenOptions {
        embedder: MetaSeed {
            name: MODEL_REGISTRY
                .iter()
                .find(|m| m.kind == ModelKind::Embedder)
                .unwrap()
                .name
                .into(),
            version: MODEL_REGISTRY
                .iter()
                .find(|m| m.kind == ModelKind::Embedder)
                .unwrap()
                .version
                .into(),
        },
        reranker: MetaSeed {
            name: MODEL_REGISTRY
                .iter()
                .find(|m| m.kind == ModelKind::Reranker)
                .unwrap()
                .name
                .into(),
            version: MODEL_REGISTRY
                .iter()
                .find(|m| m.kind == ModelKind::Reranker)
                .unwrap()
                .version
                .into(),
        },
    }
}

#[test]
fn mcp_workspace_and_global_returns_72() {
    let env = ToolEnv::new();
    let scratch = TempDir::new().unwrap();
    let out = env
        .cmd()
        .args([
            "mcp",
            "--global",
            "--workspace",
            scratch.path().to_str().unwrap(),
        ])
        .output()
        .expect("spawn tome mcp");
    assert_eq!(
        out.status.code(),
        Some(72),
        "expected exit 72 WorkspaceConflict, got {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
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
