//! Phase 4 / US2.c — `tome workspace sync [<name>]` tests.
//!
//! Exercises [`tome::commands::workspace::sync::assemble`] (the
//! pure-compute entry point) using the library API. The CLI binary
//! path is not driven here: sync is pure I/O against the central DB +
//! per-project marker files, so the library API gives full coverage
//! without spinning up the binary.
//!
//! Source-of-truth helpers (DB seed shape) mirror
//! `tests/workspace_regen_summary.rs` since both suites operate on
//! `workspaces` + `workspace_projects`.

use std::path::{Path, PathBuf};

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::cli::WorkspaceSyncArgs;
use tome::commands::workspace::sync::{
    WorkspaceSyncEntry, WorkspaceSyncReport, assemble as sync_assemble,
};
use tome::error::TomeError;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::workspace::{self, WorkspaceName, WorkspaceSyncOutcome};

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

fn args_for(name: Option<&str>) -> WorkspaceSyncArgs {
    WorkspaceSyncArgs {
        name: name.map(|s| s.to_owned()),
    }
}

fn outcome_for<'a>(report: &'a WorkspaceSyncReport, workspace: &str) -> &'a WorkspaceSyncOutcome {
    let entry: &WorkspaceSyncEntry = report
        .per_workspace
        .iter()
        .find(|e| e.workspace.as_str() == workspace)
        .unwrap_or_else(|| panic!("no entry for workspace `{workspace}` in report"));
    &entry.outcome
}

// ---------------------------------------------------------------------------
// 1. No arg → every workspace syncs.
// ---------------------------------------------------------------------------

#[test]
fn sync_with_no_arg_syncs_every_workspace() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "Rules for ws-a body\n");
    init_with_rules(&paths, "ws-b", "Rules for ws-b body\n");

    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project(&paths, "ws-a", &project_a);
    seed_bound_project(&paths, "ws-b", &project_b);

    // Pre-populate proj-a's marker RULES.md with stale content so we
    // can verify the sync actually overwrites it.
    std::fs::write(project_a.join(".tome/RULES.md"), b"STALE\n").unwrap();

    let report = sync_assemble(args_for(None), &paths).expect("assemble");

    // Both ws-a and ws-b appear in the report; plus the seeded
    // `global` workspace with zero bound projects.
    let names: Vec<&str> = report
        .per_workspace
        .iter()
        .map(|e| e.workspace.as_str())
        .collect();
    assert!(names.contains(&"ws-a"), "report missing ws-a: {names:?}");
    assert!(names.contains(&"ws-b"), "report missing ws-b: {names:?}");

    let body_a = std::fs::read(project_a.join(".tome/RULES.md")).unwrap();
    assert_eq!(body_a, b"Rules for ws-a body\n", "ws-a project not synced");
    let body_b = std::fs::read(project_b.join(".tome/RULES.md")).unwrap();
    assert_eq!(body_b, b"Rules for ws-b body\n", "ws-b project not synced");

    assert_eq!(
        outcome_for(&report, "ws-a").synced_projects.len(),
        1,
        "ws-a should sync its one bound project",
    );
    assert_eq!(
        outcome_for(&report, "ws-b").synced_projects.len(),
        1,
        "ws-b should sync its one bound project",
    );
    assert_eq!(report.total_synced, 2);
}

// ---------------------------------------------------------------------------
// 2. With name arg → only that workspace syncs.
// ---------------------------------------------------------------------------

#[test]
fn sync_with_name_only_syncs_that_workspace() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a body\n");
    init_with_rules(&paths, "ws-b", "ws-b body\n");

    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project(&paths, "ws-a", &project_a);
    seed_bound_project(&paths, "ws-b", &project_b);

    // Pre-populate both projects' marker RULES.md with stale content
    // so we can verify ws-b's was left alone.
    std::fs::write(project_a.join(".tome/RULES.md"), b"STALE_A\n").unwrap();
    std::fs::write(project_b.join(".tome/RULES.md"), b"STALE_B\n").unwrap();

    let report = sync_assemble(args_for(Some("ws-a")), &paths).expect("assemble");

    // Only ws-a in the report.
    assert_eq!(report.per_workspace.len(), 1);
    assert_eq!(report.per_workspace[0].workspace.as_str(), "ws-a");

    // ws-a project file was updated.
    let body_a = std::fs::read(project_a.join(".tome/RULES.md")).unwrap();
    assert_eq!(body_a, b"ws-a body\n");

    // ws-b project file was NOT touched.
    let body_b = std::fs::read(project_b.join(".tome/RULES.md")).unwrap();
    assert_eq!(
        body_b, b"STALE_B\n",
        "ws-b project should not have been synced"
    );
}

