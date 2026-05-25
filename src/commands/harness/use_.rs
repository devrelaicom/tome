//! `tome harness use <name> [--scope project|workspace|global] [--force]`
//!
//! Appends `<name>` to the chosen scope's settings file (`harnesses`
//! array) and runs the sync algorithm when the effective list changes.
//!
//! Validation: `<name>` must be a supported harness (lookup against
//! [`crate::harness::lookup`]) — otherwise exit 18
//! (`HarnessNotSupported`). For `--scope project`, the resolved scope
//! MUST carry a project root; otherwise exit 2 (Usage).

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
}

pub fn run(
    args: HarnessUseArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
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

    let outcome = HarnessUseOutcome {
        scope: args.scope.to_string(),
        name: args.name,
        settings_path: settings_path.clone(),
        list_changed: changed,
        sync_ran,
    };

    match mode {
        Mode::Human => emit_human(&outcome, scope, sync_ran),
        Mode::Json => write_json(&outcome),
    }
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
    let provider = super::PathsScopeProvider::new(paths);

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
            "Effective list unchanged — run `tome harness sync` in any project where this harness should activate.",
        )?;
    }
    Ok(())
}
