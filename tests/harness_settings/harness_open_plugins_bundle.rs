//! End-to-end tests for the Phase 11 / US4 Open Plugins (`tome-op`) bundle +
//! the `generic` opt-in target through `harness::sync::sync_project` with the
//! REAL harness registry (`goose` in `SUPPORTED_HARNESSES`; `generic` /
//! `generic-op` in `OPT_IN_TARGETS`).
//!
//! These close the US4 closeout BLOCKER (B1): an `OPT_IN_TARGETS` module whose
//! `name()` is explicitly in the effective list MUST be snapshotted (and thus
//! dispatched) by `sync_project`. The `reconcile/open_plugins.rs` unit tests
//! only exercise the reconciler directly; these prove the orchestrator's
//! snapshot-union + partition route generic-op/generic/goose end-to-end:
//!
//! - generic-op / goose → the Open Plugins partition → the atomic `tome-op`
//!   bundle (4 files, `SyncSubsystem::OpenPlugins` change, `plugins_action ==
//!   Created`, idempotent re-sync, drop-removes + sibling survives, real
//!   `--harness`/`--workspace` stamped into `.mcp.json` + `hooks/hooks.json`).
//! - generic → the STANDARD sinks → one `tome:begin` AGENTS.md region + a
//!   pinned `mcp.json`; drop cleans both.
//! - a NON-op harness → NO `SyncSubsystem::OpenPlugins` entry (partition no-op).
//!
//! Driven against the REAL registry (NO `HarnessModulesGuard` override) because
//! the opt-in targets live in `OPT_IN_TARGETS`, not the override slot. The
//! effective list is driven by the project marker's `harnesses = [...]`
//! declaration (NOT detection — the fresh `$HOME` has no harness dirs), so the
//! explicit-selection-only invariant is exercised faithfully. Each test still
//! serialises on `HARNESS_OVERRIDE_MUTEX` so a co-resident override-installing
//! test cannot leak its boxed registry in and shadow the real one mid-run.

use std::path::PathBuf;
use std::time::Duration;

use crate::common::{ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::HarnessModule;
use tome::harness::sync::{self, Action, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

struct Fixture {
    home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    /// Build a bound project whose marker declares exactly `harnesses_toml`.
    fn build(workspace_name: &str, harnesses_toml: &str) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\n{harnesses_toml}\n"),
        )
        .expect("write marker config");
        // Seed the inline rules source the bundle's AGENTS.md / generic AGENTS.md
        // mirror — so the directive body is non-empty + assertable.
        std::fs::write(marker_dir.join("RULES.md"), "# rules body\n").expect("write RULES.md");

        Fixture {
            home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self.home.path(),
            workspace_name: &self.workspace,
            force: false,
            only_harness: None,
            dry_run: false,
        }
    }

    /// Rewrite the marker to a new harnesses declaration (e.g. to drop the
    /// harness for the removal-path assertions).
    fn set_harnesses(&self, harnesses_toml: &str) {
        std::fs::write(
            self.project.join(".tome/config.toml"),
            format!(
                "workspace = \"{}\"\n{harnesses_toml}\n",
                self.workspace.as_str()
            ),
        )
        .expect("rewrite marker config");
    }
}

fn mutex_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The on-disk bundle root for an op harness, relative to the project.
fn bundle_root(project: &std::path::Path, harness: &str) -> PathBuf {
    match harness {
        "goose" => project.join(".config/goose/plugins/tome-op"),
        "generic-op" => project.join("tome-op"),
        other => panic!("not an op harness: {other}"),
    }
}

// ---------------------------------------------------------------------------
// The Open Plugins bundle harnesses (generic-op + goose) — F3 / M2 / M3.
//
// Both are driven through the SAME assertion body keyed on the harness id, so a
// regression that breaks one but not the other is caught.
// ---------------------------------------------------------------------------

