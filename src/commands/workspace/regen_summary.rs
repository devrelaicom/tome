//! `tome workspace regen-summary [<name>]` CLI wrapper.
//!
//! The compute path lives in [`crate::workspace::regen_summary`]; this
//! module resolves the workspace name, selects the summariser via
//! [`crate::summarise::build_summariser`] (a configured remote provider, else
//! the bundled Qwen), and emits the outcome. A foreground provider failure
//! PROPAGATES (fail-loud, exit 94 — FR-027).
//!
//! `run_with_summariser` is the dependency-injection seam used by
//! tests — it bypasses the production summariser selection and accepts a
//! `&dyn Summariser` directly.

use std::io::Write;

use crate::cli::WorkspaceRegenSummaryArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::summarise::prompts::validate_long_max_chars;
use crate::summarise::{LONG_MAX_CHARS, Summariser, build_summariser};
use crate::workspace::{self, RegenSummaryOutcome, ResolvedScope, WorkspaceName};

pub fn run(
    args: WorkspaceRegenSummaryArgs,
    _scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;
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
