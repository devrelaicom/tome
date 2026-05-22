//! `tome plugin disable <catalog>/<plugin>`.
//!
//! Thin CLI wrapper over `plugin::lifecycle::disable`. The lock acquisition,
//! "already-disabled" detection, and atomic UPDATE all live in the library.
//! This module owns the confirmation-prompt UX (`--force` short-circuit,
//! non-TTY refusal with pointer message) and the human / JSON presentation
//! contract.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin disable`".

use std::io::Write;
use std::str::FromStr;

use serde::Serialize;

use crate::catalog::store;
use crate::cli::PluginDisableArgs;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, DisableOutcome};
use crate::workspace::ResolvedScope;

use super::{registry_seeds, resolve_plugin_dir};

pub fn run(args: PluginDisableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 reintroduces workspace-aware view.
    let config = store::load(&paths.global_config_file)?;

    // Surface CatalogNotFound / PluginNotFound before any prompt — typo on
    // the address shouldn't waste the user's "y" keystroke.
    let _ = resolve_plugin_dir(&id, &config)?;

    if !args.force {
        // Non-TTY without --force → exit 54 per FR-007 / FR-051. We emit the
        // documented pointer line to stderr before returning so the user
        // sees specific guidance, not just the generic NotATerminal Display.
        // (Same defensive pattern as the interactive flow — P4 retro
        // §"NotATerminal message duplication".)
        if !(output::stdin_is_tty() && output::stdout_is_tty()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "Disable requires confirmation. Re-run with --force to skip the prompt."
            );
            return Err(TomeError::NotATerminal);
        }

        // TTY: ask. Default no per contract step 3.
        if !crate::presentation::prompt::confirm(&format!("Disable {id}?"), false)? {
            // User declined — clean exit, no state change. Surfacing this as
            // Ok(()) matches the interactive flow's "decline, no error"
            // semantics. We emit a stderr note in human mode for parity with
            // the model-download decline path.
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: disable declined.");
            }
            return Ok(());
        }
    }

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();

    // Banner — human mode only. JSON stdout stays byte-stable.
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Disabling {}…", id)?;
    }

    let outcome = lifecycle::disable(
        &id,
        &paths,
        &scope.scope,
        &config,
        embedder_seed,
        reranker_seed,
        summariser_seed,
    )?;

    match mode {
        Mode::Human => emit_human(&id, &outcome),
        Mode::Json => emit_json(&id, &outcome),
    }
}

fn emit_human(id: &PluginId, outcome: &DisableOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} disabled {} ({} skill records retained)",
        crate::presentation::colour::success("✓"),
        id,
        outcome.skills_retained,
    )?;
    Ok(())
}

#[derive(Serialize)]
struct DisableRecord {
    plugin: String,
    status: &'static str,
    skills_retained: u32,
}

fn emit_json(id: &PluginId, outcome: &DisableOutcome) -> Result<(), TomeError> {
    let record = DisableRecord {
        plugin: id.to_string(),
        status: "disabled",
        skills_retained: outcome.skills_retained,
    };
    output::write_json(&record)
}
