//! Cross-product coverage for the four `bind_project` pre-state
//! combinations (Phase 4 / US1.d-1 / T160).
//!
//! Library-API tests using `binding::bind_project` directly. Harness
//! sync is bypassed entirely — these tests install an empty harness
//! module set via `HarnessModulesGuard` so the bind path is exercised
//! without a real harness in scope. Each test isolates its own
//! `TempDir` so they run in parallel without colliding.
//!
//! Coverage matrix:
//!
//! | # | Pre-state                                | Expected                          |
//! |---|------------------------------------------|-----------------------------------|
//! | 1 | no marker, no DB row                     | created_marker, rebind_from=None  |
//! | 2 | marker present, DB row to same workspace | !created_marker, rebind_from=None |
//! | 3 | marker present, DB row to other workspace| !created_marker, rebind_from=Some |
//! | 4 | DB row exists but marker deleted         | created_marker, rebind_from=None  |
//!
//! Test 4 reflects the documented Phase A atomicity tier in
//! `src/workspace/binding.rs` — when the DB row's workspace_id matches
//! the new workspace_id, the bind is semantically idempotent against
//! the DB even if the marker was lost. The marker is re-materialised on
//! disk; the row's `bound_at` advances on the re-UPSERT.

use std::time::Duration;

use crate::common::{HarnessModulesGuard, lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

/// Build a fresh fixture: TempDir root, project subdir, fake home, and
/// `Paths` rooted under `<root>/.tome`. Returns the TempDir guard plus
/// the resolved Paths, project root, and fake home path.
struct Fixture {
    tmp: TempDir,
    paths: tome::paths::Paths,
    project: std::path::PathBuf,
    home: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let paths = lifecycle_paths(&tmp.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let home = tmp.path().join("fake-home");
        std::fs::create_dir_all(&home).expect("create home");
        Self {
            tmp,
            paths,
            project,
            home,
        }
    }

    fn deps(&self) -> BindDeps<'_> {
        BindDeps {
            paths: &self.paths,
            home_root: &self.home,
        }
    }

    fn project_canonical(&self) -> std::path::PathBuf {
        self.project.canonicalize().expect("canonicalize project")
    }
}

/// Read the bound_at column for a project's row.
fn read_bound_at(paths: &tome::paths::Paths, project_path: &std::path::Path) -> i64 {
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: crate::common::stub_embedder_seed(),
            reranker: crate::common::stub_reranker_seed(),
            summariser: crate::common::stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT bound_at FROM workspace_projects WHERE project_path = ?1",
        rusqlite::params![project_path.to_string_lossy().into_owned()],
        |row| row.get(0),
    )
    .expect("read bound_at")
}

/// Count rows in `workspace_projects` for a given project_path.
fn count_rows(paths: &tome::paths::Paths, project_path: &std::path::Path) -> i64 {
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: crate::common::stub_embedder_seed(),
            reranker: crate::common::stub_reranker_seed(),
            summariser: crate::common::stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT COUNT(*) FROM workspace_projects WHERE project_path = ?1",
        rusqlite::params![project_path.to_string_lossy().into_owned()],
        |row| row.get(0),
    )
    .expect("count rows")
}

// ---------------------------------------------------------------------------
// 1. Fresh project: no marker, no DB row → created_marker, rebind_from=None.
// ---------------------------------------------------------------------------

#[test]
fn bind_creates_marker_when_none_exists() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);
    let fx = Fixture::new();
    seed_workspace(&fx.paths, "ws-a");

    // Pre-condition: no marker yet.
    assert!(
        !fx.project.join(".tome").exists(),
        "fresh project must not have a marker"
    );

    let name = WorkspaceName::parse("ws-a").unwrap();
    let outcome = binding::bind_project(&fx.project, name, false, &fx.deps()).expect("bind");

    assert!(
        outcome.created_marker,
        "created_marker must be true on first bind"
    );
    assert!(
        outcome.rebind_from.is_none(),
        "rebind_from must be None on first bind"
    );

    // Marker landed on disk with the workspace line.
    let cfg_path = fx.project_canonical().join(".tome").join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path).expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"ws-a\""),
        "config.toml must contain workspace line; got: {cfg}"
    );

    // Exactly one DB row.
    assert_eq!(count_rows(&fx.paths, &fx.project_canonical()), 1);

    // Pin TempDir lifetime past the assertion phase.
    drop(fx.tmp);
}

// ---------------------------------------------------------------------------
// 2. Re-bind to same workspace: marker exists, row exists → !created_marker,
//    rebind_from=None, bound_at advances, config.toml content unchanged.
// ---------------------------------------------------------------------------

