//! Per-workspace RULES.md sync tests.
//!
//! The `tome workspace sync` COMMAND surface was removed pre-launch
//! (its multi-workspace fan-out is superseded by the unified `tome sync`
//! command, which is workspace-scoped to the resolved workspace's bound
//! projects). What remains testable here is the compute layer that
//! `tome sync` (and `regen-summary`) still depend on:
//!
//!  * [`tome::workspace::sync_one`] — fan out one workspace's central
//!    RULES.md to every bound project, partitioning the outcome into
//!    synced / unchanged / missing.
//!  * [`tome::workspace::sync::sync_rules_to_project`] — the single-project
//!    write the `tome sync` command reuses.
//!
//! Source-of-truth helpers (DB seed shape) mirror
//! `tests/workspace_regen_summary.rs` since both suites operate on
//! `workspaces` + `workspace_projects`.

use std::path::Path;

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open central DB")
}

fn seed_bound_project(paths: &Paths, workspace_name: &str, project_root: &Path) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\n"),
    )
    .expect("write project config.toml");
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            project_root.to_string_lossy().to_string(),
            workspace_id,
            now
        ],
    )
    .expect("seed workspace_projects");
}

/// Initialise a workspace and stamp its central RULES.md with the
/// supplied body, replacing whatever `workspace::init::init` wrote.
fn init_with_rules(paths: &Paths, workspace_name: &str, rules_body: &str) {
    workspace::init::init(parse(workspace_name), false, paths).expect("init workspace");
    std::fs::write(
        paths.workspace_rules_file(&parse(workspace_name)),
        rules_body,
    )
    .expect("overwrite central RULES.md");
}

// ---------------------------------------------------------------------------
// 1. `sync_one` fans out to EVERY bound project of one workspace.
// ---------------------------------------------------------------------------

#[test]
fn sync_one_fans_out_to_every_bound_project() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "Rules for ws-a body\n");

    let project_1 = tmp.path().join("proj-1");
    let project_2 = tmp.path().join("proj-2");
    seed_bound_project(&paths, "ws-a", &project_1);
    seed_bound_project(&paths, "ws-a", &project_2);

    // Pre-populate proj-1's marker RULES.md with stale content so we can
    // verify the sync actually overwrites it.
    std::fs::write(project_1.join(".tome/RULES.md"), b"STALE\n").unwrap();

    let outcome = workspace::sync_one(&parse("ws-a"), &paths).expect("sync_one");

    assert_eq!(
        outcome.synced_projects.len(),
        2,
        "both bound projects should sync: {outcome:?}",
    );

    let body_1 = std::fs::read(project_1.join(".tome/RULES.md")).unwrap();
    assert_eq!(body_1, b"Rules for ws-a body\n", "proj-1 not synced");
    let body_2 = std::fs::read(project_2.join(".tome/RULES.md")).unwrap();
    assert_eq!(body_2, b"Rules for ws-a body\n", "proj-2 not synced");
}

// ---------------------------------------------------------------------------
// 2. A missing project directory is reported in `missing_project_dirs`
//    and the sync does not fail.
// ---------------------------------------------------------------------------

#[test]
fn sync_one_missing_project_dir_is_reported_and_skipped() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a body\n");

    let project = tmp.path().join("project-gone");
    seed_bound_project(&paths, "ws-a", &project);
    // Now make the project disappear entirely (the row remains).
    std::fs::remove_dir_all(&project).unwrap();

    let outcome = workspace::sync_one(&parse("ws-a"), &paths).expect("sync_one");

    assert_eq!(
        outcome.missing_project_dirs.len(),
        1,
        "missing project should be reported: {outcome:?}",
    );
    assert!(
        outcome.missing_project_dirs[0].ends_with("project-gone"),
        "expected project-gone in missing list, got {:?}",
        outcome.missing_project_dirs,
    );
    assert!(outcome.synced_projects.is_empty());
    assert!(outcome.unchanged.is_empty());
}

// ---------------------------------------------------------------------------
// 3. Idempotent re-run: no writes (verified via mtime).
// ---------------------------------------------------------------------------

#[test]
fn sync_one_idempotent_re_run_no_writes() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "stable body\n");
    let project = tmp.path().join("proj");
    seed_bound_project(&paths, "ws-a", &project);

    // First sync writes the file.
    let first = workspace::sync_one(&parse("ws-a"), &paths).expect("first sync");
    assert_eq!(first.synced_projects.len(), 1);

    let dest = project.join(".tome/RULES.md");
    let mtime_before = std::fs::metadata(&dest).unwrap().modified().unwrap();

    // Wait long enough that any rename() would produce a distinct mtime
    // even on coarse-resolution filesystems (1500ms covers HFS+ and ext3
    // at 1s granularity).
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Second sync: bytes match, MUST NOT write.
    let second = workspace::sync_one(&parse("ws-a"), &paths).expect("second sync");
    assert_eq!(
        second.synced_projects.len(),
        0,
        "second sync should produce zero writes; got {second:?}",
    );
    assert_eq!(second.unchanged.len(), 1, "should land in unchanged");

    let mtime_after = std::fs::metadata(&dest).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "destination mtime changed despite byte-equal idempotent sync",
    );
}

// ---------------------------------------------------------------------------
// 4. Direct unit coverage of the extracted single-project helper. Guards
//    the byte-for-byte idempotence + missing-dir classification that the
//    project-scoped `tome sync` command reuses.
// ---------------------------------------------------------------------------

#[test]
fn sync_rules_to_project_is_idempotent() {
    use tome::workspace::sync::{RulesSync, sync_rules_to_project};
    let ws = parse("demo");
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("p");
    std::fs::create_dir_all(proj.join(".tome")).unwrap();
    assert_eq!(
        sync_rules_to_project(b"body\n", &proj, &ws).unwrap(),
        RulesSync::Synced
    );
    assert_eq!(
        sync_rules_to_project(b"body\n", &proj, &ws).unwrap(),
        RulesSync::Unchanged
    );
    let missing = tmp.path().join("nope");
    assert_eq!(
        sync_rules_to_project(b"body\n", &missing, &ws).unwrap(),
        RulesSync::MissingProjectDir
    );
}
