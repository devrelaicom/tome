//! `tome harness use <name> [--scope project|workspace|global] [--force]`
//!
//! Appends `<name>` to the chosen scope's settings file (`harnesses`
//! array) and runs the sync algorithm when the effective list changes.
//!
//! Validation: `<name>` must be a supported harness (lookup against
//! [`crate::harness::with_effective_modules`]) — otherwise exit 18
//! (`HarnessNotSupported`). For `--scope project`, the resolved scope
//! MUST carry a project root; otherwise exit 2 (Usage).
//!
//! ## Concurrency (C-M5 / R-M2 / S-M2 from US3 review)
//!
//! The advisory lock at `paths.index_lock` is held across the whole
//! read-modify-write window: pre-edit effective list snapshot → open /
//! mutate / persist the settings document → post-edit effective list
//! recompute → sync dispatch. Two concurrent `tome harness use` calls
//! against the same project would otherwise race on the on-disk file
//! (last-writer-wins on the rename, but the lost edit is silent).
//! `IndexBusy` (exit 50) is the documented contention error.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{HarnessScopeArg, HarnessUseArgs};
use crate::error::TomeError;
use crate::harness::{sync, with_effective_modules};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::edit::{add_harness, open_settings, save_settings};
use crate::workspace::ResolvedScope;

use super::home_root;

#[derive(Debug, Clone, Serialize)]
pub struct HarnessUseOutcome {
    pub scope: String,
    pub name: String,
    pub settings_path: PathBuf,
    pub list_changed: bool,
    /// `true` iff the sync algorithm ran (i.e. effective list
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

pub fn run(
    args: HarnessUseArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let sync_ran_for_human = std::cell::Cell::new(false);
    let outcome = run_inner(args, scope, paths, &sync_ran_for_human)?;

    match mode {
        Mode::Human => emit_human(&outcome, scope, sync_ran_for_human.get()),
        Mode::Json => write_json(&outcome),
    }
}

/// Compute the full [`HarnessUseOutcome`] — the entire read-modify-write +
/// sync + notice pipeline, MINUS the terminal emit (the "silent compute /
/// emit wrapper" split). `run` wraps this and emits per mode; integration
/// tests call this directly to assert the EMITTED outcome (e.g. `mcp_notice`)
/// without capturing stdout, proving the real `run → compute_mcp_notice →
/// outcome` chain rather than just the helper.
///
/// `sync_ran_out` carries the human-mode-only `sync_ran` signal back to `run`
/// (the field on the outcome is the same value, but `emit_human` takes it
/// positionally for the legacy signature).
#[doc(hidden)]
pub fn run_inner(
    args: HarnessUseArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    sync_ran_out: &std::cell::Cell<bool>,
) -> Result<HarnessUseOutcome, TomeError> {
    // 1. Validate harness name against the effective registry
    //    (consults `HARNESS_MODULES_OVERRIDE` for tests).
    let supported = with_effective_modules(|mods| mods.iter().any(|m| m.name() == args.name));
    if !supported {
        return Err(TomeError::HarnessNotSupported {
            name: args.name.clone(),
        });
    }

    // 2. Resolve target settings file path.
    let settings_path = resolve_settings_path(&args.scope, scope, paths)?;

    // Hold the advisory lock across the entire pre-snapshot → edit →
    // post-snapshot → sync window. The lock path's parent (`paths.root`)
    // is created by Paths construction; the lockfile itself is touched
    // on first acquire.
    std::fs::create_dir_all(&paths.root)?;
    let _lock = crate::index::acquire_lock(&paths.index_lock)?;

    // 3. Snapshot pre-edit effective list (when a project root is
    //    resolved; otherwise sync is meaningless).
    let pre = match scope.project_root.as_deref() {
        Some(_) => Some(compute_effective_names(scope, paths)?),
        None => None,
    };

    // 4. Read-modify-write settings file.
    let mut doc = open_settings(&settings_path)?;
    let changed = add_harness(&mut doc, &args.name);
    if changed {
        save_settings(&settings_path, &doc)?;
    }

    // 5. Recompute effective list and dispatch sync when it changed.
    let mut sync_ran = false;
    if let Some(pre_names) = pre {
        let post_names = compute_effective_names(scope, paths)?;
        if pre_names != post_names {
            let project_root = scope
                .project_root
                .as_deref()
                .expect("project_root present when pre was Some");
            let home = home_root()?;
            let deps = sync::build_deps(paths, &home, scope.scope.name(), args.force);
            sync::sync_project(project_root, &deps)?;
            sync_ran = true;
        }
    }

    // Phase 11 / US5 (T064): compute the MCP-only notice. Reached only on a
    // successful `use` (any rules/hook write failure errored out of
    // `sync_project` above), so success-with-notice is structurally scoped to
    // MCP only (M6/FR-011). A `mcp_manual_only` harness (jetbrains-ai) gets the
    // paste-the-snippet notice; an adapter harness (pi) gets its
    // `mcp_adapter_notice`. Resolved against the effective registry (alias-
    // aware, override-aware), keyed off the resolved workspace name.
    let mcp_notice = compute_mcp_notice(&args.name, scope.scope.name().as_str());

    sync_ran_out.set(sync_ran);

    Ok(HarnessUseOutcome {
        scope: args.scope.to_string(),
        name: args.name,
        settings_path: settings_path.clone(),
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
///   snippet (the same bytes `tome harness info` prints), plus a pointer to
///   `tome harness info`.
/// - `mcp_adapter_notice` (pi): the MCP file IS written, but an external adapter
///   must be installed; the notice carries the harness's install instruction.
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
        HarnessScopeArg::Global => Ok(paths.global_settings_file.clone()),
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

fn emit_human(
    outcome: &HarnessUseOutcome,
    _scope: &ResolvedScope,
    sync_ran: bool,
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if !outcome.list_changed {
        writeln!(
            out,
            "Harness `{}` was already present in {} settings ({}). No change.",
            outcome.name,
            outcome.scope,
            outcome.settings_path.display(),
        )?;
        emit_mcp_notice(&mut out, outcome)?;
        return Ok(());
    }
    writeln!(
        out,
        "Added `{}` to {} settings: {}",
        outcome.name,
        outcome.scope,
        outcome.settings_path.display(),
    )?;
    if sync_ran {
        writeln!(out, "Sync ran for the resolved project.")?;
    } else {
        writeln!(
            out,
            "Effective list unchanged — run `tome sync` in any project where this harness should activate.",
        )?;
    }
    emit_mcp_notice(&mut out, outcome)?;
    Ok(())
}

/// Print the MCP-only notice (T064), if any, under a clear "Note:" heading.
fn emit_mcp_notice(out: &mut impl Write, outcome: &HarnessUseOutcome) -> Result<(), TomeError> {
    if let Some(notice) = &outcome.mcp_notice {
        writeln!(out)?;
        writeln!(out, "Note (MCP): {notice}")?;
    }
    Ok(())
}
