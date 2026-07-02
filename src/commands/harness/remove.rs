//! `tome harness remove [<name>...] [--all] [--scope project|workspace|global]`
//!
//! Mirror of `harness use`. Drops each selected harness from the chosen
//! scope's `harnesses` array (global scope: `config.toml [harness].enabled`)
//! and runs the cleanup pass when the effective list changes (the harness's
//! integration is torn down from the current project).
//!
//! Selection (issue #315, mirroring `harness use`):
//! - **names given** → exactly those. A name need NOT be a supported harness —
//!   the user may have a stale or typo'd entry in their settings file; a
//!   missing name simply no-ops for that entry.
//! - **`--all`** → every harness CONFIGURED in the resolved scope (i.e. the
//!   names currently in that scope's list), cleared in one pass. When the
//!   scope's list is empty, `--all` is a whole no-op.
//! - **no names + no `--all`** → a usage error (exit 2). Unlike `use`, there is
//!   no "all detected" default for a destructive op; name a harness or `--all`.
//!
//! `--all` + names is a clap conflict.
//!
//! Forward-progress (`first_error`): each selected harness is removed in turn;
//! a per-harness failure is recorded and the loop CONTINUES, surfacing the
//! first error's exit code at the end so partial progress still lands.
//!
//! ## Concurrency (C-M5 / R-M2 / S-M2 from US3 review)
//!
//! The advisory lock at `paths.index_lock` is held across the whole
//! read-modify-write window — now spanning the WHOLE multi-harness loop, so
//! the entire selection is one atomic settings transaction. See
//! `harness::use_` for the rationale.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{HarnessRemoveArgs, HarnessScopeArg};
use crate::error::TomeError;
use crate::harness::sync;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::edit::{
    configured_harnesses, configured_harnesses_from_config, open_settings, remove_harness,
    remove_harness_from_config, save_settings,
};
use crate::workspace::ResolvedScope;

use super::use_::{compute_effective_names, resolve_settings_path};
use super::{effective_harness_scope, home_root};

/// One harness's `tome harness remove` outcome.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessRemoveOutcome {
    pub scope: String,
    pub name: String,
    pub settings_path: PathBuf,
    pub list_changed: bool,
    pub sync_ran: bool,
}

/// One selected harness's result: either a successful [`HarnessRemoveOutcome`]
/// or a failure recorded by the forward-progress loop. Serialised in the
/// `--json` envelope so a partial-failure run is fully machine-readable.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HarnessRemoveResult {
    /// The harness was removed successfully (cleanup ran, or no-op).
    Ok(HarnessRemoveOutcome),
    /// The harness failed; `name` is the harness, `error` the rendered message,
    /// `exit_code` the closed-set code the run will surface (the FIRST failure's
    /// code is the process exit).
    Failed {
        name: String,
        error: String,
        exit_code: i32,
    },
}

/// The full `tome harness remove` report over the selected harness set.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessRemoveReport {
    /// How the selection was derived: `"explicit"` (names given) or `"all"`
    /// (`--all` cleared the scope).
    pub selection: &'static str,
    /// Per-harness results, in selection order.
    pub results: Vec<HarnessRemoveResult>,
}

