//! `tome workspace remove <name> [--force]` CLI wrapper.
//!
//! The 5-step cascade lives in [`crate::workspace::remove`]; this module
//! is the thin arg-validation + emit layer per the silent-compute /
//! emit-wrapper pattern.

use std::io::Write;
use std::path::PathBuf;

use crate::cli::WorkspaceRemoveArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, RemoveOutcome, WorkspaceName};

pub fn run(args: WorkspaceRemoveArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;
    let home_root = resolve_home_root()?;
    let outcome = workspace::remove::remove(name, args.force, paths, &home_root)?;
    emit(&outcome, mode)
}

/// Resolve `$HOME` for the harness-teardown step. Returned as a
/// `PathBuf` so the borrow in [`workspace::remove::remove`] is
/// straightforward. The same resolver shape lives on the `bind_project`
/// path; tests bypass this by calling the library API directly with a
/// `TempDir`-rooted home.
fn resolve_home_root() -> Result<PathBuf, TomeError> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        TomeError::Io(std::io::Error::other(
            "HOME is not set — cannot tear down harness integration during workspace removal",
        ))
    })
}

fn emit(outcome: &RemoveOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(outcome),
        Mode::Json => write_json(outcome),
    }
}

fn emit_human(outcome: &RemoveOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Removed workspace `{}`", outcome.removed.as_str())?;
    writeln!(
        out,
        "  bound projects torn down: {}",
        outcome.bound_projects_torn_down,
    )?;
    writeln!(
        out,
        "  catalog caches cleaned:   {}",
        outcome.catalog_caches_cleaned.len(),
    )?;
    if !outcome.orphaned_paths.is_empty() {
        writeln!(out, "  orphaned paths (run `tome doctor --fix` to clean):",)?;
        for p in &outcome.orphaned_paths {
            writeln!(out, "    - {}", p.display())?;
        }
    }
    Ok(())
}
