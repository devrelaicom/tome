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
            profile: None,
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
        harness: vec![],
        dry_run: false,
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
        harness: vec![],
        dry_run: false,
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
// #426. `--all` partial failure: the failing project is REPORTED (not just
//       warn!-logged), the succeeding project still lands, and the first
//       error's exit code is preserved by `run`.
// ---------------------------------------------------------------------------

#[test]
fn sync_all_partial_failure_is_reported_and_preserves_exit_code() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    let project_ok = tmp.path().join("proj-ok");
    let project_bad = tmp.path().join("proj-bad");
    seed_bound_project(&paths, "ws-a", &project_ok);
    seed_bound_project(&paths, "ws-a", &project_bad);

    // Sabotage proj-bad: its `.tome/RULES.md` destination is a NON-EMPTY
    // DIRECTORY, so the rules write's atomic rename fails with a real Io error.
    let dest = project_bad.join(".tome/RULES.md");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("occupier"), b"x").unwrap();

    let args = SyncArgs {
        all: true,
        rules_only: true,
        harness_only: false,
        harness: vec![],
        dry_run: false,
    };

    // `sync_all` itself: forward progress + the failure lands on the REPORT.
    let report = sync_all(&parse("ws-a"), &args, &paths).expect("sync_all is forward-progress");
    assert_eq!(report.projects.len(), 1, "{report:?}");
    assert_eq!(report.projects[0].project, project_ok);
    assert_eq!(report.projects[0].rules, Some("synced"));
    assert_eq!(report.failures.len(), 1, "{report:?}");
    assert_eq!(report.failures[0].project, project_bad);
    assert!(
        !report.failures[0].error.is_empty(),
        "failure carries the error text: {report:?}",
    );
    let first = report
        .first_error
        .as_ref()
        .expect("first_error preserved for the exit code");
    assert_eq!(
        first.exit_code(),
        7,
        "an Io failure keeps exit 7: {first:?}"
    );

    // The succeeding project's file landed despite the sibling failure.
    assert_eq!(
        std::fs::read(project_ok.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
    );

    // `run` (the CLI path): emits the report, THEN returns the first error —
    // the pre-#426 exit semantics, now with visible partial progress.
    let args2 = SyncArgs {
        all: true,
        rules_only: true,
        harness_only: false,
        harness: vec![],
        dry_run: false,
    };
    let scope = scope_without_marker("ws-a");
    let err = tome::commands::sync::run(args2, &scope, &paths, tome::output::Mode::Json)
        .expect_err("the first per-project error must set the exit code");
    assert_eq!(err.exit_code(), 7, "exit code preserved: {err:?}");
}

/// Build a `ResolvedScope` for `ws` with NO project marker — the bare-sync
/// case (issue #303). `project_root: None` drives the new fan-out branch.
fn scope_without_marker(ws: &str) -> tome::workspace::ResolvedScope {
    tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(parse(ws)),
        source: tome::workspace::ScopeSource::Config,
        project_root: None,
        overridden_project_marker: None,
    }
}

// ---------------------------------------------------------------------------
// #303-a. Bare `tome sync` outside a project (no marker, no --all) with >=1
//         bound project fans out to EVERY bound project — same as --all.
// ---------------------------------------------------------------------------

