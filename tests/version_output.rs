//! Phase 8 / US6 slice 2 — extended `tome --version` output.
//!
//! Asserts both the plain-text three-line form and the JSON form include
//! tome version + embedder identity + reranker identity. The model
//! identities come from `MODEL_REGISTRY` at compile time, so bumping a
//! model auto-bumps the output.

mod common;

use common::ToolEnv;
use serde_json::Value;
use tome::embedding::ModelKind;
use tome::embedding::registry::MODEL_REGISTRY;

fn embedder_entry() -> &'static tome::embedding::registry::ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, ModelKind::Embedder))
        .expect("embedder entry")
}

fn reranker_entry() -> &'static tome::embedding::registry::ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, ModelKind::Reranker))
        .expect("reranker entry")
}

#[test]
fn version_plain_text_emits_three_lines() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["--version"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "expected three lines, got {stdout:?}");

    let tome = env!("CARGO_PKG_VERSION");
    let embedder = embedder_entry();
    let reranker = reranker_entry();

    assert_eq!(lines[0], format!("tome {tome}"));
    assert_eq!(
        lines[1],
        format!("embedder: {} {}", embedder.name, embedder.version),
    );
    assert_eq!(
        lines[2],
        format!("reranker: {} {}", reranker.name, reranker.version),
    );
}

#[test]
fn version_json_emits_structured_record() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["--version", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");

    let tome = env!("CARGO_PKG_VERSION");
    let embedder = embedder_entry();
    let reranker = reranker_entry();

    assert_eq!(v["tome"], tome);
    assert_eq!(v["embedder"]["name"], embedder.name);
    assert_eq!(v["embedder"]["version"], embedder.version);
    assert_eq!(v["reranker"]["name"], reranker.name);
    assert_eq!(v["reranker"]["version"], reranker.version);
}

#[test]
fn version_json_flag_position_is_irrelevant() {
    // `--json --version` should produce the same JSON record as
    // `--version --json` (the pre-parse hook just scans env::args).
    let env = ToolEnv::new();
    let out = env.cmd().args(["--json", "--version"]).output().unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");
    assert!(v.get("tome").is_some());
    assert!(v["embedder"].is_object());
    assert!(v["reranker"].is_object());
}

#[test]
fn short_v_flag_also_triggers_the_version_handler() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["-V"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("tome "));
}
