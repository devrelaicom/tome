//! `tome workspace init <name>` CLI wrapper.
//!
//! The atomic DB + filesystem work lives in [`crate::workspace::init`];
//! this module is the thin arg-validation + emit layer per the silent-
//! compute / emit-wrapper pattern documented on CLAUDE.md.

use std::io::Write;

use crate::cli::WorkspaceInitArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, InitOutcome, WorkspaceName};

pub fn run(args: WorkspaceInitArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;
    let outcome = workspace::init::init(name, args.inherit_global, paths)?;
    emit(&outcome, args.inherit_global, mode)
}

fn emit(outcome: &InitOutcome, inherit_requested: bool, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(outcome, inherit_requested),
        Mode::Json => write_json(outcome),
    }
}

fn emit_human(outcome: &InitOutcome, inherit_requested: bool) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Initialised workspace `{}` at {}",
        outcome.name.as_str(),
        outcome.workspace_dir.display(),
    )?;
    if outcome.inherited_catalogs > 0 {
        writeln!(
            out,
            "  catalogs: {} (inherited from global)",
            outcome.inherited_catalogs,
        )?;
    } else if inherit_requested {
        // Documented no-op per FR-400: --inherit-global was set but
        // global had no enrolments.
        writeln!(
            out,
            "  catalogs: 0 (--inherit-global: global has no enrolled catalogs)",
        )?;
    } else {
        writeln!(out, "  catalogs: 0")?;
    }
    writeln!(
        out,
        "  next:     `cd <project> && tome workspace use {}` to bind a project",
        outcome.name.as_str(),
    )?;
    Ok(())
}