#[test]
fn bare_sync_no_marker_fans_out_to_bound_projects() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project(&paths, "ws-a", &project_a);
    seed_bound_project(&paths, "ws-a", &project_b);

    // Stale bodies so we can prove both were reconciled by the fan-out.
    std::fs::write(project_a.join(".tome/RULES.md"), b"STALE_A\n").unwrap();
    std::fs::write(project_b.join(".tome/RULES.md"), b"STALE_B\n").unwrap();

    // `--rules-only` keeps the reconcile off the $HOME-dependent harness path
    // while still exercising the bare-sync fan-out fallback end to end.
    let args = SyncArgs {
        all: false,
        rules_only: true,
        harness_only: false,
        harness: vec![],
        dry_run: false,
    };
    let scope = scope_without_marker("ws-a");

    // Bare sync with no marker MUST succeed (exit 0), not error.
    tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json)
        .expect("bare sync fans out cleanly");

    // Both bound projects were reconciled with the workspace body.
    assert_eq!(
        std::fs::read(project_a.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
        "proj-a not synced by bare-sync fan-out",
    );
    assert_eq!(
        std::fs::read(project_b.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
        "proj-b not synced by bare-sync fan-out",
    );
}

// ---------------------------------------------------------------------------
// #303-b. Bare `tome sync` with EXACTLY ONE bound project syncs that one
//         (subsumed by the --all path — a one-element fan-out).
// ---------------------------------------------------------------------------

#[test]
fn bare_sync_no_marker_single_bound_syncs_it() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    let project = tmp.path().join("only-proj");
    seed_bound_project(&paths, "ws-a", &project);
    std::fs::write(project.join(".tome/RULES.md"), b"STALE\n").unwrap();

    let args = rules_only_args();
    let scope = scope_without_marker("ws-a");

    tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json)
        .expect("bare sync single-bound succeeds");

    assert_eq!(
        std::fs::read(project.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
        "the single bound project was not synced",
    );
}

// ---------------------------------------------------------------------------
// #303-c. Bare `tome sync` with ZERO bound projects → detect-and-suggest
//         usage error (exit 2), message names the concrete next steps.
// ---------------------------------------------------------------------------

#[test]
fn bare_sync_no_marker_no_bindings_is_detect_and_suggest_error() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Workspace exists but has NO bound projects.
    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    let args = rules_only_args();
    let scope = scope_without_marker("ws-a");

    let err = tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json)
        .expect_err("no bindings must error");

    assert!(
        matches!(err, TomeError::Usage(_)),
        "expected Usage, got {err:?}",
    );
    assert_eq!(err.exit_code(), 2, "no-bindings usage error is exit 2");

    let msg = err.to_string();
    assert!(
        msg.contains("tome workspace use"),
        "message must name `tome workspace use`: {msg}",
    );
    assert!(
        msg.contains("tome sync --all"),
        "message must name `tome sync --all`: {msg}",
    );
}

// ---------------------------------------------------------------------------
// #303-d. Bare `tome sync` outside a project against a workspace with NO
//         registry row (no DB at all) → the SAME detect-and-suggest usage
//         error, never a bare WorkspaceNotFound (exit 13).
// ---------------------------------------------------------------------------

#[test]
fn bare_sync_no_marker_no_db_is_detect_and_suggest_error() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately NO workspace init and NO central DB.

    let args = rules_only_args();
    let scope = scope_without_marker("ws-a");

    let err = tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json)
        .expect_err("no db must error");

    assert!(
        matches!(err, TomeError::Usage(_)),
        "expected Usage (not WorkspaceNotFound), got {err:?}",
    );
    assert_eq!(err.exit_code(), 2);
}

// ---------------------------------------------------------------------------
// #303-e. `run()` with a resolved project marker (project_root: Some) syncs
//         EXACTLY that project, never a fan-out — proves the refactored
//         in-project `else if let Some(..)` branch end to end.
// ---------------------------------------------------------------------------

