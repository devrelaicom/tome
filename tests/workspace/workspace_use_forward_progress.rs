//! FR-403 forward-progress coverage (Phase 4 / US1.d-1 / T161).
//!
//! The contract: `bind_project` commits the central-DB UPSERT and lands
//! the project marker **before** the harness sync runs. When sync fails
//! mid-way (HarnessClash exit 19, IO error, etc.) the binding state must
//! remain visible so the next `tome workspace use` invocation converges
//! the harness writes without re-binding.
//!
//! Tests model the CLI wrapper sequence directly:
//!
//! 1. `bind_project` → must succeed end-to-end.
//! 2. `sync_for_project_root` → expected to fail.
//! 3. Assert marker + DB row both present despite the sync failure.
//!
//! Test 1 covers HarnessClash via a pre-populated user-owned `tome`
//! entry — same pattern as `harness_sync_stub.rs`. Test 2 covers an IO
//! failure under Unix by pointing a synthetic harness's rules_file
//! target at a read-only directory (0o500).

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::commands::harness::sync_for_project_root;
use tome::harness::{
    BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy, StubHarness,
};
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

/// Shared fixture state — TempDir holds the root for the entire test.
struct Fixture {
    tmp: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    home: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let paths = lifecycle_paths(&tmp.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).expect("create project");

        let home = tmp.path().join("fake-home");
        std::fs::create_dir_all(&home).expect("create home");

        Self {
            tmp,
            paths,
            project,
            home,
            workspace,
        }
    }

    fn bind_deps(&self) -> BindDeps<'_> {
        BindDeps {
            paths: &self.paths,
            home_root: &self.home,
        }
    }

    /// Mirror the CLI wrapper: bind then sync. Returns the bind's
    /// canonicalised project root + the sync result.
    fn bind_then_sync(&self, force: bool) -> (PathBuf, Result<(), tome::error::TomeError>) {
        let outcome = binding::bind_project(
            &self.project,
            self.workspace.clone(),
            false,
            &self.bind_deps(),
        )
        .expect("bind_project");
        let result = sync_for_project_root(
            &outcome.project_root,
            &outcome.workspace,
            &self.bind_deps(),
            force,
        )
        .map(|_| ());
        (outcome.project_root, result)
    }
}

/// Verify the central DB has a `workspace_projects` row for the project.
fn db_row_exists(paths: &tome::paths::Paths, project_path: &Path) -> bool {
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
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_projects WHERE project_path = ?1",
            rusqlite::params![project_path.to_string_lossy().into_owned()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    count > 0
}

// ---------------------------------------------------------------------------
// 1. HarnessClash mid-sync: bind committed, sync returns exit 19.
// ---------------------------------------------------------------------------

#[test]
fn binding_commits_even_when_harness_clash_returns_exit_19() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);
    let fx = Fixture::build("test-workspace");

    // Pre-populate the stub harness's MCP config path with a user-owned
    // `tome` entry. `is_tome_owned` requires command="tome" + first
    // arg="mcp"; "evil"/"serve" satisfies neither.
    let mcp_path = fx.project.join("stub.mcp.json");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&conflict).unwrap())
        .expect("write conflict");

    // Global settings declare the stub as effective. The bind step
    // writes a fresh marker (overwriting anything we'd pre-staged here)
    // so the harnesses list has to come from a layer the bind doesn't
    // touch — global config.toml is the natural fit.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &fx.paths.global_config_file,
        "[harness]\nenabled = [\"stub\"]\n",
    )
    .expect("write global config");

    let (project_root, result) = fx.bind_then_sync(false);

    let err = result.expect_err("sync must clash");
    assert_eq!(
        err.exit_code(),
        19,
        "want HarnessClash exit 19; got {err:?}"
    );

    // Forward-progress invariants per FR-403:
    // 1. The project marker is present and names the workspace.
    let cfg_path = project_root.join(".tome").join("config.toml");
    assert!(cfg_path.is_file(), "marker config.toml must exist");
    let cfg = std::fs::read_to_string(&cfg_path).expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"test-workspace\""),
        "marker must name the workspace; got: {cfg}",
    );

    // 2. The DB row is present.
    assert!(
        db_row_exists(&fx.paths, &project_root),
        "workspace_projects row must persist past the sync clash",
    );

    drop(fx.tmp);
}

