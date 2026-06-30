//! `tome harness use [<name>...] [--all] [--scope project|workspace|global] [--force]`
//!
//! Appends each selected harness to the chosen scope's settings file
//! (`harnesses` array) and runs the sync algorithm when the effective list
//! changes. Phase 11 / US6 widened this from a single name to a multi-harness
//! selection:
//!
//! - **no names + no `--all`** → every AUTO-DETECTED harness (opt-in targets
//!   are inert-detect, so excluded).
//! - **names given** → exactly those, each resolved via `lookup` so aliases
//!   (`antigravity-cli` → `gemini`) and opt-in targets (`generic`) by name work.
//! - **`--all`** → every real `SUPPORTED_HARNESSES` module (NOT the opt-in
//!   `generic` / `generic-op` targets). `--all` + names is a clap conflict.
//!
//! Alias resolution happens BEFORE dedupe (M5): `tome harness use
//! antigravity-cli gemini` collapses to a SINGLE `gemini` configuration pass.
//!
//! Forward-progress (`first_error`): each selected harness is configured in
//! turn; a per-harness failure is recorded and the loop CONTINUES, surfacing
//! the first error's exit code at the end so partial progress still lands.
//!
//! Validation: an explicitly-named harness must resolve via `lookup` (or the
//! effective-registry override), else exit 18 (`HarnessNotSupported`). For
//! `--scope project`, the resolved scope MUST carry a project root; otherwise
//! exit 2 (Usage).
//!
//! ## Concurrency (C-M5 / R-M2 / S-M2 from US3 review)
//!
//! The advisory lock at `paths.index_lock` is held across the whole
//! read-modify-write window — now spanning the WHOLE multi-harness loop, so
//! the entire selection is one atomic settings transaction. Two concurrent
//! `tome harness use` calls against the same project would otherwise race on
//! the on-disk file (last-writer-wins on the rename, but the lost edit is
//! silent). `IndexBusy` (exit 50) is the documented contention error.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{HarnessScopeArg, HarnessUseArgs};
use crate::error::TomeError;
use crate::harness::{resolve_alias, sync, with_effective_modules};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::edit::{add_harness, add_harness_to_config, open_settings, save_settings};
use crate::workspace::ResolvedScope;

use super::{effective_harness_scope, home_root};

/// One harness's `tome harness use` outcome.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessUseOutcome {
    pub scope: String,
    pub name: String,
    pub settings_path: PathBuf,
    pub list_changed: bool,
    /// `true` iff the sync algorithm ran for this harness (i.e. effective list
    /// post-edit differs from the pre-edit effective list).
    pub sync_ran: bool,
    /// Phase 11 / US5 (T064, M6/FR-011): actionable MCP-only notice surfaced
    /// when this harness's MCP server must be added (or activated) by hand —
    /// `manual` (jetbrains-ai: no file written, paste the snippet) or
    /// `unverified` (pi: file written but an adapter is required). The
    /// success-with-notice is scoped to MCP ONLY: a failure in ANY
    /// auto-writable capability (rules / hooks) still errors with its normal
    /// exit code BEFORE this is reached, because `sync_project` propagates
    /// those errors and the notice only emits on a successful `use`.
    ///
    /// Appended LAST + `skip_serializing_if`-gated so existing `--json` pins
    /// don't move; `None` for every harness with a fully-automatic MCP write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_notice: Option<String>,
}

/// One selected harness's result: either a successful [`HarnessUseOutcome`] or
/// a failure recorded by the forward-progress loop. Serialised in the `--json`
/// envelope so a partial-failure run is fully machine-readable.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HarnessUseResult {
    /// The harness was configured successfully (sync ran, or no-op).
    Ok(HarnessUseOutcome),
    /// The harness failed; `name` is the canonical harness, `error` the
    /// rendered message, `exit_code` the closed-set code the run will surface
    /// (the FIRST failure's code is the process exit).
    Failed {
        name: String,
        error: String,
        exit_code: i32,
    },
}

/// The full `tome harness use` report over the selected harness set.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessUseReport {
    /// How the selection was derived: `"detected"` (no names, no `--all`),
    /// `"explicit"` (names given), or `"all"` (`--all`).
    pub selection: &'static str,
    /// Per-harness results, in selection order.
    pub results: Vec<HarnessUseResult>,
}