#[test]
fn in_project_run_syncs_only_that_project_not_a_fanout() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    init_with_rules(&paths, "ws-a", "ws-a rules\n");

    // Two bound projects; the scope marker resolves to project_a only.
    let project_a = tmp.path().join("proj-a");
    let project_b = tmp.path().join("proj-b");
    seed_bound_project(&paths, "ws-a", &project_a);
    seed_bound_project(&paths, "ws-a", &project_b);

    // Stale both so a reconcile is observable; only proj-a must change.
    std::fs::write(project_a.join(".tome/RULES.md"), b"STALE_A\n").unwrap();
    std::fs::write(project_b.join(".tome/RULES.md"), b"STALE_B\n").unwrap();

    let scope = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(parse("ws-a")),
        source: tome::workspace::ScopeSource::ProjectMarker,
        project_root: Some(project_a.clone()),
        overridden_project_marker: None,
    };

    // Bare `tome sync` (no --all) with a resolved marker → in-project branch.
    let args = rules_only_args();
    tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json)
        .expect("in-project sync succeeds");

    // proj-a reconciled to the workspace body...
    assert_eq!(
        std::fs::read(project_a.join(".tome/RULES.md")).unwrap(),
        b"ws-a rules\n",
        "the resolved project was not synced",
    );
    // ...and proj-b UNTOUCHED — no fan-out happened.
    assert_eq!(
        std::fs::read(project_b.join(".tome/RULES.md")).unwrap(),
        b"STALE_B\n",
        "in-project sync must NOT fan out to other bound projects",
    );
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
        overridden_project_marker: None,
    };

    let args = SyncArgs {
        all: false,
        rules_only: false,
        harness_only: false,
        harness: vec!["not-a-harness".to_string()],
        dry_run: false,
    };

    let err =
        tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json).unwrap_err();
    assert!(
        matches!(err, TomeError::HarnessNotSupported { .. }),
        "expected HarnessNotSupported, got {err:?}",
    );
    assert_eq!(err.exit_code(), 18);
}

// ---------------------------------------------------------------------------
// 3b. Phase 11 / US6 (T080): one BAD name among several repeated `--harness`
//     values still errors with HarnessNotSupported (every name is validated).
// ---------------------------------------------------------------------------

#[test]
fn sync_one_bad_harness_among_many_errors() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let project = tmp.path().join("proj");
    std::fs::create_dir_all(project.join(".tome")).unwrap();

    let scope = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(parse("global")),
        source: tome::workspace::ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
        overridden_project_marker: None,
    };

    // cursor is valid; nope is not. The whole run must error (exit 18).
    let args = SyncArgs {
        all: false,
        rules_only: false,
        harness_only: false,
        harness: vec!["cursor".to_string(), "nope".to_string()],
        dry_run: false,
    };

    let err =
        tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json).unwrap_err();
    assert!(
        matches!(err, TomeError::HarnessNotSupported { .. }),
        "expected HarnessNotSupported, got {err:?}",
    );
    assert_eq!(err.exit_code(), 18);
}

// ---------------------------------------------------------------------------
// 3c. Phase 11 / US6 (T080): an ALIAS `--harness antigravity-cli` passes
//     validation (resolves to the gemini module via `lookup`), so the run
//     proceeds past the eager name check.
// ---------------------------------------------------------------------------

#[test]
fn sync_alias_harness_passes_validation() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // `--rules-only` short-circuits the harness reconcile (so no $HOME-dependent
    // detection runs) while STILL exercising the name-validation gate — which is
    // skipped under rules-only. So instead use a real project + harness-only and
    // assert the error is NOT HarnessNotSupported: validation accepted the alias.
    let project = tmp.path().join("proj");
    std::fs::create_dir_all(project.join(".tome")).unwrap();
    std::fs::write(
        project.join(".tome").join("config.toml"),
        "workspace = \"global\"\n",
    )
    .unwrap();

    let scope = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(parse("global")),
        source: tome::workspace::ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
        overridden_project_marker: None,
    };

    let args = SyncArgs {
        all: false,
        rules_only: false,
        harness_only: true,
        harness: vec!["antigravity-cli".to_string()],
        dry_run: false,
    };

    // The eager validation must NOT reject the alias. The reconcile itself runs
    // against the real registry; whatever it returns, it is NOT an exit-18
    // HarnessNotSupported (which would mean validation rejected the alias).
    let result = tome::commands::sync::run(args, &scope, &paths, tome::output::Mode::Json);
    if let Err(e) = result {
        assert_ne!(
            e.exit_code(),
            18,
            "alias antigravity-cli must pass name validation; got {e:?}",
        );
    }
}
