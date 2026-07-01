//! `tome plugin disable <catalog>/<plugin>`.
//!
//! Thin CLI wrapper over `plugin::lifecycle::disable`. The lock acquisition,
//! "already-disabled" detection, and atomic UPDATE all live in the library.
//! This module owns the confirmation-prompt UX (`--force` short-circuit,
//! non-TTY refusal with pointer message) and the human / JSON presentation
//! contract.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin disable`".

use std::io::Write;
use std::str::FromStr;

use serde::Serialize;

use crate::cli::PluginDisableArgs;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, DisableOutcome};
use crate::workspace::ResolvedScope;

use super::{open_index_for_read, registry_seeds, resolve_plugin_dir};

pub fn run(args: PluginDisableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;
    let paths = Paths::resolve()?;

    // Surface CatalogNotFound / PluginNotFound before any prompt — typo on
    // the address shouldn't waste the user's "y" keystroke. Resolution reads
    // the catalog enrolment from the DB (F11b).
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let _ = resolve_plugin_dir(&id, &conn, scope.scope.name().as_str(), &paths)?;
    drop(conn);

    if !args.force {
        // Non-TTY without --force → exit 54 per FR-007 / FR-051. We emit the
        // documented pointer line to stderr before returning so the user
        // sees specific guidance, not just the generic NotATerminal Display.
        // (Same defensive pattern as the interactive flow — P4 retro
        // §"NotATerminal message duplication".)
        if !(output::stdin_is_tty() && output::stdout_is_tty()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "Disable requires confirmation. Re-run with --force to skip the prompt."
            );
            return Err(TomeError::NotATerminal);
        }

        // TTY: ask. Default no per contract step 3.
        if !crate::presentation::prompt::confirm(&format!("Disable {id}?"), false)? {
            // User declined — clean exit, no state change. Surfacing this as
            // Ok(()) matches the interactive flow's "decline, no error"
            // semantics. We emit a stderr note in human mode for parity with
            // the model-download decline path.
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: disable declined.");
            }
            return Ok(());
        }
    }

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();

    // Banner — human mode only. JSON stdout stays byte-stable.
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Disabling {}…", id)?;
    }

    let outcome = lifecycle::disable(
        &id,
        &paths,
        &scope.scope,
        embedder_seed,
        reranker_seed,
        summariser_seed,
    )?;

    // FR-381 + FR-385: regenerate cached summaries AFTER the
    // workspace_skills DELETE commits. See `commands::plugin::enable`
    // for the forward-progress invariant.
    crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;

    crate::telemetry::emit(crate::telemetry::event::PluginActionEvent {
        action: crate::telemetry::event::PluginAction::Disabled,
    });

    // FR-052: ALONGSIDE the anonymous event above, emit the catalog-attributed
    // `catalog.<id>.plugin_disabled` ONLY when the plugin's catalog resolves —
    // by SOURCE, at emit time — to an allowlisted catalog. The version read is
    // still valid post-disable: disable removes the `workspace_skills` junction
    // but RETAINS the `skills` rows (hence `skills_retained`), so the plugin's
    // `plugin_version` is still in the index. Best-effort throughout.
    if let Some(catalog_id) = crate::telemetry::resolve_attribution(scope, &id.catalog) {
        crate::telemetry::emit(crate::telemetry::event::PluginDisabled {
            catalog: catalog_id,
            plugin_name: id.plugin.clone(),
            plugin_version: super::attributed_plugin_version(&paths, &scope.scope, &id),
        });
    }

    // --sync (#280): propagate the change to bound harnesses inline, reusing the
    // SAME path `tome sync --all` uses (`commands::sync::sync_bound_projects` →
    // `sync_all` → `sync_project`), so this inherits every writer safety and the
    // forward-progress fan-out. It runs AFTER the disable + summary trigger have
    // committed: a sync failure here surfaces the underlying `sync_project` exit
    // code but the disable itself is already durable and IS reported first (the
    // success line prints before we propagate the error).
    let projects_synced = if args.sync {
        // Print the disable success line NOW so the user always sees that the
        // disable landed, even if the follow-up sync errors out below.
        emit_disable_success(&id, &outcome, mode, true)?;
        let report = crate::commands::sync::sync_bound_projects(scope.scope.name(), &paths)?;
        Some(report.projects.len())
    } else {
        None
    };

    match (mode, projects_synced) {
        // --sync succeeded: the success line already printed above; now confirm
        // what was applied (human) / carry the count (json).
        (Mode::Human, Some(n)) => emit_synced_confirmation(n),
        (Mode::Json, Some(n)) => emit_json(&id, &outcome, Some(n)),
        // No --sync: normal success emit with the "run `tome sync`" reminder.
        (Mode::Human, None) => emit_disable_success(&id, &outcome, mode, false),
        (Mode::Json, None) => emit_json(&id, &outcome, None),
    }
}

