//! End-to-end tests for the Phase 11 / US3 `TsPlugin` session-steering shim
//! through `harness::sync::sync_project` with the REAL harness modules
//! (`OPENCODE`, `CLINE`).
//!
//! The foundation's `reconcile/plugins.rs` unit tests already exercise the
//! reconciler against a `StubHarness` declaring `TsPlugin`. These tests prove
//! the wiring lands end-to-end through `sync_project` once the real modules
//! return `TsPlugin` from `session_steering()`: the embedded `tome.ts` is
//! written to the harness's plugin dir with the EXACT embedded bytes, the
//! harness's `SyncOutcome` decision carries `plugins_action == Created`, a
//! re-sync is idempotent (`LeftAlone`, no mtime advance), and dropping the
//! harness removes ONLY Tome's `tome.ts` (a developer sibling survives).
//!
//! Process-global serialisation: `HARNESS_MODULES_OVERRIDE` is shared, so each
//! test holds `crate::common::HARNESS_OVERRIDE_MUTEX` for its whole duration —
//! the convention every other `harness_settings` sync test follows.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::ShimKind;
use tome::harness::sync::{self, Action, SyncDeps};
use tome::workspace::WorkspaceName;

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
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

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force: false,
            only_harness: None,
            dry_run: false,
        }
    }
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

/// The exact embedded `tome.ts` bytes Tome ships for a `ShimKind`.
fn embedded_shim_bytes(kind: ShimKind) -> &'static [u8] {
    let harness = match kind {
        ShimKind::Cline => "cline",
        ShimKind::Pi => "pi",
        ShimKind::OpenCode => "opencode",
    };
    tome::harness::plugin_assets::find(harness)
        .expect("embedded shim exists")
        .files
        .iter()
        .find(|f| f.rel_path == "tome.ts")
        .expect("tome.ts in shim")
        .bytes
}

// ---------------------------------------------------------------------------
// 1. OpenCode (sync_project integration): a live sync writes
//    `.opencode/plugin/tome.ts` with the EXACT embedded bytes AND the harness
//    decision carries `plugins_action == Created`.
// ---------------------------------------------------------------------------

#[test]
fn opencode_live_sync_writes_shim_and_decision_is_created() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::opencode::OPENCODE)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"opencode\"]");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // The embedded shim landed at the project-relative plugin dir.
    let shim = fx.project.join(".opencode/plugin/tome.ts");
    assert!(
        shim.is_file(),
        "opencode tome.ts must be written on a live sync"
    );
    assert_eq!(
        std::fs::read(&shim).unwrap(),
        embedded_shim_bytes(ShimKind::OpenCode),
        "the installed shim must be byte-identical to the embedded asset",
    );

    // The harness decision carries the new trailing `plugins_action`.
    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "opencode")
        .expect("opencode decision present");
    assert_eq!(
        decision.plugins_action,
        Action::Created,
        "the TsPlugin shim sink must record Created on first install",
    );
}

// ---------------------------------------------------------------------------
// 2. Cline (sync_project integration): a live sync writes
//    `.clinerules`-adjacent `.cline/plugins/tome.ts` byte-identically, and the
//    decision is Created.
// ---------------------------------------------------------------------------

#[test]
fn cline_live_sync_writes_shim_and_decision_is_created() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cline::CLINE)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"cline\"]");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let shim = fx.project.join(".cline/plugins/tome.ts");
    assert!(
        shim.is_file(),
        "cline tome.ts must be written on a live sync"
    );
    assert_eq!(
        std::fs::read(&shim).unwrap(),
        embedded_shim_bytes(ShimKind::Cline),
        "the installed cline shim must be byte-identical to the embedded asset",
    );

    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "cline")
        .expect("cline decision present");
    assert_eq!(decision.plugins_action, Action::Created);
}

// ---------------------------------------------------------------------------
// 2b. Pi (sync_project integration): a live sync writes
//     `.pi/extensions/tome.ts` byte-identically, and the decision is Created.
//     (F1 — the pi real-path coverage that was missing.)
// ---------------------------------------------------------------------------

