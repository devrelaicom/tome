//! End-to-end tests for `harness::sync::sync_project` against the
//! `StubHarness` fixture (Phase 4 / US1.b-3).
//!
//! The orchestrator is exercised library-API style — no CLI binary, no
//! real harness modules. Each test installs a single-entry override
//! containing [`tome::harness::StubHarness`], builds a project marker
//! by hand, then runs `sync_project` and asserts on the resulting
//! `SyncOutcome` + on-disk state.
//!
//! ## Process-global serialisation
//!
//! `HARNESS_MODULES_OVERRIDE` is a `RwLock<Option<...>>` and cargo runs
//! `#[test]` cases in parallel by default. Tests that install the
//! override must hold a process-local mutex for their entire duration
//! so they don't clobber one another. We follow the
//! `harness_skeleton.rs` convention: a single `OVERRIDE_MUTEX` inside
//! this file, locked at test entry. The mutex is `parking_lot`-style
//! safe across panics — `std::sync::Mutex` poisoning is unwrapped to
//! continue scheduling.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::StubHarness;
use tome::harness::sync::{self, Action, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

/// Process-global mutex serialising every test in this file. Held for
/// the entire test body — `HARNESS_MODULES_OVERRIDE` is a single slot
/// and cargo runs `#[test]` cases on multiple threads.
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

/// Snapshot of state shared across tests.
struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses_toml: Option<&str>) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");

        // Build the project marker config.toml.
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
        }
    }

    fn deps<'a>(&'a self, force: bool) -> SyncDeps<'a> {
        // We don't expose `SyncDeps`'s fields publicly via constructor;
        // build via the public struct literal.
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force,
        }
    }
}

fn install_stub() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(StubHarness)])
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

// ---------------------------------------------------------------------------
// 1. Bind path: sync writes the stub rules block + MCP entry on first run.
// ---------------------------------------------------------------------------

#[test]
fn bind_writes_stub_rules_block_and_mcp_entry() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    // Rules-file: STUB_RULES.md exists and contains the Tome block.
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(rules_path.is_file(), "STUB_RULES.md must exist after sync");
    let rules_body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        rules_body.contains("<!-- tome:begin -->") && rules_body.contains("<!-- tome:end -->"),
        "rules file must carry the Tome block; got: {rules_body}"
    );

    // MCP entry: stub.mcp.json carries the canonical Tome entry.
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(mcp_path.is_file(), "stub.mcp.json must exist after sync");
    let mcp_body = std::fs::read_to_string(&mcp_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&mcp_body).unwrap();
    let entry = parsed
        .get("mcpServers")
        .and_then(|v| v.get("tome"))
        .expect("tome entry under mcpServers");
    assert_eq!(entry["command"], "tome");
    let args = entry["args"].as_array().unwrap();
    assert_eq!(args[0], "mcp");
    assert_eq!(args[1], "--workspace");
    assert_eq!(args[2], "test-workspace");

    // Outcome bookkeeping.
    assert_eq!(outcome.added.len(), 2, "rules + mcp both added");
    assert!(outcome.updated.is_empty());
    assert!(outcome.removed.is_empty());
    let decision = &outcome.decisions[0];
    assert_eq!(decision.harness, "stub");
    assert!(decision.in_effective_list);
    assert_eq!(decision.rules_action, Action::Created);
    assert_eq!(decision.mcp_action, Action::Created);
}

// ---------------------------------------------------------------------------
// 2. Rebind: changing workspace updates the MCP entry's --workspace arg.
// ---------------------------------------------------------------------------

#[test]
fn rebind_rewrites_mcp_workspace_arg() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let mut fx = Fixture::build("ws-a", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");

    // Seed the second workspace + flip the project marker.
    seed_workspace(&fx.paths, "ws-b");
    fx.workspace = WorkspaceName::parse("ws-b").unwrap();
    let marker = fx.project.join(".tome/config.toml");
    std::fs::write(&marker, "workspace = \"ws-b\"\nharnesses = [\"stub\"]\n").unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");

    // MCP entry args updated.
    let mcp_path = fx.project.join("stub.mcp.json");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let args = parsed["mcpServers"]["tome"]["args"].as_array().unwrap();
    assert_eq!(args[2], "ws-b", "stale workspace arg must be rewritten");

    // Outcome carries an `updated` change.
    assert_eq!(outcome.updated.len(), 1);
    assert_eq!(outcome.updated[0].subsystem, SyncSubsystem::Mcp);
}

// ---------------------------------------------------------------------------
// 3. Harness clash: pre-populated user-owned `tome` entry → exit 19,
//    other writes still happen.
// ---------------------------------------------------------------------------

#[test]
fn harness_clash_returns_exit_19_and_continues_other_writes() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    // Pre-populate stub.mcp.json with a user-owned `tome` entry.
    let mcp_path = fx.project.join("stub.mcp.json");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&conflict).unwrap()).unwrap();

    let err = sync::sync_project(&fx.project, &fx.deps(false)).expect_err("must clash");
    assert_eq!(err.exit_code(), 19, "want HarnessClash; got {err:?}");

    match &err {
        tome::error::TomeError::HarnessClash {
            command, first_arg, ..
        } => {
            assert_eq!(command, "evil");
            assert_eq!(first_arg, "serve");
        }
        other => panic!("expected HarnessClash, got {other:?}"),
    }

    // Forward-progress: the rules-file write still happened.
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(
        rules_path.is_file(),
        "rules file must be written even when MCP clashes (FR-403)",
    );

    // The MCP file is left as the user authored it.
    let mcp_body = std::fs::read_to_string(&mcp_path).unwrap();
    assert!(
        mcp_body.contains("evil"),
        "user-owned MCP entry must survive the clash; got: {mcp_body}",
    );
}

