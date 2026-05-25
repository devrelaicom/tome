//! `tome workspace rename <old> <new>` CLI wrapper.
//!
//! The DB + filesystem work lives in [`crate::workspace::rename`]; this
//! module is the thin arg-validation + emit layer per the silent-compute
//! / emit-wrapper pattern.

use std::io::Write;

use crate::cli::WorkspaceRenameArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, RenameOutcome, WorkspaceName};

pub fn run(args: WorkspaceRenameArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let old = WorkspaceName::parse(&args.old)?;
    let new = WorkspaceName::parse(&args.new)?;
    let outcome = workspace::rename::rename(old, new, paths)?;
    emit(&outcome, mode)
}

fn emit(outcome: &RenameOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(outcome),
        Mode::Json => write_json(outcome),
    }
}

fn emit_human(outcome: &RenameOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Renamed workspace `{}` to `{}`",
        outcome.old_name.as_str(),
        outcome.new_name.as_str(),
    )?;
    writeln!(
        out,
        "  directory:        {}",
        outcome.workspace_dir.display(),
    )?;
    writeln!(
        out,
        "  bound projects:   {} marker(s) updated",
        outcome.bound_projects_updated,
    )?;
    Ok(())
}