// ---------------------------------------------------------------------------
// 3. Missing project directory is reported in `missing_project_dirs`
//    and the sync does not fail.
// ---------------------------------------------------------------------------

#[test]
fn sync_missing_project_dir_is_reported_and_skipped() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a body\n");

    let project = tmp.path().join("project-gone");
    seed_bound_project(&paths, "ws-a", &project);
    // Now make the project disappear entirely (the row remains).
    std::fs::remove_dir_all(&project).unwrap();

    let report = sync_assemble(args_for(Some("ws-a")), &paths).expect("assemble");

    let outcome = outcome_for(&report, "ws-a");
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
// 4. Idempotent re-run: no writes (verified via mtime).
// ---------------------------------------------------------------------------

#[test]
fn sync_idempotent_re_run_no_writes() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "stable body\n");
    let project = tmp.path().join("proj");
    seed_bound_project(&paths, "ws-a", &project);

    // First sync writes the file.
    let first = sync_assemble(args_for(Some("ws-a")), &paths).expect("first sync");
    assert_eq!(outcome_for(&first, "ws-a").synced_projects.len(), 1);

    let dest = project.join(".tome/RULES.md");
    let mtime_before = std::fs::metadata(&dest).unwrap().modified().unwrap();

    // Wait long enough that any rename() would produce a distinct
    // mtime even on coarse-resolution filesystems (1500ms covers HFS+
    // and ext3 at 1s granularity).
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Second sync: bytes match, MUST NOT write.
    let second = sync_assemble(args_for(Some("ws-a")), &paths).expect("second sync");
    let outcome = outcome_for(&second, "ws-a");
    assert_eq!(
        outcome.synced_projects.len(),
        0,
        "second sync should produce zero writes; got {outcome:?}",
    );
    assert_eq!(outcome.unchanged.len(), 1, "should land in unchanged");

    let mtime_after = std::fs::metadata(&dest).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "destination mtime changed despite byte-equal idempotent sync",
    );
}

// ---------------------------------------------------------------------------
// 5. Invalid workspace name → exit 15.
// ---------------------------------------------------------------------------

#[test]
fn sync_with_invalid_name_exits_15() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let err = sync_assemble(args_for(Some("Bad!Name")), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNameInvalid { .. }),
        "expected WorkspaceNameInvalid, got {err:?}",
    );
    assert_eq!(err.exit_code(), 15);
}

// ---------------------------------------------------------------------------
// 6. Unknown workspace → exit 13.
// ---------------------------------------------------------------------------

#[test]
fn sync_unknown_workspace_exits_13() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Init one workspace so the DB is bootstrapped (otherwise the
    // membership check short-circuits before the registry exists).
    workspace::init::init(parse("real"), false, &paths).expect("init real");

    let err = sync_assemble(args_for(Some("missing")), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNotFound { .. }),
        "expected WorkspaceNotFound, got {err:?}",
    );
    assert_eq!(err.exit_code(), 13);
}

// ---------------------------------------------------------------------------
// 7. JSON wire-shape pin (one fixed-state outcome). Follows the byte-
//    stable pattern from `tests/workspace_use_json_shape.rs`.
// ---------------------------------------------------------------------------

#[test]
fn report_serialises_to_byte_stable_json_for_empty_state() {
    let report = WorkspaceSyncReport {
        per_workspace: vec![WorkspaceSyncEntry {
            workspace: parse("demo"),
            outcome: WorkspaceSyncOutcome {
                synced_projects: vec![PathBuf::from("/tmp/proj")],
                unchanged: vec![],
                missing_project_dirs: vec![],
            },
        }],
        total_synced: 1,
        total_unchanged: 0,
        total_missing: 0,
    };
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"per_workspace":[{"workspace":"demo","outcome":{"synced_projects":["/tmp/proj"],"unchanged":[],"missing_project_dirs":[]}}],"total_synced":1,"total_unchanged":0,"total_missing":0}"#,
        "WorkspaceSyncReport wire shape drift"
    );
}