#[test]
fn idempotent_bind_to_same_workspace_no_marker_change() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);
    let fx = Fixture::new();
    seed_workspace(&fx.paths, "ws-a");

    let name = WorkspaceName::parse("ws-a").unwrap();
    binding::bind_project(&fx.project, name.clone(), false, &fx.deps()).expect("bind 1");

    let canonical = fx.project_canonical();
    let cfg_path = canonical.join(".tome").join("config.toml");
    let cfg_before = std::fs::read_to_string(&cfg_path).expect("read config.toml 1");
    let bound_at_before = read_bound_at(&fx.paths, &canonical);

    // Wait long enough for second-granularity timestamps to advance.
    std::thread::sleep(Duration::from_secs(1));

    let outcome = binding::bind_project(&fx.project, name, false, &fx.deps()).expect("bind 2");

    assert!(
        !outcome.created_marker,
        "created_marker must be false when marker already exists"
    );
    assert!(
        outcome.rebind_from.is_none(),
        "rebind_from must be None for same-workspace re-bind"
    );

    // config.toml content is byte-for-byte identical (the workspace
    // line is the only content; nothing else can drift).
    let cfg_after = std::fs::read_to_string(&cfg_path).expect("read config.toml 2");
    assert_eq!(
        cfg_before, cfg_after,
        "marker config.toml must be byte-identical on idempotent re-bind",
    );

    // bound_at advanced on the re-UPSERT.
    let bound_at_after = read_bound_at(&fx.paths, &canonical);
    assert!(
        bound_at_after > bound_at_before,
        "bound_at must advance on re-UPSERT: was {bound_at_before}, now {bound_at_after}",
    );

    drop(fx.tmp);
}

// ---------------------------------------------------------------------------
// 3. Rebind to a different workspace → rebind_from=Some(ws-a),
//    workspace=ws-b, marker updated, single DB row.
// ---------------------------------------------------------------------------

#[test]
fn rebind_to_different_workspace_upserts() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);
    let fx = Fixture::new();
    seed_workspace(&fx.paths, "ws-a");
    seed_workspace(&fx.paths, "ws-b");

    let a = WorkspaceName::parse("ws-a").unwrap();
    let b = WorkspaceName::parse("ws-b").unwrap();

    binding::bind_project(&fx.project, a, false, &fx.deps()).expect("bind ws-a");
    let outcome = binding::bind_project(&fx.project, b, false, &fx.deps()).expect("bind ws-b");

    assert_eq!(
        outcome.rebind_from.as_ref().map(|n| n.as_str()),
        Some("ws-a"),
        "rebind_from must name the prior workspace",
    );
    assert_eq!(
        outcome.workspace.as_str(),
        "ws-b",
        "workspace must reflect the new binding",
    );

    let canonical = fx.project_canonical();

    // Marker carries the new workspace line.
    let cfg = std::fs::read_to_string(canonical.join(".tome").join("config.toml"))
        .expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"ws-b\""),
        "config.toml must reflect new workspace; got: {cfg}",
    );
    assert!(
        !cfg.contains("workspace = \"ws-a\""),
        "config.toml must not contain stale workspace; got: {cfg}",
    );

    // Exactly one row.
    assert_eq!(
        count_rows(&fx.paths, &canonical),
        1,
        "UPSERT must keep one row per project_path",
    );

    drop(fx.tmp);
}

// ---------------------------------------------------------------------------
// 4. DB row survives marker deletion: re-bind to the same workspace
//    re-creates the marker, does NOT report a rebind, single row preserved.
// ---------------------------------------------------------------------------

#[test]
fn stale_db_row_after_marker_deletion_heals_on_rebind() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);
    let fx = Fixture::new();
    seed_workspace(&fx.paths, "ws-a");

    let name = WorkspaceName::parse("ws-a").unwrap();
    binding::bind_project(&fx.project, name.clone(), false, &fx.deps()).expect("bind 1");

    let canonical = fx.project_canonical();
    let marker_dir = canonical.join(".tome");
    assert!(marker_dir.exists(), "marker must exist after first bind");

    // Manually delete the marker — simulates the orphan state where
    // `<project>/.tome/` was removed without going through `tome` (rm -rf,
    // or partial-failure between DB UPSERT commit and marker landing).
    std::fs::remove_dir_all(&marker_dir).expect("remove marker");
    assert!(!marker_dir.exists(), "marker must be gone");

    // Re-bind to the SAME workspace. The DB row still references ws-a;
    // since the new workspace_id matches the row's existing workspace_id,
    // `rebind_from` stays None per `binding.rs` semantics.
    let outcome = binding::bind_project(&fx.project, name, false, &fx.deps()).expect("bind 2");

    assert!(
        outcome.created_marker,
        "created_marker must be true when marker was absent",
    );
    assert!(
        outcome.rebind_from.is_none(),
        "rebind_from must be None when re-binding to same workspace",
    );

    // Marker re-materialised.
    let cfg = std::fs::read_to_string(marker_dir.join("config.toml"))
        .expect("read config.toml after heal");
    assert!(
        cfg.contains("workspace = \"ws-a\""),
        "healed marker must reflect workspace; got: {cfg}",
    );

    // Still exactly one row.
    assert_eq!(
        count_rows(&fx.paths, &canonical),
        1,
        "re-bind must not duplicate the row",
    );

    drop(fx.tmp);
}