/// Print the human success line + `next:` reminder (or, in JSON mode, nothing —
/// the structured record is the contract there). `sync_will_run` suppresses the
/// "run `tome sync`" reminder when `--sync` is about to apply the change itself.
fn emit_disable_success(
    id: &PluginId,
    outcome: &DisableOutcome,
    mode: Mode,
    sync_will_run: bool,
) -> Result<(), TomeError> {
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        write_disable_human(&mut out, id, outcome, sync_will_run)?;
    }
    Ok(())
}

/// Write the human-mode success lines for `plugin disable` to `out`.
///
/// Uses the `write<W: Write>` seam so the `next:` reminder (#280) is testable
/// against an in-memory sink. `sync_will_run` suppresses the "run `tome sync`"
/// clause when `--sync` is applying the change inline (avoids telling the user
/// to run a sync that is already running).
fn write_disable_human<W: Write>(
    out: &mut W,
    id: &PluginId,
    outcome: &DisableOutcome,
    sync_will_run: bool,
) -> std::io::Result<()> {
    writeln!(
        out,
        "{} disabled {} ({} skill records retained)",
        crate::presentation::colour::success("✓"),
        id,
        outcome.skills_retained,
    )?;
    // #280: disable previously dead-ended with no follow-up guidance. Mirror
    // the enable `next:` line — point the user at applying the change to bound
    // harnesses, unless `--sync` is applying it for them.
    if !sync_will_run {
        writeln!(out, "  next:     `tome sync` to apply to your harnesses",)?;
    }
    Ok(())
}

/// Human-mode confirmation printed after a successful `--sync`: mirrors
/// `tier set`'s post-sync "applied" messaging rather than the "run `tome sync`"
/// reminder.
fn emit_synced_confirmation(projects: usize) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "  synced:   applied to harnesses in {} bound project(s)",
        projects,
    )?;
    Ok(())
}

#[derive(Serialize)]
struct DisableRecord {
    plugin: String,
    status: &'static str,
    skills_retained: u32,
    /// #280: number of bound projects the inline `--sync` propagated to.
    /// Absent (omitted) when `--sync` was not passed, so the byte-stable JSON
    /// pin for the no-`--sync` path is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    projects_synced: Option<usize>,
}

fn emit_json(
    id: &PluginId,
    outcome: &DisableOutcome,
    projects_synced: Option<usize>,
) -> Result<(), TomeError> {
    let record = DisableRecord {
        plugin: id.to_string(),
        status: "disabled",
        skills_retained: outcome.skills_retained,
        projects_synced,
    };
    output::write_json(&record)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use super::{DisableRecord, write_disable_human};
    use crate::plugin::PluginId;
    use crate::plugin::lifecycle::DisableOutcome;

    fn sample_id() -> PluginId {
        PluginId::from_str("acme/widgets").expect("valid id")
    }

    fn sample_outcome() -> DisableOutcome {
        DisableOutcome {
            plugin: sample_id(),
            skills_retained: 4,
            duration: Duration::from_millis(500),
        }
    }

    /// #280: without `--sync`, the disable human output carries a `next:`
    /// reminder pointing at `tome sync` (previously it dead-ended).
    #[test]
    fn human_output_includes_sync_reminder() {
        let mut buf: Vec<u8> = Vec::new();
        write_disable_human(&mut buf, &sample_id(), &sample_outcome(), false).expect("write");
        let text = String::from_utf8(buf).expect("utf8");

        assert!(text.contains("disabled"), "success line missing: {text}");
        assert!(text.contains("next:"), "reminder hint missing: {text}");
        assert!(
            text.contains("tome sync"),
            "`tome sync` not referenced: {text}",
        );
    }

    /// #280: when `--sync` will apply the change inline, the "run `tome sync`"
    /// reminder is suppressed so the output never contradicts itself.
    #[test]
    fn human_output_suppresses_sync_reminder_when_sync_will_run() {
        let mut buf: Vec<u8> = Vec::new();
        write_disable_human(&mut buf, &sample_id(), &sample_outcome(), true).expect("write");
        let text = String::from_utf8(buf).expect("utf8");

        assert!(text.contains("disabled"), "success line missing: {text}");
        assert!(
            !text.contains("tome sync"),
            "`tome sync` reminder must be suppressed when --sync runs: {text}",
        );
    }

    /// #280: with no `--sync`, `projects_synced` is `None` and omitted, so the
    /// wire shape is byte-identical to the pre-#280 record.
    #[test]
    fn json_record_omits_projects_synced_without_sync() {
        let record = DisableRecord {
            plugin: sample_id().to_string(),
            status: "disabled",
            skills_retained: 4,
            projects_synced: None,
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert_eq!(
            json,
            r#"{"plugin":"acme/widgets","status":"disabled","skills_retained":4}"#,
        );
    }

    /// #280: with `--sync`, the JSON record carries the additive
    /// `projects_synced` count as a trailing field.
    #[test]
    fn json_record_carries_projects_synced_with_sync() {
        let record = DisableRecord {
            plugin: sample_id().to_string(),
            status: "disabled",
            skills_retained: 4,
            projects_synced: Some(3),
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert_eq!(
            json,
            r#"{"plugin":"acme/widgets","status":"disabled","skills_retained":4,"projects_synced":3}"#,
        );
    }
}
