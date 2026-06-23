//! `tome harness remove <name> [--scope project|workspace|global]`
//!
//! Mirror of `harness use`. Drops `<name>` from the chosen scope's
//! `harnesses` array and runs the cleanup pass when the effective list
//! changes (the harness's integration is torn down from the current
//! project).
//!
//! Validation: `<name>` does NOT need to be a supported harness — the
//! user may have a stale or typo'd entry in their settings file. We
//! still perform a lookup for early signal but proceed regardless when
//! the array literally contains the name; a missing name + missing
//! harness simply no-ops.
//!
//! ## Concurrency (C-M5 / R-M2 / S-M2 from US3 review)
//!
//! The advisory lock at `paths.index_lock` is held across the whole
//! read-modify-write window. See `harness::use_` for the rationale.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::HarnessRemoveArgs;
use crate::error::TomeError;
use crate::harness::sync;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::edit::{
    open_settings, remove_harness, remove_harness_from_config, save_settings,
};
use crate::workspace::ResolvedScope;

use super::{effective_harness_scope, home_root};
use super::use_::{compute_effective_names, resolve_settings_path};

#[derive(Debug, Clone, Serialize)]
pub struct HarnessRemoveOutcome {
    pub scope: String,
    pub name: String,
    pub settings_path: PathBuf,
    pub list_changed: bool,
    pub sync_ran: bool,
}

pub fn run(
    args: HarnessRemoveArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Resolve effective scope: explicit flag → [harness] default_scope in config → project.
    let eff_scope = effective_harness_scope(args.scope, paths);
    let settings_path = resolve_settings_path(&eff_scope, scope, paths)?;

    // Lock for the entire read-modify-write + sync window.
    std::fs::create_dir_all(&paths.root)?;
    let _lock = crate::index::acquire_lock(&paths.index_lock)?;

    let pre = match scope.project_root.as_deref() {
        Some(_) => Some(compute_effective_names(scope, paths)?),
        None => None,
    };

    // Global scope reads/writes `config.toml [harness].enabled`; other
    // scopes use the legacy `harnesses = [...]` key.
    let mut doc = open_settings(&settings_path)?;
    let changed = if settings_path == paths.global_config_file.as_path() {
        remove_harness_from_config(&mut doc, &args.name)
    } else {
        remove_harness(&mut doc, &args.name)
    };
    if changed {
        save_settings(&settings_path, &doc)?;
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
            // Cleanup never needs --force; the orchestrator only
            // refuses on user-owned MCP entries during a write, and
            // cleanup of a Tome-owned entry is unconditional.
            let deps = sync::build_deps(paths, &home, scope.scope.name(), false);
            sync::sync_project(project_root, &deps)?;
            sync_ran = true;
        }
    }

    let outcome = HarnessRemoveOutcome {
        scope: eff_scope.to_string(),
        name: args.name,
        settings_path: settings_path.clone(),
        list_changed: changed,
        sync_ran,
    };

    match mode {
        Mode::Human => emit_human(&outcome),
        Mode::Json => write_json(&outcome),
    }
}

fn emit_human(outcome: &HarnessRemoveOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
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
