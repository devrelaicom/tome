//! `tome workspace regen-summary [<name>]` CLI wrapper.
//!
//! The compute path lives in [`crate::workspace::regen_summary`]; this
//! module resolves the workspace name, selects the summariser via
//! [`crate::summarise::build_summariser`] (a configured remote provider, else
//! the bundled Qwen), and emits the outcome. A foreground provider failure
//! PROPAGATES (fail-loud, exit 94 — FR-027).
//!
//! Issue #321 makes the `<name>` positional optional. With a name it is
//! byte-identical to the pre-#321 behaviour (no confirmation — the explicit
//! name IS the confirmation). Omitted, it resolves the workspace from the
//! scope (mirroring `info [<name>]`) and CONFIRMS before regenerating; on a
//! non-terminal it refuses ([`TomeError::NotATerminal`], exit 54) so it never
//! silently regenerates the resolved (often `global`) scope.
//!
//! `run_with_summariser` is the dependency-injection seam used by
//! tests — it bypasses the production summariser selection and accepts a
//! `&dyn Summariser` directly.

use std::io::Write;

use crate::cli::WorkspaceRegenSummaryArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::prompt;
use crate::summarise::prompts::validate_long_max_chars;
use crate::summarise::{LONG_MAX_CHARS, Summariser, build_summariser};
use crate::workspace::{self, RegenSummaryOutcome, ResolvedScope, WorkspaceName};

pub fn run(
    args: WorkspaceRegenSummaryArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Resolve the target name, and decide whether a confirmation gate
    // applies. An explicit `<name>` is its own confirmation (no gate); an
    // omitted name resolves the scope + gates behind an interactive confirm.
    let name = match args.name.as_deref() {
        Some(raw) => WorkspaceName::parse(raw)?,
        None => {
            let resolved = scope.scope.name().clone();
            // On a non-terminal, refuse rather than silently regenerating the
            // resolved scope (often `global`). The name is required there.
            if prompt::non_interactive() {
                return Err(TomeError::Usage(format!(
                    "workspace regen-summary requires an explicit <name> on a non-terminal; \
                     run `tome workspace regen-summary {}`",
                    resolved.as_str(),
                )));
            }
            // #435: name the non-interactive alternative BEFORE the generic
            // `NotATerminal` refusal `prompt::confirm` would raise, mirroring
            // the `plugin disable` / `models remove` pointer discipline.
            if !(crate::output::stdin_is_tty() && crate::output::stdout_is_tty()) {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(
                    err,
                    "Confirmation needs a terminal. Run `tome workspace regen-summary {}` to name the workspace explicitly.",
                    resolved.as_str(),
                );
                return Err(TomeError::NotATerminal);
            }
            let ok = prompt::confirm(
                &format!(
                    "Regenerate summaries for workspace `{}`?",
                    resolved.as_str()
                ),
                false,
            )?;
            if !ok {
                // Declined — a clean no-op (exit 0).
                return Ok(());
            }
            resolved
        }
    };
    // Load config strictly (exit 5 on malformed) — the explicit regen-summary
    // command surfaces config errors loudly. `build_summariser` selects a remote
    // provider summariser when `[summariser] provider` is set, else the bundled
    // Qwen. A foreground provider failure PROPAGATES (exit 94, fail-loud).
    let cfg = crate::config::load(paths)?;
    let summariser = build_summariser(&cfg, paths, false)?;
    run_with_summariser(&name, summariser.as_ref(), paths, mode)
}

/// Dependency-injection variant used by tests. Production code goes
/// through [`run`], which selects the summariser via
/// [`crate::summarise::build_summariser`].
///
/// Loads the global config strictly (exit 5 on malformed) to resolve
/// `effective_long_max`. The explicit regen-summary command surfaces
/// config errors loudly — unlike the trigger path which uses
/// `load_or_default` to never fail a post-commit summarisation step.
pub fn run_with_summariser(
    name: &WorkspaceName,
    summariser: &dyn Summariser,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let cfg = crate::config::load(paths)?;
    let effective_long_max =
        validate_long_max_chars(cfg.summariser.long_max_chars.unwrap_or(LONG_MAX_CHARS));
    let outcome = workspace::regen_summary::regen(name, summariser, paths, effective_long_max)?;
    emit(&outcome, mode)
}

fn emit(outcome: &RegenSummaryOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(outcome),
        Mode::Json => write_json(outcome),
    }
}

fn emit_human(outcome: &RegenSummaryOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Regenerated summary for workspace `{}`",
        outcome.workspace.as_str(),
    )?;
    writeln!(out, "  short:           {} chars", outcome.short_chars,)?;
    writeln!(out, "  long:            {} chars", outcome.long_chars,)?;
    writeln!(
        out,
        "  bound projects:  {} synced",
        outcome.bound_projects_synced,
    )?;
    Ok(())
}
