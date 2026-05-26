//! Phase 4 / US2.a-2 — `tome workspace rename <old> <new>` library-API tests.
//!
//! Exercises [`tome::workspace::rename::rename`] directly. The CLI
//! binary surface is a thin emit wrapper around the library API; CLI
//! exit-code coverage is enforced by `tests/exit_codes.rs`.

mod common;

use std::path::{Path, PathBuf};

use common::{lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::error::TomeError;
use tome::index::{self, OpenOptions};
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
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

fn workspace_exists(paths: &tome::paths::Paths, name: &str) -> bool {
    let conn = open_central(paths);
    conn.query_row(
        "SELECT 1 FROM workspaces WHERE name = ?1",
        rusqlite::params![name],
        |_| Ok(()),
    )
    .is_ok()
}

/// Manually seed a `workspace_projects` row for `(workspace_name,
/// project_path)`. The project's marker `.tome/config.toml` is also
/// written so the rename pre-check sees a healthy binding.
fn seed_bound_project(paths: &tome::paths::Paths, workspace_name: &str, project_root: &Path) {
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

#[test]
fn rename_invalid_new_name_exits_15() {
    // The WorkspaceName::parse gate fires before rename is called.
    let err = WorkspaceName::parse("Bad!").unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn rename_to_existing_name_exits_14() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("foo"), false, &paths).expect("init foo");
    workspace::init::init(parse("bar"), false, &paths).expect("init bar");

    let err = workspace::rename::rename(parse("foo"), parse("bar"), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceAlreadyExists { .. }),
        "expected WorkspaceAlreadyExists, got {err:?}",
    );
    assert_eq!(err.exit_code(), 14);

    // No state change: both workspaces still present under their
    // original names.
    assert!(workspace_exists(&paths, "foo"));
    assert!(workspace_exists(&paths, "bar"));
}

#[test]
fn rename_global_refused_exits_15() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init mine");

    let err = workspace::rename::rename(parse("global"), parse("mine"), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNameInvalid { .. }),
        "expected WorkspaceNameInvalid, got {err:?}",
    );
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn rename_to_global_refused_exits_15() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init mine");

    let err = workspace::rename::rename(parse("mine"), parse("global"), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNameInvalid { .. }),
        "expected WorkspaceNameInvalid, got {err:?}",
    );
    assert_eq!(err.exit_code(), 15);

    // Cross-check: `mine` still exists, `global` (the seeded one) still
    // exists.
    assert!(workspace_exists(&paths, "mine"));
    assert!(workspace_exists(&paths, "global"));
}

#[test]
fn rename_zero_bound_projects_happy_path() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let outcome_init = workspace::init::init(parse("mine"), false, &paths).expect("init");
    let old_dir = outcome_init.path.clone();

    let outcome = workspace::rename::rename(parse("mine"), parse("yours"), &paths).expect("rename");
    assert_eq!(outcome.old_name.as_str(), "mine");
    assert_eq!(outcome.new_name.as_str(), "yours");
    assert_eq!(outcome.bound_projects_updated, 0);

    // DB rows: `mine` gone, `yours` present.
    assert!(!workspace_exists(&paths, "mine"));
    assert!(workspace_exists(&paths, "yours"));

    // Directory: <root>/workspaces/yours/ exists; <root>/workspaces/mine/
    // is gone.
    let new_dir = paths.workspace_dir(&parse("yours"));
    assert!(new_dir.is_dir(), "{} should exist", new_dir.display());
    assert!(!old_dir.exists(), "{} should be removed", old_dir.display());
    assert_eq!(outcome.workspace_dir, new_dir);
}

#[test]
fn rename_updates_bound_project_markers() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project_a = tmp.path().join("project-a");
    let project_b = tmp.path().join("project-b");
    seed_bound_project(&paths, "mine", &project_a);
    seed_bound_project(&paths, "mine", &project_b);

    let outcome = workspace::rename::rename(parse("mine"), parse("yours"), &paths).expect("rename");
    assert_eq!(outcome.bound_projects_updated, 2);

    // Each project's .tome/config.toml now names `yours`.
    for p in [&project_a, &project_b] {
        let body = std::fs::read_to_string(p.join(".tome/config.toml")).expect("read marker");
        assert!(
            body.contains("workspace = \"yours\""),
            "marker at {} should name `yours`: {body}",
            p.display(),
        );
    }

    // Central DB + dir state: `yours` is now the workspace name.
    assert!(!workspace_exists(&paths, "mine"));
    assert!(workspace_exists(&paths, "yours"));
    assert!(paths.workspace_dir(&parse("yours")).is_dir());
}