pub fn run(
    args: HarnessRemoveArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let RunInner {
        report,
        first_error,
    } = run_inner(args, scope, paths)?;

    // Telemetry parity with the former single-name path: emit one
    // `tome.harness_action{Remove}` per harness removed successfully. Best-effort,
    // success-path only; failed harnesses emit nothing.
    for result in &report.results {
        if let HarnessRemoveResult::Ok(outcome) = result {
            super::emit_harness_action(
                &outcome.name,
                crate::telemetry::event::HarnessAction::Remove,
            );
        }
    }

    match mode {
        Mode::Human => emit_human(&report)?,
        Mode::Json => write_json(&report)?,
    }

    // Forward-progress: emit the full report FIRST (so partial progress is
    // visible), then surface the first failure's exit code.
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The full outcome of [`run_inner`]: the emitted [`HarnessRemoveReport`] and the
/// first forward-progress failure (which sets the process exit code).
#[doc(hidden)]
pub struct RunInner {
    pub report: HarnessRemoveReport,
    pub first_error: Option<TomeError>,
}

/// Compute the full [`HarnessRemoveReport`] — selection resolution + the
/// per-harness read-modify-write + cleanup pipeline, MINUS the terminal emit
/// (the "silent compute / emit wrapper" split). `run` wraps this and emits per
/// mode; integration tests call this directly to assert the EMITTED report.
///
/// Returns the report PLUS the first error captured by the forward-progress
/// loop. The `selection` resolution itself can fail loud (`--scope project`
/// with no project root → 2, or neither names nor `--all` → 2); those propagate
/// via the outer `Result` (before any harness is touched).
#[doc(hidden)]
pub fn run_inner(
    args: HarnessRemoveArgs,
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<RunInner, TomeError> {
    // Resolve effective scope: explicit flag → [harness] default_scope → project.
    let eff_scope = effective_harness_scope(args.scope, paths)?;
    let settings_path = resolve_settings_path(&eff_scope, scope, paths)?;

    // Resolve the selection AFTER the settings path (so `--all` can read the
    // scope's current list). An empty selection without `--all` fails loud (2).
    let (selection, names) = resolve_selection(&args, &settings_path, paths)?;

    // Lock for the entire read-modify-write + cleanup window so the whole
    // selection is one atomic settings transaction.
    std::fs::create_dir_all(&paths.root)?;
    let _lock = crate::index::acquire_lock(&paths.index_lock)?;

    let mut results: Vec<HarnessRemoveResult> = Vec::with_capacity(names.len());
    let mut first_error: Option<TomeError> = None;

    for name in names {
        match remove_one(&name, eff_scope, scope, paths, &settings_path) {
            Ok(outcome) => results.push(HarnessRemoveResult::Ok(outcome)),
            Err(e) => {
                tracing::warn!(
                    harness = name.as_str(),
                    error = %e,
                    "harness remove: harness failed; continuing",
                );
                results.push(HarnessRemoveResult::Failed {
                    name: name.clone(),
                    error: e.to_string(),
                    exit_code: e.exit_code(),
                });
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    Ok(RunInner {
        report: HarnessRemoveReport { selection, results },
        first_error,
    })
}

/// Resolve the variadic / `--all` selection into an ordered, deduped list of
/// harness names plus the selection-mode label.
///
/// - explicit names → order-preserving dedupe of exactly those (no validation —
///   a stale/typo'd name is still a valid removal target).
/// - `--all` → every name currently CONFIGURED in the resolved scope's list.
/// - neither → a usage error (exit 2): there is no "all detected" default for a
///   destructive op.
fn resolve_selection(
    args: &HarnessRemoveArgs,
    settings_path: &std::path::Path,
    paths: &Paths,
) -> Result<(&'static str, Vec<String>), TomeError> {
    if !args.names.is_empty() {
        let mut seen: Vec<String> = Vec::with_capacity(args.names.len());
        for raw in &args.names {
            if !seen.contains(raw) {
                seen.push(raw.clone());
            }
        }
        Ok(("explicit", seen))
    } else if args.all {
        // Read the names configured in THIS scope's list (not the effective
        // walk): `--all` clears the scope's own declarations.
        //
        // This enumeration read is intentionally BEFORE the advisory lock (the
        // caller acquires it after `resolve_selection`). Safe: each `remove_one`
        // RE-OPENS the settings doc and `retain`s under the lock, so a name that
        // vanished between this read and the write is simply a per-harness no-op
        // (`list_changed:false`) — never a data-loss window.
        Ok(("all", configured_names_in_scope(settings_path, paths)))
    } else {
        Err(TomeError::Usage(
            "name at least one harness to remove, or pass --all to clear the scope".into(),
        ))
    }
}

/// The harness names literally declared in the resolved scope's settings file
/// (`config.toml [harness].enabled` for global; the `harnesses = [...]` key
/// otherwise). A missing/empty file yields an empty vec, so `--all` no-ops.
fn configured_names_in_scope(settings_path: &std::path::Path, paths: &Paths) -> Vec<String> {
    let Ok(doc) = open_settings(settings_path) else {
        return Vec::new();
    };
    if settings_path == paths.global_config_file.as_path() {
        configured_harnesses_from_config(&doc)
    } else {
        configured_harnesses(&doc)
    }
}

/// Remove ONE harness: the read-modify-write + cleanup pipeline for a single
/// `name`. The advisory lock is already held by the caller (`run_inner`), and
/// the settings path is resolved once.
fn remove_one(
    name: &str,
    eff_scope: HarnessScopeArg,
    scope: &ResolvedScope,
    paths: &Paths,
    settings_path: &std::path::Path,
) -> Result<HarnessRemoveOutcome, TomeError> {
    let pre = match scope.project_root.as_deref() {
        Some(_) => Some(compute_effective_names(scope, paths)?),
        None => None,
    };

    // Global scope reads/writes `config.toml [harness].enabled`; other scopes
    // use the legacy `harnesses = [...]` key.
    let mut doc = open_settings(settings_path)?;
    let changed = if settings_path == paths.global_config_file.as_path() {
        remove_harness_from_config(&mut doc, name)
    } else {
        remove_harness(&mut doc, name)
    };
    if changed {
        save_settings(settings_path, &doc)?;
    }

    let mut sync_ran = false;
    if let Some(pre_names) = pre {
        let post_names = compute_effective_names(scope, paths)?;
        if pre_names != post_names {
            let project_root = scope
                .project_root
                .as_deref()
                .expect("project_root present when pre was Some");
            let home = home_root()?;
            // Cleanup never needs --force; the orchestrator only refuses on
            // user-owned MCP entries during a write, and cleanup of a Tome-owned
            // entry is unconditional.
            let deps = sync::build_deps(paths, &home, scope.scope.name(), false);
            sync::sync_project(project_root, &deps)?;
            sync_ran = true;
        }
    }

    Ok(HarnessRemoveOutcome {
        scope: eff_scope.to_string(),
        name: name.to_string(),
        settings_path: settings_path.to_path_buf(),
        list_changed: changed,
        sync_ran,
    })
}

fn emit_human(report: &HarnessRemoveReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();

    if report.results.is_empty() {
        // Only reachable via `--all` on an empty scope (the neither-case errors
        // out in resolve_selection).
        writeln!(out, "No harnesses configured in scope. Nothing to remove.")?;
        return Ok(());
    }

    for result in &report.results {
        match result {
            HarnessRemoveResult::Ok(outcome) => emit_one_ok(&mut out, outcome)?,
            HarnessRemoveResult::Failed {
                name,
                error,
                exit_code,
            } => writeln!(out, "{name}: FAILED (exit {exit_code}): {error}")?,
        }
    }
    Ok(())
}

fn emit_one_ok(out: &mut impl Write, outcome: &HarnessRemoveOutcome) -> Result<(), TomeError> {
    if !outcome.list_changed {
        writeln!(
            out,
            "Harness `{}` was not present in {} settings ({}). No change.",
            outcome.name,
            outcome.scope,
            outcome.settings_path.display(),
        )?;
        return Ok(());
    }
    writeln!(
        out,
        "Removed `{}` from {} settings: {}",
        outcome.name,
        outcome.scope,
        outcome.settings_path.display(),
    )?;
    if outcome.sync_ran {
        writeln!(out, "Cleanup ran for the resolved project.")?;
    } else {
        writeln!(
            out,
            "Effective list unchanged — run `tome sync` in any project where this harness was active.",
        )?;
    }
    Ok(())
}
