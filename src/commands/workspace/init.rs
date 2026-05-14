//! `tome workspace init` CLI wrapper. The atomic FS work lives in
//! [`crate::workspace::init`]; this module is the thin
//! arg-validation + emit layer.

use std::io::Write;

use crate::cli::WorkspaceInitArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, InitOutcome};

pub fn run(args: WorkspaceInitArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let target = args
        .path
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let outcome = workspace::init(&target, args.inherit_global, args.force, paths)?;
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
        "Initialized workspace at {}",
        outcome.workspace.display()
    )?;
    if outcome.inherited {
        writeln!(
            out,
            "  catalogs: {} (inherited from global)",
            outcome.catalogs,
        )?;
    } else {
        writeln!(out, "  catalogs: {}", outcome.catalogs)?;
    }
    writeln!(out, "  config:   {}", outcome.config_path.display())?;
    writeln!(
        out,
        "  index:    not yet bootstrapped (will be created on first enable)",
    )?;

    // Helpful Next-step hint when there are no catalogs and the user
    // didn't ask to inherit. Mirrors the contract example.
    if !inherit_requested && outcome.catalogs == 0 {
        writeln!(out)?;
        writeln!(
            out,
            "Next: run `tome --workspace {} catalog add <source>` to add a catalog,",
            outcome.workspace.display(),
        )?;
        writeln!(
            out,
            "      or rerun init with --inherit-global to seed catalogs from the global config.",
        )?;
    }
    Ok(())
}
