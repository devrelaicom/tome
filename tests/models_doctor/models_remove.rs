//! Integration tests for `tome models remove <name>` against the CLI binary.
//!
//! Covers FR-024 plus the four documented error paths from
//! `contracts/models-commands.md` §"`tome models remove`":
//!   * Unknown model → exit 2 (`Usage`).
//!   * Not installed → exit 30 (`ModelMissing`).
//!   * Non-TTY without `--force` → exit 54 (`NotATerminal`) with the
//!     documented pointer message on stderr.
//!   * Happy path with `--force` → manifest and model directory both
//!     removed.

use crate::common::{
    ToolEnv, fabricate_all_registry_models, fabricate_installed_models, paths_for,
};
use tome::embedding::registry::MODEL_REGISTRY;

fn embedder() -> &'static tome::embedding::registry::ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::registry::ModelKind::Embedder))
        .expect("registry has an embedder")
}

fn reranker() -> &'static tome::embedding::registry::ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::registry::ModelKind::Reranker))
        .expect("registry has a reranker")
}

#[test]
fn remove_with_unknown_model_name_exits_2_with_usage_error() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "remove", "no-such-model", "--force"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 (Usage) for unknown model, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown model"),
        "stderr must mention the unknown-model usage error, got: {stderr}",
    );
}

#[test]
fn remove_of_uninstalled_registered_model_exits_30() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let entry = embedder();
    let out = env
        .cmd()
        .args(["models", "remove", entry.name, "--force"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(30),
        "expected exit 30 (ModelMissing), got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn remove_without_force_in_non_tty_exits_54_with_pointer_message() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let entry = embedder();
    fabricate_installed_models(&paths, &[entry]);

    // Subprocess stdin/stdout/stderr are pipes — not TTYs. Without `--force`
    // the handler must short-circuit before the confirm prompt, emit the
    // documented pointer line to stderr, and exit 54.
    let out = env
        .cmd()
        .args(["models", "remove", entry.name])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(54),
        "expected exit 54 (NotATerminal), got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--yes"),
        "stderr must point the user at --yes (#438), got: {stderr}",
    );

    // State must NOT have changed.
    let model_dir = paths.models_dir.join(entry.name);
    assert!(
        model_dir.is_dir(),
        "refused remove must not delete the model dir"
    );
    assert!(
        model_dir.join("manifest.toml").is_file(),
        "refused remove must not delete the manifest",
    );
}

#[test]
fn remove_with_force_deletes_manifest_and_model_directory() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let entry = embedder();
    fabricate_installed_models(&paths, &[entry]);

    let model_dir = paths.models_dir.join(entry.name);
    assert!(model_dir.is_dir(), "pre-condition: fixture must exist");

    let out = env
        .cmd()
        .args(["models", "remove", entry.name, "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    assert!(
        !model_dir.exists(),
        "model directory must be gone after --force remove",
    );
}

// ── issue #315: variadic + --all ─────────────────────────────────────────────

/// (a) Multiple positional names are each removed (durable-effect).
#[test]
fn remove_multiple_names_deletes_each() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let (emb, rnk) = (embedder(), reranker());
    fabricate_installed_models(&paths, &[emb, rnk]);

    let out = env
        .cmd()
        .args(["models", "remove", emb.name, rnk.name, "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(!paths.models_dir.join(emb.name).exists(), "embedder gone");
    assert!(!paths.models_dir.join(rnk.name).exists(), "reranker gone");
}

/// (b) `--all` evicts every INSTALLED model.
#[test]
fn remove_all_evicts_every_installed_model() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    // Pre-condition: at least the embedder + reranker are on disk.
    assert!(paths.models_dir.join(embedder().name).is_dir());

    let out = env
        .cmd()
        .args(["models", "remove", "--all", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    for entry in MODEL_REGISTRY.iter() {
        assert!(
            !paths.models_dir.join(entry.name).exists(),
            "{} must be evicted by --all",
            entry.name,
        );
    }
}

/// `--all` on an empty install set is a whole no-op (success, nothing removed).
#[test]
fn remove_all_with_nothing_installed_is_noop_success() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["--json", "models", "remove", "--all", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json envelope");
    assert!(
        v["models"].as_array().expect("models array").is_empty(),
        "no installed models → empty removal list",
    );
}

/// (c) `--all` conflicts with a positional name (clap parse error, exit 2).
#[test]
fn remove_all_with_positional_is_a_clap_conflict() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "remove", "--all", embedder().name, "--force"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "`--all <name>` must be a clap conflict (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Neither names nor `--all` → a usage error (exit 2).
#[test]
fn remove_with_no_selection_is_usage_2() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "remove", "--force"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "no selection → exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// A mid-list unknown name fails at resolve time (before any delete) — exit 2,
/// and the earlier valid target is NOT removed (fail-loud on a bad name,
/// matching the single-name path; this is a resolution error, not a
/// per-item runtime failure).
#[test]
fn remove_mixed_unknown_name_fails_before_any_delete() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let emb = embedder();
    fabricate_installed_models(&paths, &[emb]);

    let out = env
        .cmd()
        .args(["models", "remove", emb.name, "no-such-model", "--force"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "unknown name → exit 2");
    assert!(
        paths.models_dir.join(emb.name).is_dir(),
        "no deletion happened — resolution failed before the loop",
    );
}

/// `--all` still respects the non-TTY confirmation refusal (destructive opt-in
/// is NOT bypassed by `--all`): without `--force` on a non-TTY, exit 54 and
/// nothing is removed.
#[test]
fn remove_all_without_force_on_non_tty_is_54_and_removes_nothing() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let emb = embedder();
    fabricate_installed_models(&paths, &[emb]);

    let out = env
        .cmd()
        .args(["models", "remove", "--all"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(54),
        "--all must still require confirmation on a non-TTY; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        paths.models_dir.join(emb.name).is_dir(),
        "refused --all remove must not delete anything",
    );
}

/// Byte-stable wire-shape pin (binary-driven): a SINGLE `models remove <name>
/// --json` must serialise as the BARE record `{"name":..,"status":"removed"}`
/// — byte-identical to the pre-#315 shape (no `{"models":[..]}` envelope).
#[test]
fn remove_single_json_wire_shape_pin_is_bare_record() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let entry = embedder();
    fabricate_installed_models(&paths, &[entry]);

    let out = env
        .cmd()
        .args(["--json", "models", "remove", entry.name, "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = format!(r#"{{"name":"{}","status":"removed"}}"#, entry.name);
    assert_eq!(
        stdout.trim_end(),
        expected,
        "single `models remove --json` must be the BARE pre-#315 record shape",
    );
}

/// Load-bearing dedupe: `models remove <name> <name> --force` (same name twice)
/// deletes the model exactly ONCE and exits 0. Without the `resolve_targets`
/// dedupe the second `remove_one` would `remove_file` an already-gone manifest
/// → I/O error / non-zero exit.
#[test]
fn remove_duplicate_names_deletes_once_and_succeeds() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let entry = embedder();
    fabricate_installed_models(&paths, &[entry]);

    let out = env
        .cmd()
        .args([
            "--json", "models", "remove", entry.name, entry.name, "--force",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "duplicate names must not error (dedupe); got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    // Deduped to a single target → the bare record shape (one element).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = format!(r#"{{"name":"{}","status":"removed"}}"#, entry.name);
    assert_eq!(
        stdout.trim_end(),
        expected,
        "duplicate names dedupe to a single bare record",
    );
    assert!(
        !paths.models_dir.join(entry.name).exists(),
        "model deleted exactly once",
    );
}