fn op_harness_full_lifecycle(harness: &str) {
    let _lock = mutex_lock();
    let fx = Fixture::build("test-workspace", &format!("harnesses = [\"{harness}\"]"));
    let root = bundle_root(&fx.project, harness);

    // ---- (a) live sync lands all 4 bundle files at the right root ----------
    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("live sync");

    assert!(
        root.join(".plugin/plugin.json").is_file(),
        "{harness}: .plugin/plugin.json must land at {}",
        root.display(),
    );
    assert!(root.join("hooks/hooks.json").is_file(), "{harness}: hooks");
    assert!(root.join(".mcp.json").is_file(), "{harness}: .mcp.json");
    assert!(root.join("AGENTS.md").is_file(), "{harness}: AGENTS.md");

    // ---- (b) `outcome.added` carries an OpenPlugins change -----------------
    assert!(
        outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::OpenPlugins && c.harness == harness),
        "{harness}: outcome.added must carry an OpenPlugins change; got {:?}",
        outcome.added,
    );

    // ---- (c) the op-harness decision is `plugins_action == Created` --------
    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == harness)
        .unwrap_or_else(|| panic!("{harness}: decision present"));
    assert_eq!(
        decision.plugins_action,
        Action::Created,
        "{harness}: first bundle emit records Created",
    );
    assert!(
        decision.in_effective_list,
        "{harness}: live harness must be in_effective_list",
    );

    // ---- (f) the real harness id + workspace are stamped into the commands -
    let mcp = std::fs::read_to_string(root.join(".mcp.json")).unwrap();
    assert!(
        mcp.contains("--harness") && mcp.contains(harness),
        "{harness}: .mcp.json must stamp --harness {harness}; got {mcp}",
    );
    assert!(
        mcp.contains("--workspace") && mcp.contains("test-workspace"),
        "{harness}: .mcp.json must stamp the real --workspace; got {mcp}",
    );
    let hooks = std::fs::read_to_string(root.join("hooks/hooks.json")).unwrap();
    assert!(
        hooks.contains(&format!("--harness {harness}")),
        "{harness}: hooks.json must stamp --harness {harness} (real harness id, NOT the workspace); got {hooks}",
    );
    assert!(
        hooks.contains("--workspace test-workspace"),
        "{harness}: hooks.json must stamp the real --workspace; got {hooks}",
    );

    // ---- (d) re-sync is idempotent (Updated or LeftAlone) ------------------
    std::thread::sleep(Duration::from_millis(1100));
    let outcome2 = sync::sync_project(&fx.project, &fx.deps()).expect("re-sync");
    let decision2 = outcome2
        .decisions
        .iter()
        .find(|d| d.harness == harness)
        .unwrap();
    assert!(
        matches!(
            decision2.plugins_action,
            Action::Updated | Action::LeftAlone
        ),
        "{harness}: re-sync must be idempotent (Updated|LeftAlone); got {:?}",
        decision2.plugins_action,
    );

    // ---- (e) dropping it removes the bundle; a seeded sibling survives -----
    // Seed a developer's sibling dir in the SAME plugins parent.
    let sibling = root.parent().expect("bundle parent").join("their-plugin");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("keep.txt"), b"mine").unwrap();

    fx.set_harnesses("harnesses = []");
    let outcome3 = sync::sync_project(&fx.project, &fx.deps()).expect("drop sync");

    assert!(!root.exists(), "{harness}: bundle removed on drop");
    assert!(
        sibling.join("keep.txt").is_file(),
        "{harness}: a developer sibling dir must survive the drop",
    );
    let decision3 = outcome3
        .decisions
        .iter()
        .find(|d| d.harness == harness)
        .unwrap();
    assert_eq!(
        decision3.plugins_action,
        Action::Removed,
        "{harness}: drop records Removed",
    );
    assert!(
        outcome3
            .removed
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::OpenPlugins && c.harness == harness),
        "{harness}: outcome.removed must carry an OpenPlugins change",
    );
}

#[test]
fn generic_op_bundle_full_lifecycle_through_sync_project() {
    op_harness_full_lifecycle("generic-op");
}

#[test]
fn goose_bundle_full_lifecycle_through_sync_project() {
    op_harness_full_lifecycle("goose");
}

// ---------------------------------------------------------------------------
// The `generic` opt-in target — standard sinks (M4 / F3).
//
// `generic` writes `<project>/AGENTS.md` (one Inline tome:begin region) and
// `<project>/mcp.json` (pinned bytes) through the NORMAL rules/MCP loop — NOT
// the open-plugins partition. Drop cleans both.
// ---------------------------------------------------------------------------

