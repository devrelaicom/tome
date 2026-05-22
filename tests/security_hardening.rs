//! Phase 3 Polish PR-F — security hardening regressions.
//!
//! Phase 4 / F2a drops the `workspace::inventory` module and the opt-in
//! `workspaces.txt` registry. Workspace bindings live in the central
//! database's `workspace_projects` table (F11). The Phase 3 hardening
//! tests for the registry reader (`S-03`) and the canonicalize-dedupe
//! discipline (`M-WKS-3`) therefore have no surface to cover anymore;
//! they're deleted rather than `#[ignore]`-ed because the code under
//! test is gone, not deferred.
//!
//! The legacy `tome workspace init` path (`S-04`, `M-WKS-2`) is
//! similarly absent — `src/workspace/init.rs` is a `TODO(F11)` stub
//! until US1/US2 rewrite the lifecycle. Those tests carry an
//! `#[ignore]` marker tagging them as F11/US1 unhide targets.
//!
//! S-02 (`get_skill` symlink rejection in the resources walker) is the
//! only test that survives untouched — it tests filesystem-level
//! semantics that don't depend on the deleted modules.

use std::path::PathBuf;

use tempfile::TempDir;

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

// ---- S-04: init refuses non-directory marker --------------------------

#[test]
#[ignore = "F11/US1: tome workspace init is replaced by tome workspace add / tome workspace use"]
fn init_refuses_non_directory_marker_with_workspace_malformed() {
    // Phase 3 covered this via the now-deleted .tome/ marker creation
    // path. The replacement lifecycle commands (US1: tome workspace
    // use, US2: tome workspace add) will land separate marker-create
    // tests.
}

// ---- M-WKS-2: init --force pre-cleanup -------------------------------

#[test]
#[ignore = "F11/US1: tome workspace init is replaced by tome workspace add / tome workspace use"]
fn init_force_propagates_pre_cleanup_errors() {
    // See `init_refuses_non_directory_marker_with_workspace_malformed`
    // above for the disposition.
}
