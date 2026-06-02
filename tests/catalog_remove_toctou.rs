//! Phase 7 / R-9 — `catalog remove --force` cascade TOCTOU (F-REMOVE-TOCTOU).
//!
//! The cascade-disable input MUST be the *current* enabled set, re-derived
//! from the connection opened under the advisory lock — not the
//! stale-tolerant snapshot taken before the lock for the `--force` prompt.
//!
//! THE RACE: a concurrent `plugin enable` that enrols a NEW plugin in the
//! catalog in the window between the pre-lock read and the lock acquisition
//! must still be cascade-disabled by `remove --force`. If the cascade
//! iterates the pre-lock snapshot, the late-enabled plugin's
//! `workspace_skills` enrolment survives after the catalog row + cache are
//! deleted — a GHOST-ENABLED plugin (enabled against a removed catalog).
//!
//! Deterministic, not flaky: rather than gamble on hitting the window with
//! a wall-clock race, we drive the REAL `commands::catalog::remove::run`
//! and inject the concurrent enable exactly in the window via the
//! `AFTER_PRELOCK_READ_HOOK` test seam. The seam fires once, after the
//! pre-lock read and before the lock — the precise TOCTOU window. Before
//! the fix, `run` cascades the stale Vec and leaves the ghost; after the
//! fix it re-derives under the lock and catches it.

mod common;

use std::sync::Mutex;

use common::{
    Fixture, HomeGuard, ToolEnv, fabricate_models, global_scope, has_global_enrolment, paths_for,
};
use tome::cli::CatalogRemoveArgs;
use tome::commands::catalog::remove::{AFTER_PRELOCK_READ_HOOK, run as remove_run};
use tome::index::{self, OpenOptions};
use tome::output::Mode;
use tome::workspace::{ResolvedScope, ScopeSource};

const CATALOG: &str = "sample-plugin-catalog";

/// Serialise the two tests in this file: both install the process-global
/// `AFTER_PRELOCK_READ_HOOK` slot and both mutate `$HOME` via `HomeGuard`
/// (which itself locks `HOME_MUTEX`). Holding this for the whole test body
/// keeps cargo's parallel per-file threads from clobbering the hook slot.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Open the central DB with the REGISTRY seeds the CLI binary stamps at
/// `catalog add`, so subsequent opens never trip the meta drift check.
fn open_registry_seeded(paths: &tome::paths::Paths) -> rusqlite::Connection {
    let pick = |kind| {
        let entry = tome::embedding::registry::MODEL_REGISTRY
            .iter()
            .find(|m| std::mem::discriminant(&m.kind) == std::mem::discriminant(&kind))
            .unwrap();
        tome::index::MetaSeed {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
        }
    };
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: pick(tome::embedding::registry::ModelKind::Embedder),
            reranker: pick(tome::embedding::registry::ModelKind::Reranker),
            summariser: pick(tome::embedding::registry::ModelKind::Summariser),
        },
    )
    .expect("open central DB (registry seeds)")
}

/// Directly enrol one plugin's lone skill in the `global` workspace —
/// the minimal on-disk shape `plugin enable` leaves behind, without
/// loading the real embedder. `enabled_plugins_for_catalog` returns a
/// plugin iff a `skills` row joined to a `workspace_skills` row exists,
/// so this is all the cascade needs to see.
fn enrol_plugin(conn: &rusqlite::Connection, plugin: &str) {
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'skill', 'd', '0.0.0', ?4, ?5, 1, 0, NULL,
                 '1970-01-01T00:00:00Z')",
        rusqlite::params![
            CATALOG,
            plugin,
            format!("{plugin}-skill"),
            format!("skills/{plugin}/SKILL.md"),
            format!("hash-{plugin}"),
        ],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog = ?1 AND plugin = ?2",
            rusqlite::params![CATALOG, plugin],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
            r.get(0)
        })
        .expect("global ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill in global");
}