#[test]
fn generic_target_writes_agents_and_mcp_through_standard_sinks() {
    let _lock = mutex_lock();
    let fx = Fixture::build("test-workspace", "harnesses = [\"generic\"]");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("live sync");

    // AGENTS.md carries exactly ONE Inline `tome:begin` region with the directive.
    let agents_path = fx.project.join("AGENTS.md");
    let agents = std::fs::read_to_string(&agents_path).expect("AGENTS.md written");
    assert_eq!(
        agents.matches("<!-- tome:begin -->").count(),
        1,
        "exactly one tome:begin region; got {agents}",
    );
    assert!(
        agents.contains("# rules body"),
        "the Inline region carries the verbatim directive; got {agents}",
    );

    // `<project>/mcp.json`: mcpServers + CommandArgs + env:{} + the real
    // --workspace/--harness args. #337: the `command` is now the resolved
    // absolute launcher (machine-specific `current_exe` / `$TOME_BIN`), so the
    // structure is asserted field-by-field rather than as a byte-pin — the
    // command is checked to be a RECOGNISED Tome launcher (the #337 fix on the
    // `generic` target the issue specifically calls out).
    let mcp_path = fx.project.join("mcp.json");
    let mcp = std::fs::read_to_string(&mcp_path).expect("mcp.json written");
    let parsed: serde_json::Value = serde_json::from_str(&mcp).expect("mcp.json parses");
    let entry = &parsed["mcpServers"]["tome"];
    assert!(
        tome::harness::launcher::looks_like_tome_launcher(entry["command"].as_str().unwrap()),
        "generic mcp.json command must be a recognised Tome launcher; got {}",
        entry["command"],
    );
    assert_eq!(
        entry["args"],
        serde_json::json!([
            "mcp",
            "--workspace",
            "test-workspace",
            "--harness",
            "generic"
        ]),
        "generic mcp.json args pinned",
    );
    assert_eq!(
        entry["env"],
        serde_json::json!({}),
        "generic mcp.json env:{{}}"
    );
    // The Tome-owned entry round-trips through the ownership predicate.
    let read = tome::harness::mcp_config::read_entry(
        &mcp_path,
        &tome::harness::generic::GENERIC.mcp_dialect(),
    )
    .unwrap()
    .unwrap();
    assert!(
        tome::harness::mcp_config::is_tome_owned(&read),
        "generic mcp.json entry must be recognised as Tome-owned",
    );

    // The change is recorded under the standard Rules + Mcp subsystems, NOT
    // OpenPlugins (generic goes through the per-sink loop). The shared
    // `<project>/AGENTS.md` rules write is attributed to whichever sharer first
    // touches the path (the dedup names a concrete harness; `generic` is appended
    // LAST so a co-owner like `codex` records it), so assert the Rules write
    // landed at AGENTS.md by SOME harness, and `generic` exclusively records its
    // OWN `mcp.json` write.
    assert!(
        outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Rules && c.path == agents_path),
        "the shared AGENTS.md rules write must be recorded; got {:?}",
        outcome.added,
    );
    assert!(
        outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Mcp && c.harness == "generic"),
        "generic mcp.json write recorded under generic; got {:?}",
        outcome.added,
    );
    assert!(
        !outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::OpenPlugins),
        "generic must NOT produce an OpenPlugins change (standard sinks only)",
    );

    // ---- drop cleans both files -------------------------------------------
    fx.set_harnesses("harnesses = []");
    sync::sync_project(&fx.project, &fx.deps()).expect("drop sync");

    // The standalone-less AGENTS.md is a BlockInExistingFile, so the region is
    // removed (the file may remain if it had other content; here it had only the
    // Tome block, so the block is stripped).
    let agents_after = std::fs::read_to_string(&agents_path).unwrap_or_default();
    assert!(
        !agents_after.contains("<!-- tome:begin -->"),
        "the tome region must be removed on drop; got {agents_after}",
    );
    // The MCP entry is Tome-owned → removed on drop. `mcp.json` had only the
    // tome server, so the entry is gone.
    let mcp_after = std::fs::read_to_string(&mcp_path).unwrap_or_default();
    assert!(
        !mcp_after.contains("\"tome\""),
        "the tome MCP entry must be removed on drop; got {mcp_after}",
    );
}

// ---------------------------------------------------------------------------
// M4 — partition no-op byte-identity: a NON-op harness produces NO
// `SyncSubsystem::OpenPlugins` entry, and the decision list contains exactly
// that harness (no spurious op-harness decisions from the empty partition).
// ---------------------------------------------------------------------------