#[test]
fn pi_live_sync_writes_shim_and_decision_is_created() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::pi::PI)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"pi\"]");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let shim = fx.project.join(".pi/extensions/tome.ts");
    assert!(shim.is_file(), "pi tome.ts must be written on a live sync");
    assert_eq!(
        std::fs::read(&shim).unwrap(),
        embedded_shim_bytes(ShimKind::Pi),
        "the installed pi shim must be byte-identical to the embedded asset",
    );

    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "pi")
        .expect("pi decision present");
    assert_eq!(decision.plugins_action, Action::Created);
}

// ---------------------------------------------------------------------------
// 3. Idempotent re-sync: the shim is left alone (LeftAlone, no mtime advance).
// ---------------------------------------------------------------------------

#[test]
fn resync_is_idempotent_for_the_shim() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::opencode::OPENCODE)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"opencode\"]");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let shim = fx.project.join(".opencode/plugin/tome.ts");
    assert!(shim.is_file());
    let shim_mtime = mtime(&shim);

    std::thread::sleep(Duration::from_millis(1100));
    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "opencode")
        .expect("opencode decision present");
    assert_eq!(
        decision.plugins_action,
        Action::LeftAlone,
        "an idempotent re-sync must leave the shim alone",
    );
    assert_eq!(
        mtime(&shim),
        shim_mtime,
        "the shim mtime must not advance on an idempotent re-sync",
    );
}

// ---------------------------------------------------------------------------
// 4. Removal + mass-delete safeguard: dropping the harness from the effective
//    list removes ONLY Tome's `tome.ts`; a developer sibling in the SAME dir
//    survives.
// ---------------------------------------------------------------------------

#[test]
fn dropping_harness_removes_only_tome_shim_and_keeps_sibling() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::opencode::OPENCODE)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"opencode\"]");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let shim = fx.project.join(".opencode/plugin/tome.ts");
    assert!(shim.is_file());

    // Seed a developer file alongside the shim.
    let sibling = fx.project.join(".opencode/plugin/dev.ts");
    std::fs::write(&sibling, b"// developer's own plugin\n").expect("write sibling");

    // Drop opencode from the effective list.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    assert!(
        !shim.exists(),
        "Tome's tome.ts must be removed when the harness drops"
    );
    assert!(
        sibling.is_file(),
        "a developer's sibling plugin file must NOT be removed (mass-delete safeguard)",
    );
    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "opencode")
        .expect("opencode decision present");
    assert_eq!(
        decision.plugins_action,
        Action::Removed,
        "the shim sink must record Removed when the harness drops",
    );
}

// ---------------------------------------------------------------------------
// 5. Symlink refusal through a real module's sync: a symlinked plugin dir
//    component refuses fail-closed → exit 7 (Io); nothing written through the
//    link. Mirrors the agents-sink symlink integration test.
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn shim_write_through_symlinked_dir_is_refused_exit_7() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::opencode::OPENCODE)]);

    let fx = Fixture::build("test-workspace", "harnesses = [\"opencode\"]");

    // Pre-plant a symlinked intermediate component on the shim's plugin-dir
    // path. The write must refuse to follow it rather than write through.
    let real_target = fx.project.join("real_opencode_dir");
    std::fs::create_dir_all(&real_target).expect("create real dir");
    let oc_dir = fx.project.join(".opencode");
    std::os::unix::fs::symlink(&real_target, &oc_dir).expect("plant symlink");

    let err = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("a symlinked plugin-dir component must be refused");
    assert_eq!(
        err.exit_code(),
        7,
        "the shim sink's symlink refusal maps to Io (exit 7); got {err:?}",
    );
    // Fail-closed: no `tome.ts` was written through the link.
    assert!(
        !real_target.join("plugin/tome.ts").exists(),
        "no shim file may be written through the symlinked component",
    );
}

// ---------------------------------------------------------------------------
// Cline real-path coverage (F2): the removal/idempotence/symlink behaviours
// were previously proven through `sync_project` for OpenCode ONLY. Each
// depends on the per-module `session_steering().dir`, so a second real module
// is exercised here. A tiny `RealShim` descriptor keeps these readable without
// re-stating the fixture/guard boilerplate per case.
// ---------------------------------------------------------------------------

