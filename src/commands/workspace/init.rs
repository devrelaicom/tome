//! `tome workspace init <name>` CLI wrapper.
//!
//! The atomic DB + filesystem work lives in [`crate::workspace::init`];
//! this module is the thin arg-validation + emit layer per the silent-
//! compute / emit-wrapper pattern documented on CLAUDE.md.
//!
//! Issue #321 adds `--bind`: the mirror of `workspace use --create`. After
//! the workspace is created (and inherit-global seeding), `--bind` binds
//! `$CWD` to the new workspace via the SAME shared path `use` runs
//! ([`crate::commands::workspace::bind_cwd_and_sync`]) — one bind+sync SSOT
//! rather than a duplicated sequence.

use std::io::Write;

use crate::cli::WorkspaceInitArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, InitOutcome, WorkspaceName};

use super::bind_cwd_and_sync;

pub fn run(args: WorkspaceInitArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;
    // All-or-nothing for `init --bind`: run the dangerous-cwd guard BEFORE
    // `init::init` creates the workspace, so `tome workspace init foo --bind`
    // at $HOME / `/` refuses (exit 2) WITHOUT creating an orphan. `init` has
    // no `--force`, so the effective bind force is always false. Plain `init`
    // (no `--bind`) is UNCHANGED — never guarded; it may run at any CWD.
    if args.bind {
        super::guard_dangerous_cwd(false)?;
    }
    let mut outcome = workspace::init::init(name, args.inherit_global, paths)?;
    if args.bind {
        // Mirror of `use --create`: bind `$CWD` to the freshly-created
        // workspace through the shared bind+sync path. `force` is false —
        // `init --bind` is not a "bind a dangerous CWD" escape hatch; the
        // home/`/` refusal still applies (rerun `workspace use --force` if
        // that is genuinely intended). The bind step re-checks the guard
        // (idempotent defense-in-depth).
        let _ = bind_cwd_and_sync(outcome.name.clone(), false, paths)?;
        outcome.bound = true;
    }
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
        outcome.path.display(),
    )?;
    if outcome.catalogs_inherited > 0 {
        writeln!(
            out,
            "  catalogs: {} (inherited from global)",
            outcome.catalogs_inherited,
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
    if outcome.bound {
        // `--bind`: the current directory is already bound, so the "next:
        // bind a project" hint no longer applies. Report the bind instead.
        writeln!(out, "  bound:    current directory")?;
    } else {
        writeln!(
            out,
            "  next:     `cd <project> && tome workspace use {}` to bind a project",
            outcome.name.as_str(),
        )?;
    }
    Ok(())
}
