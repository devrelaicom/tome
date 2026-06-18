//! Task 2.3a — unified `tome sync` command tests.
//!
//! Exercises the in-process orchestrator (`tome::commands::sync`) directly:
//! the pure helpers `sync_one_project` / `sync_all` (and `run` for the
//! flag-validation path). Sync is pure I/O against the central DB +
//! per-project marker files, so the library API gives full coverage without
//! spinning up the binary.
//!
//! The DB-seed fixture (workspaces + workspace_projects) mirrors
//! `tests/workspace/workspace_sync.rs`; the seed helpers there are private to
//! that module, so the minimal shape is replicated inline here.

mod common;

use std::path::Path;

use common::{lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::cli::SyncArgs;
use tome::commands::sync::{sync_all, sync_one_project};
use tome::error::TomeError;
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
        },
    )
    .expect("open central DB")
}

/// Create the project marker (`<root>/.tome/config.toml`) and insert the
/// `workspace_projects` binding row.
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

/// Init a workspace and overwrite its central RULES.md with `rules_body`.
fn init_with_rules(paths: &Paths, workspace_name: &str, rules_body: &str) {
    workspace::init::init(parse(workspace_name), false, paths).expect("init workspace");
    std::fs::write(
        paths.workspace_rules_file(&parse(workspace_name)),
        rules_body,
    )
    .expect("overwrite central RULES.md");
}

fn rules_only_args() -> SyncArgs {
    SyncArgs {
        all: false,
        rules_only: true,
        harness_only: false,
        harness: None,
    }
}

// ---------------------------------------------------------------------------
// 1. Rules-only, current project: writes <project>/.tome/RULES.md.
// ---------------------------------------------------------------------------

#[test]
fn sync_rules_only_current_project_writes_rules_md() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "Workspace ws-a rules body\n");

    let project = tmp.path().join("proj");
    seed_bound_project(&paths, "ws-a", &project);

    let ws = parse("ws-a");
    let args = rules_only_args();
    let outcome = sync_one_project(&ws, &project, &args, &paths).expect("sync_one_project");

    // The destination matches the workspace's central RULES.md.
    let dest = project.join(".tome/RULES.md");
    let body = std::fs::read(&dest).unwrap();
    assert_eq!(body, b"Workspace ws-a rules body\n");

    // First write → classified `synced`; harness reconcile skipped.
    assert_eq!(outcome.rules, Some("synced"));
    assert_eq!(outcome.harness_changes, 0);

    // Re-run is idempotent: bytes already match → `unchanged`, no write.
    let outcome2 = sync_one_project(&ws, &project, &args, &paths).expect("re-run");
    assert_eq!(outcome2.rules, Some("unchanged"));
}

// ---------------------------------------------------------------------------
// 2. --all --rules-only fans out to every bound project.
// ---------------------------------------------------------------------------

#[test]
fn sync_all_rules_only_fans_out() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project(&paths, "ws-a", &project_a);
    seed_bound_project(&paths, "ws-a", &project_b);

    // Pre-populate with stale content so we can verify both were overwritten.
    std::fs::write(project_a.join(".tome/RULES.md"), b"STALE_A\n").unwrap();
    std::fs::write(project_b.join(".tome/RULES.md"), b"STALE_B\n").unwrap();

    let args = SyncArgs {
        all: true,
        rules_only: true,
        harness_only: false,
        harness: None,
    };
    let report = sync_all(&parse("ws-a"), &args, &paths).expect("sync_all");

    // Both projects appear in the report.
    assert_eq!(
        report.projects.len(),
        2,
        "expected both projects: {report:?}"
    );
    let projects: Vec<_> = report.projects.iter().map(|p| p.project.clone()).collect();
    assert!(
        projects.contains(&project_a),
        "missing proj-a: {projects:?}"
    );
    assert!(
        projects.contains(&project_b),
        "missing proj-b: {projects:?}"
    );

    // Both files were overwritten with the workspace body.
    assert_eq!(
        std::fs::read(project_a.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
    );
    assert_eq!(
        std::fs::read(project_b.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
    );

    // Each outcome: rules synced, harness reconcile skipped.
    for p in &report.projects {
        assert_eq!(p.rules, Some("synced"));
        assert_eq!(p.harness_changes, 0);
    }
}

// ---------------------------------------------------------------------------
// 3. Unknown --harness (not rules-only) errors with HarnessNotSupported.
// ---------------------------------------------------------------------------

#[test]
fn sync_unknown_harness_errors() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // A resolved scope with a project root so the current-project branch is
    // reached — but the unknown-harness validation fires first, before any
    // filesystem work.
    let project = tmp.path().join("proj");
    std::fs::create_dir_all(project.join(".tome")).unwrap();

    let scope = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(parse("global")),
        source: tome::workspace::ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
    };

    let args = SyncArgs {
        all: false,
        rules_only: false,
        harness_only: false,
        harness: Some("not-a-harness".to_string()),
    };

    let err =
        tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json).unwrap_err();
    assert!(
        matches!(err, TomeError::HarnessNotSupported { .. }),
        "expected HarnessNotSupported, got {err:?}",
    );
    assert_eq!(err.exit_code(), 18);
}