/// A real module's identity for the cross-module shim assertions: how to box a
/// fresh instance, its registry `name`, and the project-relative shim path.
/// (Byte-identity vs the embedded asset is pinned in the per-module live-write
/// tests above; these cross-module cases assert presence/removal/idempotence.)
struct RealShim {
    boxed: fn() -> Box<dyn tome::harness::HarnessModule>,
    name: &'static str,
    shim_rel: &'static str,
}

const CLINE_SHIM: RealShim = RealShim {
    boxed: || Box::new(tome::harness::cline::CLINE),
    name: "cline",
    shim_rel: ".cline/plugins/tome.ts",
};

// 6. Cline removal + sibling survival through `sync_project`.
#[test]
fn cline_dropping_harness_removes_only_tome_shim_and_keeps_sibling() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let s = &CLINE_SHIM;
    let _guard = HarnessModulesGuard::install(vec![(s.boxed)()]);

    let fx = Fixture::build("test-workspace", &format!("harnesses = [\"{}\"]", s.name));

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let shim = fx.project.join(s.shim_rel);
    assert!(shim.is_file(), "{} shim written on live sync", s.name);

    // Seed a developer file alongside the shim.
    let sibling = shim.parent().unwrap().join("dev.ts");
    std::fs::write(&sibling, b"// developer's own plugin\n").expect("write sibling");

    // Drop the harness from the effective list.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    assert!(
        !shim.exists(),
        "Tome's {} tome.ts must be removed when the harness drops",
        s.name,
    );
    assert!(
        sibling.is_file(),
        "a developer's sibling plugin file must NOT be removed (mass-delete safeguard)",
    );
    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == s.name)
        .expect("cline decision present");
    assert_eq!(
        decision.plugins_action,
        Action::Removed,
        "the shim sink must record Removed when the harness drops",
    );
}

// 7. Cline idempotent re-sync (LeftAlone, no mtime advance) through `sync_project`.
#[test]
fn cline_resync_is_idempotent_for_the_shim() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let s = &CLINE_SHIM;
    let _guard = HarnessModulesGuard::install(vec![(s.boxed)()]);

    let fx = Fixture::build("test-workspace", &format!("harnesses = [\"{}\"]", s.name));

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let shim = fx.project.join(s.shim_rel);
    assert!(shim.is_file());
    let shim_mtime = mtime(&shim);

    std::thread::sleep(Duration::from_millis(1100));
    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == s.name)
        .expect("cline decision present");
    assert_eq!(
        decision.plugins_action,
        Action::LeftAlone,
        "an idempotent re-sync must leave the shim alone",
    );
    assert_eq!(
        mtime(&shim),
        shim_mtime,
        "the shim mtime must not advance on an idempotent re-sync",
    );
}

// 8. Cline symlink refusal through `sync_project` (exit 7, nothing written).
#[test]
#[cfg(unix)]
fn cline_shim_write_through_symlinked_dir_is_refused_exit_7() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let s = &CLINE_SHIM;
    let _guard = HarnessModulesGuard::install(vec![(s.boxed)()]);

    let fx = Fixture::build("test-workspace", &format!("harnesses = [\"{}\"]", s.name));

    // Plant a symlinked intermediate component (`.cline`) on the shim path.
    let real_target = fx.project.join("real_cline_dir");
    std::fs::create_dir_all(&real_target).expect("create real dir");
    let cline_dir = fx.project.join(".cline");
    std::os::unix::fs::symlink(&real_target, &cline_dir).expect("plant symlink");

    let err = sync::sync_project(&fx.project, &fx.deps())
        .expect_err("a symlinked plugin-dir component must be refused");
    assert_eq!(
        err.exit_code(),
        7,
        "the shim sink's symlink refusal maps to Io (exit 7); got {err:?}",
    );
    // Fail-closed: no `tome.ts` was written through the link.
    assert!(
        !real_target.join("plugins/tome.ts").exists(),
        "no shim file may be written through the symlinked component",
    );
}