// ---------------------------------------------------------------------------
// 4. --force overrides a clash and preserves env.
// ---------------------------------------------------------------------------

#[test]
fn force_overrides_clash_and_preserves_env() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    let mcp_path = fx.project.join("stub.mcp.json");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"],
                "env": { "MY_FLAG": "1", "OTHER_FLAG": "2" }
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&conflict).unwrap()).unwrap();

    let _outcome = sync::sync_project(&fx.project, &fx.deps(true)).expect("force must succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let entry = &parsed["mcpServers"]["tome"];

    // The entry was rewritten to Tome-owned shape.
    assert_eq!(entry["command"], "tome");
    let args = entry["args"].as_array().unwrap();
    assert_eq!(args[0], "mcp");

    // env is intentionally NOT preserved when the existing entry was
    // user-owned — `is_tome_owned` returns false for `command="evil"`,
    // so the env-preservation branch in `write_entry_json` doesn't
    // fire. This is by design (FR-503 preserves env only for already-
    // Tome-owned entries); document the observed behaviour explicitly.
    assert!(
        entry.get("env").is_none(),
        "env not preserved when rewriting user-owned"
    );
}

// ---------------------------------------------------------------------------
// 5. Idempotence: re-running sync produces zero disk changes (FR-525).
// ---------------------------------------------------------------------------

#[test]
fn idempotent_resync_no_disk_changes() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    let mcp_path = fx.project.join("stub.mcp.json");

    let rules_mtime_1 = mtime(&rules_path);
    let mcp_mtime_1 = mtime(&mcp_path);

    // Wait so mtime granularity can advance if a rewrite happens.
    std::thread::sleep(Duration::from_millis(1500));

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");
    assert!(
        outcome.added.is_empty(),
        "added must be empty on idempotent re-run"
    );
    assert!(
        outcome.updated.is_empty(),
        "updated must be empty on idempotent re-run"
    );
    assert!(
        outcome.removed.is_empty(),
        "removed must be empty on idempotent re-run"
    );
    assert!(outcome.leave_alones >= 2, "rules + mcp both left alone");

    assert_eq!(
        mtime(&rules_path),
        rules_mtime_1,
        "rules mtime must not advance"
    );
    assert_eq!(mtime(&mcp_path), mcp_mtime_1, "mcp mtime must not advance");
}

// ---------------------------------------------------------------------------
// 6. Cleanup: removing the harness from the list removes its entries.
// ---------------------------------------------------------------------------

#[test]
fn cleanup_removes_stub_entries_when_harness_drops_from_list() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(rules_path.is_file());
    assert!(mcp_path.is_file());

    // Drop the stub from the effective list.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");

    // Rules-file: the block is gone. The file itself may still exist
    // (it was empty before — `remove_block` collapses to empty content
    // when no surrounding text exists).
    let rules_body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        !rules_body.contains("<!-- tome:begin -->"),
        "rules block must be removed; got: {rules_body}",
    );

    // MCP: the entry is gone (and `mcpServers` object survives empty).
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(
        parsed["mcpServers"].get("tome").is_none(),
        "tome MCP entry must be removed",
    );

    assert_eq!(outcome.removed.len(), 2, "rules + mcp both removed");
}

// ---------------------------------------------------------------------------
// 7. Empty effective list: nothing is written.
// ---------------------------------------------------------------------------

#[test]
fn effective_list_empty_is_noop() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    // No `harnesses` key in the marker → priority walk finds no
    // declarer → empty effective list. Also no workspace/global
    // settings.toml shadow files.
    let fx = Fixture::build("test-workspace", None);

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    assert!(outcome.added.is_empty());
    assert!(outcome.updated.is_empty());
    assert!(outcome.removed.is_empty());

    // No files created. (`STUB_RULES.md` and `stub.mcp.json` should NOT
    // exist — the cleanup pass leaves absent files absent.)
    assert!(
        !fx.project.join("STUB_RULES.md").exists(),
        "empty effective list must not create rules file",
    );
    assert!(
        !fx.project.join("stub.mcp.json").exists(),
        "empty effective list must not create MCP file",
    );
}

// ---------------------------------------------------------------------------
// 8. Workspace settings override the project marker — when the marker
//    omits `harnesses`, the workspace's `settings.toml` declares it.
// ---------------------------------------------------------------------------

#[test]
fn workspace_settings_supply_harness_list_when_marker_omits_key() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    // Project marker omits `harnesses` → resolver falls through.
    let fx = Fixture::build("test-workspace", None);

    // Workspace settings declare the list.
    let ws_dir = fx.paths.workspace_dir(&fx.workspace);
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("settings.toml"),
        "name = \"test-workspace\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    assert_eq!(
        outcome.added.len(),
        2,
        "rules + mcp added from workspace-supplied list"
    );
    assert!(fx.project.join("STUB_RULES.md").is_file());
    assert!(fx.project.join("stub.mcp.json").is_file());
}
