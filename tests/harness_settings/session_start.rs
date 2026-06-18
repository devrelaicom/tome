//! `tome harness session-start` reconciles the current project's files BEFORE
//! printing the routing directive (Task 3.2), and does so FAIL-SOFT: a sync
//! error must never block or fail the hook — the directive prints regardless.
//!
//! These tests drive `session_start::run` library-API style (no CLI binary),
//! reusing the multi-harness fixture pattern from `harness_sync_stub.rs`:
//! a bound project marker + a `HarnessModulesGuard` installing real harness
//! modules. Because `run` resolves the per-project home via `$HOME`
//! (`commands::harness::home_root`), each test installs a `HomeGuard` pointing
//! at the fixture's isolated temp home, and holds `HARNESS_OVERRIDE_MUTEX` for
//! its whole body (the override is process-global).

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::cli::HarnessSessionStartArgs;
use tome::output::Mode;
use tome::workspace::WorkspaceName;
use tome::workspace::scope::{ResolvedScope, Scope, ScopeSource};

/// Build a bound project marker rooted under the fixture's temp `$HOME`,
/// declaring the given harness list. Returns the home `TempDir` (kept alive),
/// the `Paths`, the project root, and the parsed `WorkspaceName`.
struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
    home_path: PathBuf,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses_toml: Option<&str>) -> Self {
        let env = ToolEnv::new();
        let home_path = env.home_path().to_path_buf();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = home_path.join("project");
        std::fs::create_dir_all(&project).expect("create project");

        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        let mut body = format!("workspace = \"{workspace_name}\"\n");
        if let Some(harnesses) = harnesses_toml {
            body.push_str(harnesses);
            body.push('\n');
        }
        std::fs::write(marker_dir.join("config.toml"), body).expect("write marker config");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
            home_path,
        }
    }

    /// A `ResolvedScope` bound to this project (mirrors a `ProjectMarker`
    /// resolution): the directive prints for `workspace`, and `project_root`
    /// points at the bound project so the reconcile fires.
    fn project_scope(&self) -> ResolvedScope {
        ResolvedScope {
            scope: Scope(self.workspace.clone()),
            source: ScopeSource::ProjectMarker,
            project_root: Some(self.project.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// Test A — reconcile-then-print: a bound project with claude-code effective
//          gets its CLAUDE.md reconciled (the Tome managed block lands) before
//          the directive prints.
// ---------------------------------------------------------------------------

#[test]
fn session_start_reconciles_project_before_printing() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"claude-code\"]"));
    // `run` -> `sync_one_project` -> `home_root()` reads `$HOME`; pin it to the
    // fixture's isolated temp home so the reconcile resolves the same paths.
    let _home = HomeGuard::install(&fx.home_path);

    let scope = fx.project_scope();
    let res = session_start_run(&fx, &scope);
    assert!(res.is_ok(), "session-start must succeed; got {res:?}");

    // The harness reconcile actually ran: claude-code's rules sink is
    // `<project>/CLAUDE.md`, and it now carries the Tome managed block.
    let claude_md = fx.project.join("CLAUDE.md");
    assert!(
        claude_md.is_file(),
        "claude-code's CLAUDE.md must exist after the session-start reconcile",
    );
    let body = std::fs::read_to_string(&claude_md).expect("read CLAUDE.md");
    assert!(
        body.contains("<!-- tome:begin -->") && body.contains("<!-- tome:end -->"),
        "CLAUDE.md must carry the Tome managed block after reconcile; got:\n{body}",
    );
}

// ---------------------------------------------------------------------------
// Test B — fail-soft when there is no bound project: the reconcile is skipped,
//          the directive still prints, and `run` returns Ok.
// ---------------------------------------------------------------------------

#[test]
fn session_start_no_project_is_fail_soft() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"claude-code\"]"));
    let _home = HomeGuard::install(&fx.home_path);

    // No project root → the reconcile branch is skipped entirely; the directive
    // still prints. `--workspace` pins the same seeded workspace so the directive
    // build has a real workspace to read.
    let scope = ResolvedScope {
        scope: Scope(fx.workspace.clone()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    };
    let res = session_start_run(&fx, &scope);
    assert!(
        res.is_ok(),
        "session-start with no project must still succeed; got {res:?}",
    );

    // No project ⇒ no CLAUDE.md was written anywhere under the project dir.
    assert!(
        !fx.project.join("CLAUDE.md").exists(),
        "no project reconcile must have happened",
    );
}

/// Drive `commands::harness::session_start::run` with the workspace pinned via
/// `--workspace` (so the directive build is deterministic) and `Mode::Human`.
fn session_start_run(fx: &Fixture, scope: &ResolvedScope) -> Result<(), tome::error::TomeError> {
    let args = HarnessSessionStartArgs {
        workspace: Some(fx.workspace.as_str().to_string()),
        harness: None,
    };
    tome::commands::harness::session_start::run(args, scope, &fx.paths, Mode::Human)
}