#[test]
fn rename_pre_check_missing_project_dir_exits_70() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project = tmp.path().join("project-vanish");
    seed_bound_project(&paths, "mine", &project);
    // Vanish the project directory.
    std::fs::remove_dir_all(&project).expect("remove project dir");

    let err = workspace::rename::rename(parse("mine"), parse("yours"), &paths).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceMalformed { .. }),
        "expected WorkspaceMalformed, got {err:?}",
    );
    assert_eq!(err.exit_code(), 70);

    // No state change: `mine` still in DB; `<root>/workspaces/mine/`
    // still exists; `yours` still absent.
    assert!(workspace_exists(&paths, "mine"));
    assert!(!workspace_exists(&paths, "yours"));
    let old_dir = paths.workspace_dir(&parse("mine"));
    let new_dir = paths.workspace_dir(&parse("yours"));
    assert!(old_dir.is_dir());
    assert!(!new_dir.exists());
}

#[test]
fn rename_to_same_name_is_usage_error() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("foo"), false, &paths).expect("init");

    let err = workspace::rename::rename(parse("foo"), parse("foo"), &paths).unwrap_err();
    assert!(matches!(err, TomeError::Usage(_)), "got {err:?}");
    assert_eq!(err.exit_code(), 2);

    // Cross-check zero state change.
    assert!(workspace_exists(&paths, "foo"));
}

/// T-B1: a bound project marker that carries the optional `harnesses`
/// field (per data-model §7) must keep that field after rename. The
/// old wholesale `format!` rewrite dropped it; the toml_edit rewrite
/// preserves it.
#[test]
fn rename_preserves_marker_harnesses_field() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project = tmp.path().join("rich-project");
    std::fs::create_dir_all(project.join(".tome")).expect("create .tome");
    // A marker carrying both the required `workspace` and the optional
    // `harnesses` array (composition references and exclusions both
    // legal per data-model §7).
    let pre_body = "workspace = \"mine\"\nharnesses = [\"[workspace]\", \"!cursor\"]\n";
    let marker_path = project.join(".tome").join("config.toml");
    std::fs::write(&marker_path, pre_body).expect("write marker");

    // Seed the workspace_projects row so the rename pre-check is happy.
    {
        let conn = open_central(&paths);
        let workspace_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE name = ?1",
                rusqlite::params!["mine"],
                |row| row.get(0),
            )
            .expect("lookup workspace_id");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        conn.execute(
            "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![project.to_string_lossy().to_string(), workspace_id, now],
        )
        .expect("seed workspace_projects");
    }

    let outcome = workspace::rename::rename(parse("mine"), parse("yours"), &paths).expect("rename");
    assert_eq!(outcome.bound_projects_updated, 1);

    let post_body = std::fs::read_to_string(&marker_path).expect("read marker");
    // The `workspace` field is updated.
    assert!(
        post_body.contains("workspace = \"yours\""),
        "workspace not renamed: {post_body}",
    );
    // The `harnesses` array survives the rewrite intact — toml_edit
    // preserves comments, order, and untouched keys.
    assert!(
        post_body.contains("harnesses = [\"[workspace]\", \"!cursor\"]"),
        "harnesses array lost after rename: {post_body}",
    );
}

/// US3 ScopeKind/Subsystem tests live in their own files; here we just
/// keep one happy-path assertion exercising the `bound_projects_updated`
/// counter via the [`PathBuf`] in [`tome::workspace::rename::RenameOutcome`].
#[test]
fn rename_outcome_workspace_dir_is_post_rename_path() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("alpha"), false, &paths).expect("init");

    let outcome = workspace::rename::rename(parse("alpha"), parse("beta"), &paths).expect("rename");
    let expected: PathBuf = paths.root.join("workspaces/beta");
    assert_eq!(outcome.workspace_dir, expected);
}
