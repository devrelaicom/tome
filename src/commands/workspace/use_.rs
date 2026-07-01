//! `tome workspace use <name>` CLI wrapper. The binding algorithm lives
//! in [`crate::workspace::binding`]; the harness sync seam lives in
//! [`crate::commands::harness`]. This module is the thin arg-validation
//! + emit layer.
//!
//! Filename is `use_.rs` because `use` is a Rust keyword.

use std::path::PathBuf;

use crate::cli::WorkspaceUseArgs;
use crate::commands::harness;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::binding::{self, BindDeps, BindOutcome};
use crate::workspace::name::WorkspaceName;

pub fn run(
    args: WorkspaceUseArgs,
    global_workspace: Option<&str>,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let name = WorkspaceName::parse(&args.name)?;

    if let Some(global) = global_workspace
        && global != args.name
    {
        // `--workspace <name>` is a global clap flag. For every other
        // subcommand it picks the workspace; for `workspace use` it is
        // semantically nonsensical because the positional `<name>`
        // names the binding target. The Use arm always honours the
        // positional argument; the global flag is informational here.
        tracing::debug!(
            global_workspace = global,
            positional_name = args.name.as_str(),
            "workspace use: ignoring global --workspace; positional name wins",
        );
    }

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

    // Phase B of the bind algorithm: run the harness sync orchestrator
    // against the freshly-bound workspace name. `--force` is forwarded
    // so user-owned `tome` MCP entries get rewritten instead of
    // returning HarnessClash (exit 19).
    let sync_outcome = harness::sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &deps,
        args.force,
    )?;
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
    write_use_human(&mut out, outcome)?;
    Ok(())
}

/// Write the human-mode success lines for `workspace use` to `out`.
///
/// Split out from [`emit_human`] (which owns the locked stdout) so the
/// `next:` onboarding hint (#281) is unit-testable against an in-memory sink,
/// matching the `write<W: Write>` seam used by `plugin show`/`plugin enable`.
fn write_use_human<W: std::io::Write>(out: &mut W, outcome: &BindOutcome) -> std::io::Result<()> {
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
    // Onboarding step hint (#281) — human mode only, mirroring the
    // `workspace init` `next:` line. A freshly-bound workspace has no
    // catalogs until one is added; point the user there.
    writeln!(
        out,
        "  next:      `tome catalog add <source>` to enrol a catalog in workspace `{}`",
        outcome.workspace.as_str(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BindOutcome, write_use_human};
    use crate::workspace::name::WorkspaceName;

    fn outcome(created_marker: bool, rebind: bool) -> BindOutcome {
        BindOutcome {
            workspace: WorkspaceName::parse("my-workspace").expect("valid name"),
            project_root: std::path::PathBuf::from("/tmp/project"),
            created_marker,
            rebind_from: rebind.then(|| WorkspaceName::parse("old-ws").expect("valid name")),
            sync: None,
        }
    }

    fn render(outcome: &BindOutcome) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write_use_human(&mut buf, outcome).expect("write");
        String::from_utf8(buf).expect("utf8")
    }

    /// #281: the human success output carries the onboarding `next:` hint,
    /// pointing at `tome catalog add`, on both first-bind and rebind.
    #[test]
    fn human_output_includes_onboarding_next_hint() {
        for (created, rebind) in [(true, false), (false, false), (true, true)] {
            let text = render(&outcome(created, rebind));
            assert!(text.contains("next:"), "onboarding hint missing: {text}");
            assert!(
                text.contains("tome catalog add"),
                "`tome catalog add` not referenced: {text}",
            );
        }
    }
}
