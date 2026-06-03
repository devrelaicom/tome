//! Phase 4 / US5.a (T376a) — FR-563 read-only invariant test.
//!
//! `tome doctor` (no `--fix`) must NOT mutate any file under
//! `<root>/` or `<project>/.tome/`. We verify by walking both trees
//! before + after, collecting `(path, mtime)` for every regular file,
//! and asserting nothing changed.
//!
//! The CLI `doctor --fix` path does mutate (re-downloads, re-clones,
//! migration commits); that path is exercised in the existing
//! `tests/doctor.rs` repair-path tests. This file pins the inverse: the
//! default invocation is purely diagnostic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

/// Walk every regular file under `root` and capture its mtime. Returns
/// a `BTreeMap<PathBuf, SystemTime>` for deterministic comparison.
/// Hidden files and dotted dirs are included so we catch `.tome/` and
/// the central DB.
fn snapshot_mtimes(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return out;
    }
    walk(root, &mut out);
    out
}

fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, SystemTime>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            walk(&path, out);
        } else if meta.is_file()
            && let Ok(mtime) = meta.modified()
        {
            out.insert(path, mtime);
        }
    }
}

#[test]
fn doctor_without_fix_does_not_mutate_root_or_project_trees() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    // Seed the workspace's RULES.md so the binding-rules-copy check has
    // something to compare against.
    let ws = WorkspaceName::parse("alpha").unwrap();
    let src_rules = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src_rules.parent().unwrap()).unwrap();
    std::fs::write(&src_rules, b"workspace rules\n").unwrap();

    // Build a project marker bound to `alpha`.
    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"alpha\"\n",
    )
    .unwrap();
    std::fs::write(project_root.join(".tome/RULES.md"), b"workspace rules\n").unwrap();

    let home_tmp = TempDir::new().unwrap();
    let scope = ResolvedScope {
        scope: Scope(ws.clone()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root.clone()),
    };

    // Snapshot BEFORE doctor.
    let before_root = snapshot_mtimes(&paths.root);
    let before_project = snapshot_mtimes(&project_root.join(".tome"));

    // Sleep briefly so any mutation would be observable on filesystems
    // with second-resolution mtime. 1.1s is enough for ext4/HFS+ on
    // Linux/macOS; APFS gives nanosecond resolution so the gap doesn't
    // strictly need to span a tick, but the small wait is cheap insurance.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Run doctor (no --fix, no --verify).
    let _report = doctor::assemble_report(&scope, &paths, home_tmp.path(), false).unwrap();

    // Snapshot AFTER.
    let after_root = snapshot_mtimes(&paths.root);
    let after_project = snapshot_mtimes(&project_root.join(".tome"));

    // The two snapshots MUST match exactly for both trees.
    assert_eq!(
        before_root, after_root,
        "files under <root>/ were mutated by `doctor` without --fix",
    );
    assert_eq!(
        before_project, after_project,
        "files under <project>/.tome/ were mutated by `doctor` without --fix",
    );
}