/// Count `(global, CATALOG)` `workspace_skills` enrolments for `plugin`.
/// A surviving enrolment for a plugin whose catalog row is gone is the
/// ghost-enabled state the invariant forbids.
fn enrolment_count(paths: &tome::paths::Paths, plugin: &str) -> i64 {
    let conn = open_registry_seeded(paths);
    conn.query_row(
        "SELECT COUNT(*) FROM workspace_skills AS ws
         JOIN skills     AS s ON s.id = ws.skill_id
         JOIN workspaces AS w ON w.id = ws.workspace_id
         WHERE w.name = 'global' AND s.catalog = ?1 AND s.plugin = ?2",
        rusqlite::params![CATALOG, plugin],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// RAII guard installing a one-shot `AFTER_PRELOCK_READ_HOOK`. The hook
/// fires its closure the FIRST time only (re-entrancy / repeated-call
/// guard), then no-ops. Slot is cleared on drop, surviving panics.
struct HookGuard;

impl HookGuard {
    fn install<F: Fn() + Send + Sync + 'static>(f: F) -> Self {
        let fired = std::sync::atomic::AtomicBool::new(false);
        let once = move || {
            if !fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
                f();
            }
        };
        *AFTER_PRELOCK_READ_HOOK
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Box::new(once));
        Self
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        *AFTER_PRELOCK_READ_HOOK
            .write()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn global_resolved() -> ResolvedScope {
    ResolvedScope {
        scope: global_scope(),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

/// PRIMARY (race): a concurrent enable lands in the TOCTOU window. The
/// cascade must catch the late-enabled plugin so no ghost survives.
#[test]
fn concurrent_enable_in_window_leaves_no_ghost() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // No models fabricated on purpose: the post-cascade summary-regen
    // trigger short-circuits on `ModelMissing` (a documented no-op),
    // and the cascade path itself loads no model. Keeps the test free
    // of ONNX/llama artefacts.

    // Register the catalog via the CLI binary: bootstraps the central DB
    // with REGISTRY seeds + writes the `(global, CATALOG)` enrolment and
    // the on-disk cache `run` will try to clean up.
    let fix = Fixture::build_from(common::sample_plugin_catalog_fixture());
    let add = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "catalog add failed: {}",
        String::from_utf8_lossy(&add.stderr),
    );
    assert!(has_global_enrolment(&paths, CATALOG));

    // Pre-existing enabled plugin so the pre-lock read is non-empty and
    // the `--force` cascade actually fires.
    {
        let conn = open_registry_seeded(&paths);
        enrol_plugin(&conn, "plugin-alpha");
    }

    // The "concurrent enable": enrol plugin-beta exactly in the TOCTOU
    // window (fired once, after the pre-lock read, before the lock).
    let hook_paths = paths.clone();
    let _hook = HookGuard::install(move || {
        let conn = open_registry_seeded(&hook_paths);
        enrol_plugin(&conn, "plugin-beta");
    });

    // Drive the REAL command. `run` calls `Paths::resolve()`, so point
    // `$HOME` at the isolated env for the duration.
    let _home = HomeGuard::install(env.home_path());
    let outcome = remove_run(
        CatalogRemoveArgs {
            name: CATALOG.to_owned(),
            force: true,
        },
        &global_resolved(),
        Mode::Json,
    );
    assert!(outcome.is_ok(), "remove --force failed: {outcome:?}");

    // The catalog enrolment + its skills are gone (remove succeeded).
    assert!(
        !has_global_enrolment(&paths, CATALOG),
        "catalog enrolment should be removed",
    );

    // INVARIANT: no plugin remains enabled against the removed catalog.
    // plugin-alpha was in the pre-lock snapshot (always cascaded).
    assert_eq!(
        enrolment_count(&paths, "plugin-alpha"),
        0,
        "pre-lock-snapshot plugin must be cascade-disabled",
    );
    // plugin-beta enrolled in the window: the pre-fix cascade iterates
    // the stale Vec and MISSES it, leaving a ghost. Post-fix re-derives
    // under the lock and catches it.
    assert_eq!(
        enrolment_count(&paths, "plugin-beta"),
        0,
        "GHOST: plugin enabled in the TOCTOU window survived as enabled against a removed catalog \
         — the cascade used the stale pre-lock snapshot instead of re-deriving under the lock",
    );
}

/// REGRESSION (single-process): a normal `remove --force` with no racing
/// enable cascade-disables every currently-enabled plugin and leaves no
/// ghost. Guards the common path the fix must not perturb.
#[test]
fn single_process_force_cascade_leaves_no_ghost() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let fix = Fixture::build_from(common::sample_plugin_catalog_fixture());
    let add = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "catalog add failed: {}",
        String::from_utf8_lossy(&add.stderr),
    );

    {
        let conn = open_registry_seeded(&paths);
        enrol_plugin(&conn, "plugin-alpha");
        enrol_plugin(&conn, "plugin-beta");
    }

    // No hook installed — the pure single-process path.
    let _home = HomeGuard::install(env.home_path());
    let outcome = remove_run(
        CatalogRemoveArgs {
            name: CATALOG.to_owned(),
            force: true,
        },
        &global_resolved(),
        Mode::Json,
    );
    assert!(outcome.is_ok(), "remove --force failed: {outcome:?}");

    assert!(!has_global_enrolment(&paths, CATALOG));
    assert_eq!(
        enrolment_count(&paths, "plugin-alpha"),
        0,
        "alpha must be cascade-disabled",
    );
    assert_eq!(
        enrolment_count(&paths, "plugin-beta"),
        0,
        "beta must be cascade-disabled",
    );
}
