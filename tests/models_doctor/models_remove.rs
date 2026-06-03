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

use crate::common::{ToolEnv, fabricate_installed_models, paths_for};
use tome::embedding::registry::MODEL_REGISTRY;

fn embedder() -> &'static tome::embedding::registry::ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::registry::ModelKind::Embedder))
        .expect("registry has an embedder")
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
        stderr.contains("--force"),
        "stderr must point the user at --force, got: {stderr}",
    );

    // State must NOT have changed.
    let model_dir = paths.models_dir.join(entry.name);
    assert!(
        model_dir.is_dir(),
        "refused remove must not delete the model dir"
    );
    assert!(
        model_dir.join("manifest.json").is_file(),
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