// ---------------------------------------------------------------------------
// 2. IO failure mid-sync: bind committed, sync returns exit 7.
// ---------------------------------------------------------------------------

/// Synthetic harness whose `rules_file_target` lives under a read-only
/// directory (0o500) so the rules-file write fails with IO error.
/// Identical to `StubHarness` otherwise.
struct FailingStubHarness {
    rules_dir: PathBuf,
}

impl HarnessModule for FailingStubHarness {
    fn name(&self) -> &'static str {
        "failing-stub"
    }
    fn description(&self) -> &'static str {
        "deterministic test-only harness whose rules write fails"
    }
    fn detect(&self, _home: &Path) -> bool {
        true
    }
    fn rules_file_target(&self, _project_root: &Path) -> PathBuf {
        // Hand back a path inside the pre-created read-only directory.
        // The sync algorithm will attempt to write here and fail with an
        // EACCES (or equivalent) IO error.
        self.rules_dir.join("IMPOSSIBLE.md")
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        // StandaloneFile path is the simplest write to fail.
        RulesFileStrategy::StandaloneFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("failing-stub.mcp.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

#[cfg(unix)]
#[test]
fn binding_commits_even_when_harness_io_fails() {
    use std::os::unix::fs::PermissionsExt;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let fx = Fixture::build("test-workspace");

    // Pre-create a read-only directory inside the project for the
    // failing-stub's rules-file target. 0o500 = r-x for owner; writes
    // are denied even by the owner.
    let read_only_dir = fx.project.join(".read-only-dir");
    std::fs::create_dir(&read_only_dir).expect("create read-only dir");

    let _guard = HarnessModulesGuard::install(vec![Box::new(FailingStubHarness {
        rules_dir: read_only_dir.clone(),
    })]);

    // Flip the dir to 0o500 AFTER install so we don't have to chmod-back
    // before the guard drops.
    let mut perms = std::fs::metadata(&read_only_dir).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&read_only_dir, perms).expect("chmod 0500");

    // Global config declares the failing stub as effective (the bind
    // step would otherwise wipe a hand-written harnesses key from the
    // project marker).
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &fx.paths.global_config_file,
        "[harness]\nenabled = [\"failing-stub\"]\n",
    )
    .expect("write global config");

    let (project_root, result) = fx.bind_then_sync(false);

    // Restore parent perms BEFORE the assertions so TempDir cleanup
    // doesn't fight a read-only directory. The chmod runs regardless of
    // assertion outcomes — the assertions follow.
    let mut restore = std::fs::metadata(&read_only_dir).unwrap().permissions();
    restore.set_mode(0o700);
    std::fs::set_permissions(&read_only_dir, restore).ok();

    let err = result.expect_err("sync must fail with IO error");
    // Some filesystems (tmpfs without strict mode enforcement, root,
    // certain CI envs) may permit the write despite 0o500. In that case
    // the test would fail to surface; we assert the documented exit
    // code (7 = Io) but tolerate exit 18/19 if the test fixture's
    // read-only dir didn't actually block on this filesystem — see the
    // matching helper note in tests/atomic_dir.rs.
    let code = err.exit_code();
    assert!(
        code == 7,
        "want Io exit 7 from sync; got {code} (err: {err:?})",
    );

    // Forward-progress invariants:
    let cfg_path = project_root.join(".tome").join("config.toml");
    assert!(cfg_path.is_file(), "marker config.toml must exist");
    let cfg = std::fs::read_to_string(&cfg_path).expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"test-workspace\""),
        "marker must name the workspace; got: {cfg}",
    );
    assert!(
        db_row_exists(&fx.paths, &project_root),
        "workspace_projects row must persist past the sync IO failure",
    );

    drop(fx.tmp);
}
