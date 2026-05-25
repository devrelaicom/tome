//! `tome workspace use <name>` CLI wrapper. The binding algorithm lives
//! in [`crate::workspace::binding`]; the harness sync seam lives in
//! [`crate::commands::harness`]. This module is the thin arg-validation
//! + emit layer.
//!
//! Filename is `use_.rs` because `use` is a Rust keyword.

use std::io::Write;
use std::path::PathBuf;

use crate::cli::WorkspaceUseArgs;
use crate::commands::harness;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::binding::{self, BindDeps, BindOutcome};
use crate::workspace::name::WorkspaceName;

pub fn run(args: WorkspaceUseArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;

    let cwd = std::env::current_dir().map_err(TomeError::Io)?;

    let home_root = resolve_home()?;

    if !args.force {
        binding::is_project_root_acceptable(&cwd, &home_root)?;
    }

    let deps = BindDeps {
        paths,
        home_root: &home_root,
    };

    let mut outcome = binding::bind_project(&cwd, name, args.force, &deps)?;

    // US1.a: stubbed sync. US1.b replaces the body of
    // `sync_for_project_root` with the real dispatcher.
    let sync_outcome = harness::sync_for_project_root(&outcome.project_root, &deps)?;
    outcome.sync = Some(sync_outcome);

    emit(&outcome, mode)
}

/// Resolve the user's home directory for the dangerous-cwd check.
/// Errors with [`TomeError::Io`] if `$HOME` is unset — same shape as
/// [`Paths::resolve`].
fn resolve_home() -> Result<PathBuf, TomeError> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        TomeError::Io(std::io::Error::other(
            "HOME is not set — cannot decide whether the current directory is the user's home",
        ))
    })
}

fn emit(outcome: &BindOutcome, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(outcome),
        Mode::Json => write_json(outcome),
    }
}

fn emit_human(outcome: &BindOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if let Some(prior) = outcome.rebind_from.as_ref() {
        writeln!(
            out,
            "Rebound {} from `{}` to `{}`",
            outcome.project_root.display(),
            prior.as_str(),
            outcome.workspace.as_str(),
        )?;
    } else {
        writeln!(
            out,
            "Bound {} to workspace `{}`",
            outcome.project_root.display(),
            outcome.workspace.as_str(),
        )?;
    }
    if outcome.created_marker {
        writeln!(
            out,
            "  marker:    {}",
            Paths::project_marker_dir(&outcome.project_root).display(),
        )?;
    }
    Ok(())
}
