//! `tome workspace use [<name>]` CLI wrapper. The binding algorithm lives
//! in [`crate::workspace::binding`]; the shared "bind `$CWD` + harness
//! sync" sequence lives in [`crate::commands::workspace::bind_cwd_and_sync`]
//! (also used by `init --bind`). This module is the thin arg-validation +
//! emit layer.
//!
//! Issue #321 widens the surface three ways, all back-compatible:
//! - `--create` creates the workspace (create-if-absent) before binding —
//!   the `init` + `use` one-step ergonomic.
//! - the positional `<name>` is now optional; omitted on a terminal, an
//!   `inquire` picker over the existing workspaces resolves it; omitted on
//!   a non-terminal it refuses ([`TomeError::NotATerminal`], exit 54).
//! - `use <name>` (no `--create`) stays byte-identical to the pre-#321
//!   behaviour, exit codes and JSON included.
//!
//! Filename is `use_.rs` because `use` is a Rust keyword.

use crate::cli::WorkspaceUseArgs;
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::prompt;
use crate::workspace::binding::BindOutcome;
use crate::workspace::name::WorkspaceName;
use crate::workspace::sync::list_workspace_names;

use super::bind_cwd_and_sync;

pub fn run(
    args: WorkspaceUseArgs,
    global_workspace: Option<&str>,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Resolve the target workspace name. Three sources, in this order:
    //   1. explicit positional `<name>` (parse + honour --create),
    //   2. no name + --create → error (can't create an unnamed workspace),
    //   3. no name → interactive picker over the existing workspaces
    //      (refuses on a non-terminal, exit 54).
    let (name, created) = match args.name.as_deref() {
        Some(raw) => {
            log_global_flag_ignored(global_workspace, raw);
            let name = WorkspaceName::parse(raw)?;
            let created = if args.create {
                // All-or-nothing: run the dangerous-cwd guard BEFORE creating
                // the workspace so a refusal at $HOME / `/` (without --force)
                // leaves NO orphan created-but-unbound workspace behind. The
                // bind step re-checks this (idempotent defense-in-depth).
                super::guard_dangerous_cwd(args.force)?;
                create_if_absent(&name, paths)?
            } else {
                false
            };
            (name, created)
        }
        None => {
            if args.create {
                // A picker "create new" flow would need a name prompt too;
                // keep the contract simple — `--create` requires an explicit
                // name. Point the user at the non-interactive form.
                return Err(TomeError::Usage(
                    "workspace use --create requires an explicit <name>; \
                     run `tome workspace use --create <name>`"
                        .to_owned(),
                ));
            }
            (pick_workspace(paths)?, false)
        }
    };

    let mut outcome = bind_cwd_and_sync(name, args.force, paths)?;
    outcome.created = created;

    emit(&outcome, mode)
}

/// Emit the `tracing::debug!` note that the global `--workspace <name>`
/// flag is being ignored in favour of the positional. Only fires when both
/// are set and differ — the pre-#321 behaviour, preserved verbatim.
fn log_global_flag_ignored(global_workspace: Option<&str>, positional: &str) {
    if let Some(global) = global_workspace
        && global != positional
    {
        // `--workspace <name>` is a global clap flag. For every other
        // subcommand it picks the workspace; for `workspace use` it is
        // semantically nonsensical because the positional `<name>`
        // names the binding target. The Use arm always honours the
        // positional argument; the global flag is informational here.
        tracing::debug!(
            global_workspace = global,
            positional_name = positional,
            "workspace use: ignoring global --workspace; positional name wins",
        );
    }
}

/// `--create`: create the workspace via the SAME library call `workspace
/// init` uses ([`crate::workspace::init::init`]), tolerating an existing
/// workspace so the flag is idempotent + ergonomic ("init + bind in one
/// step"). Returns whether a workspace was actually created (`false` when
/// it already existed). Never inherits global catalogs — that is an
/// explicit `init --inherit-global` decision, out of scope for `use`.
fn create_if_absent(name: &WorkspaceName, paths: &Paths) -> Result<bool, TomeError> {
    match crate::workspace::init::init(name.clone(), false, paths) {
        Ok(_) => Ok(true),
        // Idempotent: an already-existing workspace is not an error under
        // `--create`; we fall through to bind it.
        Err(TomeError::WorkspaceAlreadyExists { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// A picker row. `WorkspaceName` has no `Display` impl (deliberately — it
/// is a validated newtype, not a UI type), so wrap it for `inquire::Select`.
struct WorkspaceChoice(WorkspaceName);

impl std::fmt::Display for WorkspaceChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// No-name picker: show an `inquire` `Select` over the existing workspaces
/// and return the chosen one. Refuses on a non-terminal (exit 54) with a
/// clear pointer at the non-interactive form — the same discipline the bare
/// `tome plugin` picker uses (FR-051).
fn pick_workspace(paths: &Paths) -> Result<WorkspaceName, TomeError> {
    let names = list_workspace_names(paths)?;
    if names.is_empty() {
        // Defensive: the DB always seeds `global`, so this should be
        // unreachable, but never present an empty picker.
        return Err(TomeError::Usage(
            "no workspaces to pick from; create one with `tome workspace init <name>`".to_owned(),
        ));
    }
    let choices: Vec<WorkspaceChoice> = names.into_iter().map(WorkspaceChoice).collect();
    // `prompt::select` refuses up front on a non-terminal → NotATerminal
    // (exit 54); it maps Esc/Ctrl-C to Interrupted (exit 8).
    let picked = prompt::select("Pick a workspace to bind", choices)?;
    Ok(picked.0)
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
    if outcome.created {
        // `--create` / `init --bind`: report the creation before the bind
        // line so the user sees both steps happened.
        writeln!(out, "Created workspace `{}`", outcome.workspace.as_str())?;
    }
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
            created: false,
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

    /// #321: `--create` / `init --bind` set `created` → the human output
    /// carries a leading "Created workspace" line before the bind line. The
    /// default (no create) omits it.
    #[test]
    fn human_output_created_line_gated_on_created_flag() {
        let mut oc = outcome(true, false);
        oc.created = true;
        let text = render(&oc);
        assert!(
            text.contains("Created workspace `my-workspace`"),
            "created line missing when created=true: {text}",
        );
        // The bind line still follows.
        assert!(text.contains("Bound"), "bind line missing: {text}");

        let plain = render(&outcome(true, false));
        assert!(
            !plain.contains("Created workspace"),
            "created line must be absent when created=false: {plain}",
        );
    }
}
