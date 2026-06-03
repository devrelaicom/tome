//! Phase 4 / US5.b — FR-410 orphan `.tome.tmp.*` staging directory
//! cleanup on `tome doctor --fix`.
//!
//! The atomic-directory landing helper (`crate::util::atomic_dir`) builds
//! every populated directory in a sibling `.tome.tmp.<random>` staging
//! dir, then renames it into place. A crash between `TempDir::keep()`
//! and the final `rename(2)` leaves the staging dir on disk as an
//! orphan. `doctor --fix` sweeps anything older than 1 hour.

use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use filetime::{FileTime, set_file_mtime};
use tempfile::TempDir;
use tome::doctor::orphan_cleanup::{STAGING_AGE_GATE, cleanup_stale_staging_dirs};
use tome::util::atomic_dir::STAGING_PREFIX;

fn backdate(path: &Path, age: Duration) {
    let target = SystemTime::now() - age;
    set_file_mtime(path, FileTime::from_system_time(target))
        .unwrap_or_else(|e| panic!("backdate {}: {e}", path.display()));
}

#[test]
fn stale_staging_dir_older_than_age_gate_is_removed() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.workspaces_dir).unwrap();
    fabricate_all_registry_models(&paths);

    let stale = paths.workspaces_dir.join(format!("{STAGING_PREFIX}stale1"));
    std::fs::create_dir(&stale).unwrap();
    // Put a file inside so `remove_dir_all` has something to remove.
    std::fs::write(stale.join("scratch"), b"in-progress\n").unwrap();
    backdate(&stale, STAGING_AGE_GATE + Duration::from_secs(60));

    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(
        removed, 1,
        "exactly one stale staging dir should be removed"
    );
    assert!(!stale.exists(), "stale staging dir must be gone");
}

#[test]
fn fresh_staging_dir_within_age_gate_is_kept() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.workspaces_dir).unwrap();
    fabricate_all_registry_models(&paths);

    let fresh = paths.workspaces_dir.join(format!("{STAGING_PREFIX}fresh1"));
    std::fs::create_dir(&fresh).unwrap();
    // Recently-modified — within the age gate by construction.
    backdate(&fresh, Duration::from_secs(60));

    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(removed, 0);
    assert!(fresh.exists(), "fresh staging dir must survive cleanup");
}

#[test]
fn non_staging_dirs_are_untouched_even_when_stale() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.workspaces_dir).unwrap();
    fabricate_all_registry_models(&paths);

    let real_workspace = paths.workspaces_dir.join("global");
    std::fs::create_dir(&real_workspace).unwrap();
    backdate(&real_workspace, STAGING_AGE_GATE * 10);

    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(removed, 0, "non-staging dirs must NOT be removed");
    assert!(real_workspace.exists());
}

#[test]
fn cleanup_sweeps_bound_project_parents() {
    // Bind a project in the DB, then plant a stale staging dir at
    // `<project>/.tome.tmp.<rand>` (sibling of the marker dir). The
    // cleanup should follow `workspace_projects` to find it.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    crate::common::seed_workspace(&paths, "proj-ws");

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"proj-ws\"\n",
    )
    .unwrap();

    // Insert into workspace_projects.
    let (e, r, s) = (
        crate::common::stub_embedder_seed(),
        crate::common::stub_reranker_seed(),
        crate::common::stub_summariser_seed(),
    );
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .unwrap();
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = 'proj-ws'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let now_unix = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (workspace_id, project_path, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![ws_id, project_root.to_str().unwrap(), now_unix],
    )
    .unwrap();
    drop(conn);

    // Plant a stale staging at `<project>/.tome.tmp.foo`.
    let staging = project_root.join(format!("{STAGING_PREFIX}foo"));
    std::fs::create_dir(&staging).unwrap();
    backdate(&staging, STAGING_AGE_GATE + Duration::from_secs(60));

    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(removed, 1);
    assert!(!staging.exists());
}

#[test]
fn cleanup_is_silent_when_workspaces_dir_absent() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // workspaces dir intentionally NOT created.
    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(removed, 0);
}

// ---------------------------------------------------------------------------
// PR-E S-M6: symlink refusal during the sweep walk.
// ---------------------------------------------------------------------------

/// A hostile catalog or developer might plant a symlink at
/// `<workspaces>/.tome.tmp.evil` pointing at a sensitive directory. The
/// sweep walks `read_dir` + `metadata()` (follows symlinks) +
/// `remove_dir_all`; without explicit refusal it would recursively
/// delete through the link. The fix mirrors
/// `mcp/tools/get_skill.rs::walk_dir`: inspect `entry.file_type()`
/// (which does NOT follow symlinks) and skip symlinked entries before
/// any further inspection.
#[cfg(unix)]
#[test]
fn refuses_planted_symlink_during_sweep() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.workspaces_dir).unwrap();

    // Sensitive directory that must remain untouched.
    let payload_dir = env.home_path().join("sensitive");
    std::fs::create_dir(&payload_dir).unwrap();
    std::fs::write(payload_dir.join("secret.txt"), b"keep me").unwrap();

    // Plant a symlink at `<workspaces_dir>/.tome.tmp.evil` -> sensitive/.
    let evil = paths.workspaces_dir.join(format!("{STAGING_PREFIX}evil"));
    std::os::unix::fs::symlink(&payload_dir, &evil).unwrap();
    // Backdate so even if we WERE willing to recurse, the age gate
    // wouldn't be the thing protecting us. The defence under test is
    // the symlink refusal itself.
    backdate(&evil, STAGING_AGE_GATE + Duration::from_secs(60));

    let removed = cleanup_stale_staging_dirs(&paths).unwrap();
    assert_eq!(removed, 0, "must refuse symlinks");

    // Symlink + payload undisturbed.
    assert!(
        std::fs::symlink_metadata(&evil)
            .unwrap()
            .file_type()
            .is_symlink(),
        "planted symlink must remain in place"
    );
    assert!(
        payload_dir.join("secret.txt").is_file(),
        "sensitive payload must not be touched"
    );
    assert_eq!(
        std::fs::read(payload_dir.join("secret.txt")).unwrap(),
        b"keep me".to_vec()
    );
}
