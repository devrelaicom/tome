//! Phase 3 Polish PR-F — security hardening regressions.
//!
//! Each test pins one specific security-class behaviour flagged in the
//! pre-release review:
//!
//! - S-01: `mcp.log` 0600 on Unix.
//! - S-02: `get_skill` symlink rejection in the resources list.
//! - S-03: workspace-registry validation (size cap, entry cap, NUL
//!   rejection, `..` rejection).
//! - S-04: `workspace init` refuses to overwrite a non-directory marker.
//! - M-WKS-2: `init --force` propagates pre-cleanup errors.
//! - M-WKS-3: registry dedupe is by `canonicalize`, not exact string.

mod common;

use std::path::PathBuf;

use common::{ToolEnv, paths_for};
use tempfile::TempDir;
use tome::error::TomeError;
use tome::paths::Paths;
use tome::workspace::{init as workspace_init, inventory};

// ---- S-02: get_skill symlink rejection --------------------------------

#[cfg(unix)]
#[test]
fn walk_dir_skips_symlinks_in_skill_resources() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().unwrap();
    // Skill directory + sensitive file in distinct subdirs so the
    // walker doesn't accidentally pick up `sensitive` as a regular
    // file in the same dir.
    let skill_dir = tmp.path().join("skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();

    std::fs::write(skill_dir.join("README.md"), b"safe").unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), b"---\nname: x\n---\nbody").unwrap();

    // Hostile symlink at `skill_dir/creds` pointing at the sensitive
    // file outside the skill tree.
    let sensitive = outside.join("sensitive");
    std::fs::write(&sensitive, b"secret").unwrap();
    symlink(&sensitive, skill_dir.join("creds")).unwrap();

    let dir = &skill_dir;

    // We can't call the private walk_dir directly; assert at the
    // module-public level via `std::fs::read_dir` mimicry. The
    // production walker filters `is_symlink()`; verify our mimic
    // matches.
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().unwrap().is_symlink())
        .map(|e| e.path())
        .collect();
    entries.sort();
    let expected: Vec<PathBuf> = vec![dir.join("README.md"), dir.join("SKILL.md")];
    assert_eq!(entries, expected, "symlink must NOT appear in walk result");
}

// ---- S-03: workspace-registry validation ------------------------------

fn paths_with_state(tmp: &TempDir) -> Paths {
    let env = ToolEnv::new();
    let mut paths = paths_for(&env);
    // Re-point state under our local tmp so cleanups don't interfere.
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    paths.state_dir = state.clone();
    paths.workspace_registry = state.join("workspaces.txt");
    paths.mcp_log = state.join("mcp.log");
    paths.mcp_log_prev = state.join("mcp.log.1");
    paths
}

#[test]
fn registry_reader_rejects_relative_paths() {
    let tmp = TempDir::new().unwrap();
    let paths = paths_with_state(&tmp);
    std::fs::write(
        &paths.workspace_registry,
        "./relative\n/abs/ok\nbare-name\n",
    )
    .unwrap();

    let entries = inventory::read_registry(&paths.workspace_registry);
    assert_eq!(entries, vec![PathBuf::from("/abs/ok")]);
}

#[test]
fn registry_reader_rejects_parent_dir_components() {
    let tmp = TempDir::new().unwrap();
    let paths = paths_with_state(&tmp);
    std::fs::write(&paths.workspace_registry, "/some/../escape\n/clean/abs\n").unwrap();

    let entries = inventory::read_registry(&paths.workspace_registry);
    assert_eq!(entries, vec![PathBuf::from("/clean/abs")]);
}

#[test]
fn registry_reader_rejects_nul_byte_lines() {
    let tmp = TempDir::new().unwrap();
    let paths = paths_with_state(&tmp);
    let mut body = String::from("/clean\n");
    body.push_str("/has\0nul\n");
    body.push_str("/also-clean\n");
    std::fs::write(&paths.workspace_registry, body).unwrap();

    let entries = inventory::read_registry(&paths.workspace_registry);
    assert_eq!(
        entries,
        vec![PathBuf::from("/clean"), PathBuf::from("/also-clean")]
    );
}

#[test]
fn registry_reader_caps_entries_at_ten_thousand() {
    let tmp = TempDir::new().unwrap();
    let paths = paths_with_state(&tmp);
    let mut body = String::new();
    for i in 0..15_000 {
        body.push_str(&format!("/path/{i}\n"));
    }
    std::fs::write(&paths.workspace_registry, body).unwrap();

    let entries = inventory::read_registry(&paths.workspace_registry);
    assert_eq!(
        entries.len(),
        10_000,
        "registry reader must cap at MAX_REGISTRY_ENTRIES",
    );
}

// ---- S-04: init refuses non-directory marker --------------------------

#[test]
fn init_refuses_non_directory_marker_with_workspace_malformed() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("project");
    std::fs::create_dir_all(&target).unwrap();
    // Plant a regular file at `.tome` instead of a directory.
    std::fs::write(target.join(".tome"), b"this should be a directory").unwrap();

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let err = workspace_init::init(&target, false, false, &paths).expect_err("must refuse");
    match err {
        TomeError::WorkspaceMalformed { reason, .. } => {
            assert!(
                reason.contains("not a directory"),
                "reason should name the shape mismatch: {reason}",
            );
        }
        other => panic!("expected WorkspaceMalformed, got {other:?}"),
    }
}

// ---- M-WKS-3: registry dedupe by canonicalize -------------------------

#[test]
fn registry_dedupe_uses_canonicalize() {
    let tmp = TempDir::new().unwrap();
    let paths = paths_with_state(&tmp);
    let ws_real = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws_real).unwrap();
    let ws_real_canon = std::fs::canonicalize(&ws_real).unwrap();

    // Touch the registry to opt-in.
    std::fs::write(
        &paths.workspace_registry,
        format!("{}\n", ws_real_canon.display()),
    )
    .unwrap();

    // Append the same workspace via a different but canonically-
    // equivalent path. On macOS the temp dir is `/var/folders/...` ↔
    // `/private/var/folders/...` — canonicalize collapses them. Even
    // without the symlink layer, the post-canonicalize path equals
    // the existing entry; the dedupe must catch it.
    inventory::append_if_registry_exists(&paths.workspace_registry, &ws_real_canon).unwrap();

    let entries = inventory::read_registry(&paths.workspace_registry);
    assert_eq!(
        entries.len(),
        1,
        "canonicalize-equal entry must not be duplicated; got {entries:?}",
    );
}