pub fn run(
    args: HarnessUseArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let (report, first_error) = run_inner(args, scope, paths)?;

    // Telemetry parity with the former single-name path: emit one
    // `tome.harness_action{Use}` per harness that configured successfully.
    // Best-effort, success-path only; failed harnesses emit nothing.
    for result in &report.results {
        if let HarnessUseResult::Ok(outcome) = result {
            super::emit_harness_action(&outcome.name, crate::telemetry::event::HarnessAction::Use);
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

/// Compute the full [`HarnessUseReport`] — selection resolution + the
/// per-harness read-modify-write + sync + notice pipeline, MINUS the terminal
/// emit (the "silent compute / emit wrapper" split). `run` wraps this and
/// emits per mode; integration tests call this directly to assert the EMITTED
/// report (e.g. `mcp_notice`, forward-progress) without capturing stdout,
/// proving the real `run → resolve_selection → configure_one → report` chain.
///
/// Returns the report PLUS the first error captured by the forward-progress
/// loop (so `run` can emit the report and still set the exit code). The
/// `selection` resolution itself can fail loud (an unknown explicit name → 18,
/// a `--scope project` with no project root → 2); those propagate via the
/// outer `Result` (before any harness is touched).
#[doc(hidden)]
pub fn run_inner(
    args: HarnessUseArgs,
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<(HarnessUseReport, Option<TomeError>), TomeError> {
    let home = home_root()?;

    // 1. Resolve the selection set FIRST (alias-resolve → dedupe → order-
    //    preserve). An unknown explicit name fails loud here (exit 18) before
    //    any settings edit.
    let (selection, names) = resolve_selection(&args, &home)?;

    // 2. Resolve the effective scope then the target settings file path.
    //    Precedence: explicit --scope → [harness] default_scope in config → project.
    //    Loud on project scope with no project root (exit 2).
    let eff_scope = effective_harness_scope(args.scope, paths)?;
    let settings_path = resolve_settings_path(&eff_scope, scope, paths)?;

    // Hold the advisory lock across the ENTIRE multi-harness window so the
    // whole selection is one atomic settings transaction.
    std::fs::create_dir_all(&paths.root)?;
    let _lock = crate::index::acquire_lock(&paths.index_lock)?;

    let mut results: Vec<HarnessUseResult> = Vec::with_capacity(names.len());
    let mut first_error: Option<TomeError> = None;

    for name in names {
        match configure_one(&name, &args, eff_scope, scope, paths, &settings_path, &home) {
            Ok(outcome) => results.push(HarnessUseResult::Ok(outcome)),
            Err(e) => {
                // Forward-progress: record + continue so every selected harness
                // is attempted; the first failure sets the process exit code.
                tracing::warn!(
                    harness = name.as_str(),
                    error = %e,
                    "harness use: harness failed; continuing",
                );
                results.push(HarnessUseResult::Failed {
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

    Ok((HarnessUseReport { selection, results }, first_error))
}

/// Resolve the variadic / `--all` / detected selection into an ordered,
/// deduped list of CANONICAL harness names plus the selection-mode label.
///
/// - explicit names → each `resolve_alias`'d then validated via `lookup` (or
///   the effective-registry override). Aliases resolve BEFORE dedupe (M5), so
///   `antigravity-cli gemini` → one `gemini`. An unknown name → exit 18.
/// - `--all` → every `SUPPORTED_HARNESSES` real module name (opt-in targets
///   are not in that slice, so excluded).
/// - neither → every AUTO-DETECTED supported harness (opt-in targets are
///   inert-detect → excluded).
fn resolve_selection(
    args: &HarnessUseArgs,
    home: &std::path::Path,
) -> Result<(&'static str, Vec<String>), TomeError> {
    if !args.names.is_empty() {
        // Explicit names: alias-resolve → validate → order-preserving dedupe.
        let mut seen: Vec<String> = Vec::with_capacity(args.names.len());
        for raw in &args.names {
            // M5: resolve the alias to its canonical name BEFORE dedupe.
            let canonical = resolve_alias(raw).to_string();
            // Validate against the effective registry (override-aware) or the
            // alias+opt-in-aware `lookup`.
            let supported =
                with_effective_modules(|mods| mods.iter().any(|m| m.name() == canonical))
                    || crate::harness::lookup(&canonical).is_some();
            if !supported {
                return Err(TomeError::HarnessNotSupported { name: raw.clone() });
            }
            if !seen.contains(&canonical) {
                seen.push(canonical);
            }
        }
        Ok(("explicit", seen))
    } else if args.all {
        // `--all`: every real supported module (NOT the opt-in generics). Uses
        // the effective registry so a test override is honoured.
        let names =
            with_effective_modules(|mods| mods.iter().map(|m| m.name().to_string()).collect());
        Ok(("all", names))
    } else {
        // Default: every auto-detected supported harness. Opt-in targets are
        // inert-detect, so they never surface here.
        let names = with_effective_modules(|mods| {
            mods.iter()
                .filter(|m| m.detect(home))
                .map(|m| m.name().to_string())
                .collect()
        });
        Ok(("detected", names))
    }
}

/// Configure ONE harness: the read-modify-write + sync + notice pipeline for a
/// single canonical `name`. The advisory lock is already held by the caller
/// (`run_inner`), and the settings path is resolved once.
fn configure_one(
    name: &str,
    args: &HarnessUseArgs,
    eff_scope: HarnessScopeArg,
    scope: &ResolvedScope,
    paths: &Paths,
    settings_path: &std::path::Path,
    home: &std::path::Path,
) -> Result<HarnessUseOutcome, TomeError> {
    // 1. Snapshot pre-edit effective list (when a project root is resolved;
    //    otherwise sync is meaningless).
    let pre = match scope.project_root.as_deref() {
        Some(_) => Some(compute_effective_names(scope, paths)?),
        None => None,
    };

    // 2. Read-modify-write settings file.
    // Global scope writes to `config.toml [harness].enabled`; other scopes
    // use the legacy `harnesses = [...]` key in workspace/project settings.
    let mut doc = open_settings(settings_path)?;
    let changed = if settings_path == paths.global_config_file.as_path() {
        add_harness_to_config(&mut doc, name)
    } else {
        add_harness(&mut doc, name)
    };
    if changed {
        save_settings(settings_path, &doc)?;
    }

    // 3. Recompute effective list and dispatch sync when it changed.
    let mut sync_ran = false;
    if let Some(pre_names) = pre {
        let post_names = compute_effective_names(scope, paths)?;
        if pre_names != post_names {
            let project_root = scope
                .project_root
                .as_deref()
                .expect("project_root present when pre was Some");
            // PERF (US6 NON-BLOCKING note): `build_deps` sets `only_harness =
            // None`, so each harness in a `--all` / multi-name `use` runs a FULL
            // `sync_project` reconcile of the WHOLE effective set — O(N) reconciles
            // for an N-harness selection (and each walks every registered module's
            // write/cleanup decision). Redundant but correct + idempotent; left
            // as-is deliberately. Scoping this to the single harness (via
            // `only_harness`) would change single-`use` semantics (it would stop
            // reconciling co-owned shared sinks for other live harnesses), so the
            // full reconcile is the safe choice. `tome sync --harness` is the
            // scoped path when a caller wants per-harness reconcile.
            let deps = sync::build_deps(paths, home, scope.scope.name(), args.force);
            sync::sync_project(project_root, &deps)?;
            sync_ran = true;
        }
    }

    // 4. Phase 11 / US5 (T064): compute the MCP-only notice. Reached only on a
    //    successful `use` (any rules/hook write failure errored out of
    //    `sync_project` above), so success-with-notice is structurally scoped
    //    to MCP only (M6/FR-011).
    let mcp_notice = compute_mcp_notice(name, scope.scope.name().as_str());

    Ok(HarnessUseOutcome {
        scope: eff_scope.to_string(),
        name: name.to_string(),
        settings_path: settings_path.to_path_buf(),
        list_changed: changed,
        sync_ran,
        mcp_notice,
    })
}

/// Build the MCP-only notice (T064) for the harness named `name` in workspace
/// `workspace_name`, or `None` if the harness writes its MCP file fully
/// automatically. Pure read against the effective harness registry.
///
/// - `mcp_manual_only` (jetbrains-ai): no MCP file is written; the notice tells
///   the user to add the server by hand and includes the EXACT paste-able
///   snippet (byte-identical to what `tome harness info` prints — both build it
///   via `render_entry_snippet`), plus a pointer to `tome harness info`.
/// - `mcp_adapter_notice` (pi): the MCP file IS written, but an external adapter
///   must be installed; the notice carries the harness's install instruction.
///
/// #337: the snippet `command` is the PORTABLE bare `tome` (human-pasteable),
/// which deliberately DIVERGES from the resolved absolute launcher `sync` now
/// writes via `tome_command()` (matches the `info` snippet's note). A bare-`tome`
/// pasted entry is still recognised as Tome-owned (basename match). Making the
/// manual-host snippet PATH-tolerant is deferred to Phase B.
///
/// `#[doc(hidden)] pub` for integration-test reachability (the outcome is
/// emitted, not returned, so tests assert the notice via this fn).
#[doc(hidden)]
pub fn compute_mcp_notice(name: &str, workspace_name: &str) -> Option<String> {
    use crate::harness::mcp_config;

    with_effective_modules(|mods| {
        let module = mods.iter().find(|m| m.name() == name)?;
        if module.mcp_manual_only() {
            let entry = mcp_config::TomeEntry::new(
                "tome".to_string(),
                vec![
                    "mcp".to_string(),
                    "--workspace".to_string(),
                    workspace_name.to_string(),
                    "--harness".to_string(),
                    module.name().to_string(),
                ],
            );
            let snippet = mcp_config::render_entry_snippet(&module.mcp_dialect(), &entry);
            Some(format!(
                "{} configures its MCP server manually — Tome wrote no MCP file. \
                 Add the Tome server by hand (or run `tome harness info {}`):\n\n{snippet}",
                module.description(),
                module.name(),
            ))
        } else {
            module.mcp_adapter_notice().map(str::to_string)
        }
    })
}

pub(crate) fn resolve_settings_path(
    arg_scope: &HarnessScopeArg,
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<PathBuf, TomeError> {
    match arg_scope {
        HarnessScopeArg::Project => {
            let Some(project_root) = scope.project_root.as_deref() else {
                return Err(TomeError::Usage(
                    "no project marker found above CWD; specify --scope workspace or --scope global, or run from a project directory bound via `tome workspace use`"
                        .into(),
                ));
            };
            Ok(Paths::project_marker_config(project_root))
        }
        HarnessScopeArg::Workspace => Ok(paths.workspace_settings_file(scope.scope.name())),
        HarnessScopeArg::Global => Ok(paths.global_config_file.clone()),
    }
}

pub(crate) fn compute_effective_names(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Vec<String>, TomeError> {
    use crate::settings::resolver::resolve_effective_list;

    let marker = super::list::load_project_marker_for_use(scope)?;
    let workspace_settings = super::list::load_workspace_settings_for_use(scope, paths)?;
    let global_settings = super::list::load_global_settings_for_use(paths)?;
    let provider = super::CentralDbScopeProvider::new(paths);

    let resolved = resolve_effective_list(
        marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &provider,
    )
    .map_err(TomeError::from)?;
    Ok(resolved.harnesses.into_iter().map(|h| h.name).collect())
}

fn emit_human(report: &HarnessUseReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();

    if report.results.is_empty() {
        // The only way to reach an empty selection is the detected-default
        // path in a project with NO detected harness — make that explicit
        // rather than a silent success.
        match report.selection {
            "detected" => writeln!(
                out,
                "No harness detected. Name one explicitly (e.g. `tome harness use claude-code`) or pass `--all` to configure every supported harness.",
            )?,
            _ => writeln!(out, "No harnesses selected.")?,
        }
        return Ok(());
    }

    writeln!(
        out,
        "Configuring {} harness(es) [{}]:",
        report.results.len(),
        report.selection,
    )?;

    for result in &report.results {
        match result {
            HarnessUseResult::Ok(outcome) => emit_one_ok(&mut out, outcome)?,
            HarnessUseResult::Failed {
                name,
                error,
                exit_code,
            } => writeln!(out, "  {name}: FAILED (exit {exit_code}): {error}")?,
        }
    }
    Ok(())
}

fn emit_one_ok(out: &mut impl Write, outcome: &HarnessUseOutcome) -> Result<(), TomeError> {
    if !outcome.list_changed {
        writeln!(
            out,
            "  {}: already present in {} settings ({}). No change.",
            outcome.name,
            outcome.scope,
            outcome.settings_path.display(),
        )?;
    } else {
        writeln!(
            out,
            "  {}: added to {} settings ({}).",
            outcome.name,
            outcome.scope,
            outcome.settings_path.display(),
        )?;
        if outcome.sync_ran {
            writeln!(out, "    Sync ran for the resolved project.")?;
        } else {
            writeln!(
                out,
                "    Effective list unchanged — run `tome sync` where this harness should activate.",
            )?;
        }
    }
    if let Some(notice) = &outcome.mcp_notice {
        writeln!(out)?;
        writeln!(out, "    Note (MCP): {notice}")?;
    }
    Ok(())
}
