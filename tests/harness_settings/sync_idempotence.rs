//! Cross-harness byte-for-byte idempotence proof for `sync_for_project_root`
//! (Phase 4 / US1.d-2a — T-B2 from the reviewer pass).
//!
//! `harness_sync_stub.rs::idempotent_resync_no_disk_changes` covers
//! single-harness idempotence; this file proves the FR-525 invariant
//! across a multi-harness effective list (`claude-code` real harness +
//! `StubHarness` fixture). The relevance is that the dedup logic in
//! `sync::sync_project` walks every snapshot in turn — a stale entry
//! in `outcome.added` / `outcome.updated` would surface as an mtime
//! advance on one of the four targets.
//!
//! Both tests use the `HARNESS_MODULES_OVERRIDE` slot to install a
//! deterministic two-module list. The `crate::common::HARNESS_OVERRIDE_MUTEX` discipline from
//! `harness_sync_stub.rs` applies — the slot is process-global.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::common::{HarnessModulesGuard, lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::commands::harness::sync_for_project_root;
use tome::harness::StubHarness;
use tome::harness::claude_code::ClaudeCode;
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

/// Snapshot of state shared across the bind + resync calls.
struct Fixture {
    _home: TempDir,
    _project: TempDir,
    paths: tome::paths::Paths,
    home_path: PathBuf,
    project_path: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, global_settings: &str) -> Self {
        let home = TempDir::new().expect("home tempdir");
        let project = TempDir::new().expect("project tempdir");
        let home_path = home.path().to_path_buf();
        let project_path = project.path().to_path_buf();

        let paths = lifecycle_paths(&home_path.join(".tome"));
        fs::create_dir_all(&paths.root).expect("create tome root");

        fs::write(&paths.global_settings_file, global_settings).expect("write global settings");

        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace name");

        let workspace_dir = paths.workspace_dir(&workspace);
        fs::create_dir_all(&workspace_dir).expect("create workspace dir");
        fs::write(
            workspace_dir.join("RULES.md"),
            "# Test rules\n\nHello from the workspace.\n",
        )
        .expect("write workspace RULES.md");

        Fixture {
            _home: home,
            _project: project,
            paths,
            home_path,
            project_path,
            workspace,
        }
    }

    fn bind_deps(&self) -> BindDeps<'_> {
        BindDeps {
            paths: &self.paths,
            home_root: &self.home_path,
        }
    }
}

fn install_two_harnesses() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(ClaudeCode), Box::new(StubHarness::default())])
}

fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

// ---------------------------------------------------------------------------
// 1. Cross-harness: claude-code + stub. Bind + sync writes four files;
//    re-running sync leaves all four mtimes unchanged.
// ---------------------------------------------------------------------------

#[test]
fn cross_harness_resync_is_byte_for_byte_idempotent() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_two_harnesses();

    let fx = Fixture::build("demo", "harnesses = [\"claude-code\", \"stub\"]\n");

    // First pass: bind + sync (the bind step copies the workspace's
    // RULES.md into the project marker, which the sync step's
    // AtInclude branch consumes).
    let outcome = binding::bind_project(
        &fx.project_path,
        fx.workspace.clone(),
        false,
        &fx.bind_deps(),
    )
    .expect("bind_project");
    sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &fx.bind_deps(),
        false,
    )
    .expect("first sync");

    // Phase 6 correction: claude-code's rules sink is CLAUDE.md, not AGENTS.md.
    let claude_md = fx.project_path.join("CLAUDE.md");
    let claude_settings = fx.project_path.join(".claude/settings.json");
    let stub_rules = fx.project_path.join("STUB_RULES.md");
    let stub_mcp = fx.project_path.join("stub.mcp.json");

    for path in [&claude_md, &claude_settings, &stub_rules, &stub_mcp] {
        assert!(
            path.is_file(),
            "expected {} to exist after first sync",
            path.display()
        );
    }

    let agents_mtime_1 = mtime(&claude_md);
    let claude_mtime_1 = mtime(&claude_settings);
    let stub_rules_mtime_1 = mtime(&stub_rules);
    let stub_mcp_mtime_1 = mtime(&stub_mcp);

    // Wait long enough for mtime granularity (HFS+/APFS = 1s; ext4 = ms).
    std::thread::sleep(Duration::from_millis(1500));

    // Second pass: sync only, no bind. Same inputs => no writes.
    let resync_outcome = sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &fx.bind_deps(),
        false,
    )
    .expect("re-sync");

    assert!(
        resync_outcome.added.is_empty(),
        "no additions on idempotent re-sync; got {:?}",
        resync_outcome.added,
    );
    assert!(
        resync_outcome.updated.is_empty(),
        "no updates on idempotent re-sync; got {:?}",
        resync_outcome.updated,
    );
    assert!(
        resync_outcome.removed.is_empty(),
        "no removals on idempotent re-sync; got {:?}",
        resync_outcome.removed,
    );

    assert_eq!(
        mtime(&claude_md),
        agents_mtime_1,
        "CLAUDE.md mtime advanced on idempotent re-sync",
    );
    assert_eq!(
        mtime(&claude_settings),
        claude_mtime_1,
        ".claude/settings.json mtime advanced on idempotent re-sync",
    );
    assert_eq!(
        mtime(&stub_rules),
        stub_rules_mtime_1,
        "STUB_RULES.md mtime advanced on idempotent re-sync",
    );
    assert_eq!(
        mtime(&stub_mcp),
        stub_mcp_mtime_1,
        "stub.mcp.json mtime advanced on idempotent re-sync",
    );
}

// ---------------------------------------------------------------------------
// 2. Empty effective list: nothing was written initially, second sync
//    still touches nothing.
// ---------------------------------------------------------------------------

#[test]
fn cross_harness_empty_effective_list_resync_is_noop() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_two_harnesses();

    // Empty `harnesses = []` → effective list is empty → cleanup runs
    // for every registered harness, but there's nothing to clean up
    // because no prior bind happened.
    let fx = Fixture::build("demo", "harnesses = []\n");

    let outcome = binding::bind_project(
        &fx.project_path,
        fx.workspace.clone(),
        false,
        &fx.bind_deps(),
    )
    .expect("bind_project");
    sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &fx.bind_deps(),
        false,
    )
    .expect("first sync");

    // None of the four files exist after sync.
    let candidates = [
        fx.project_path.join("AGENTS.md"),
        fx.project_path.join(".claude/settings.json"),
        fx.project_path.join("STUB_RULES.md"),
        fx.project_path.join("stub.mcp.json"),
    ];
    for path in &candidates {
        assert!(
            !path.exists(),
            "empty effective list must not create {}",
            path.display()
        );
    }

    // Re-sync: still nothing.
    let resync = sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &fx.bind_deps(),
        false,
    )
    .expect("re-sync");
    assert!(resync.added.is_empty());
    assert!(resync.updated.is_empty());
    assert!(resync.removed.is_empty());

    for path in &candidates {
        assert!(
            !path.exists(),
            "empty effective list must not create {} on re-sync either",
            path.display()
        );
    }
}