#[test]
fn non_op_harness_produces_no_open_plugins_entry() {
    let _lock = mutex_lock();
    // `codex` is a plain Phase ≤10 harness (no open_plugins_root).
    let fx = Fixture::build("test-workspace", "harnesses = [\"codex\"]");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    assert!(
        outcome
            .added
            .iter()
            .chain(outcome.removed.iter())
            .chain(outcome.updated.iter())
            .all(|c| c.subsystem != SyncSubsystem::OpenPlugins),
        "a non-op harness must never produce an OpenPlugins change; got added={:?}",
        outcome.added,
    );
    // codex decision present + live. `goose` (in SUPPORTED_HARNESSES) is always
    // snapshotted, so it APPEARS as a decision — but it must be NON-live (it was
    // not selected and the fresh $HOME has no `~/.config/goose`), and its
    // open-plugins partition produced NO change (asserted above). The opt-in
    // `generic-op` (in OPT_IN_TARGETS, no artifact present) must NOT appear at
    // all — the snapshot-union's explicit-selection-only gate excludes it.
    let codex = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "codex")
        .expect("codex decision present");
    assert!(codex.in_effective_list, "codex must be live");

    if let Some(goose) = outcome.decisions.iter().find(|d| d.harness == "goose") {
        assert!(
            !goose.in_effective_list,
            "goose must be non-live when not selected/detected",
        );
        assert_eq!(
            goose.plugins_action,
            Action::LeftAlone,
            "goose's open-plugins pass must be a no-op when not selected",
        );
    }
    assert!(
        !outcome.decisions.iter().any(|d| d.harness == "generic-op"),
        "an unselected opt-in target must not appear in the decision list; got {:?}",
        outcome
            .decisions
            .iter()
            .map(|d| d.harness.as_str())
            .collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// Explicit-selection-only invariant (B1): with NO opt-in target in the
// effective list, the bundle is NEVER written even though `generic-op`/`goose`
// modules exist. (goose IS detectable, but the fresh $HOME has no
// `~/.config/goose`, so it is not in the effective list and must not emit.)
// ---------------------------------------------------------------------------

#[test]
fn op_bundle_not_written_when_not_selected() {
    let _lock = mutex_lock();
    let fx = Fixture::build("test-workspace", "harnesses = [\"codex\"]");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    assert!(
        !bundle_root(&fx.project, "generic-op").exists(),
        "generic-op bundle must not be written when not selected",
    );
    assert!(
        !bundle_root(&fx.project, "goose").exists(),
        "goose bundle must not be written when not selected/detected",
    );
}

/// PW4 (phase-wide): `teardown_project` (the empty-effective-set teardown that
/// `tome workspace remove` Step 1 now routes through) unwinds EVERY new sink —
/// the `TsPlugin` shim (cline), the `CommandHook` entry (devin), and the
/// `tome-op` bundle (goose) — not just rules + MCP. A developer-owned sibling
/// alongside Tome's artifacts survives.
#[test]
fn teardown_project_unwinds_ts_shim_command_hook_and_tome_op_bundle() {
    let _lock = mutex_lock();
    // Three new-harness shapes in one project: cline (TsPlugin shim), devin
    // (CommandHook), goose (Open Plugins tome-op bundle).
    let fx = Fixture::build(
        "test-workspace",
        "harnesses = [\"cline\", \"devin\", \"goose\"]",
    );

    // ---- (a) live sync lands every harness's artifact ----------------------
    sync::sync_project(&fx.project, &fx.deps()).expect("live sync");

    let shim = fx.project.join(".cline/plugins/tome.ts");
    let command_hook = fx.project.join(".devin/hooks.v1.json");
    let bundle = bundle_root(&fx.project, "goose");
    assert!(shim.is_file(), "cline TsPlugin shim must land");
    assert!(command_hook.is_file(), "devin CommandHook entry must land");
    assert!(
        bundle.join(".plugin/plugin.json").is_file(),
        "goose tome-op bundle must land",
    );

    // A developer-owned sibling next to the Tome-owned shim must survive.
    let dev_sibling = fx.project.join(".cline/plugins/their-plugin.ts");
    std::fs::write(&dev_sibling, "// developer's own plugin\n").unwrap();

    // ---- (b) teardown removes EVERY Tome-owned artifact --------------------
    sync::teardown_project(&fx.project, &fx.deps()).expect("teardown");

    assert!(
        !shim.exists(),
        "teardown must remove the cline TsPlugin shim"
    );
    assert!(
        !command_hook.exists() || {
            // The hook file may persist if it carried developer entries; Tome's
            // own entry must be gone. Here Tome wrote the file, so it is removed.
            let body = std::fs::read_to_string(&command_hook).unwrap_or_default();
            !body.contains("tome harness session-start")
        },
        "teardown must remove Tome's devin CommandHook entry",
    );
    assert!(
        !bundle.exists(),
        "teardown must remove the goose tome-op bundle",
    );

    // ---- (c) the developer sibling is untouched ----------------------------
    assert!(
        dev_sibling.is_file(),
        "a developer-owned sibling must survive teardown",
    );
}
