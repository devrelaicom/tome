//! `tome plugin disable <catalog>/<plugin> [<catalog>/<plugin> ...]`.
//!
//! Thin CLI wrapper over `plugin::lifecycle::disable`. The lock acquisition,
//! "already-disabled" detection, and atomic UPDATE all live in the library.
//! This module owns the confirmation-prompt UX (`--force` short-circuit,
//! non-TTY refusal with pointer message) and the human / JSON presentation
//! contract.
//!
//! Issue #314 widened this from a single `<catalog>/<plugin>` to a variadic
//! selection of ids, `*` wildcard globs, and a `--catalog` scope. Selection is
//! resolved once via `plugin::selector::resolve`; a single confirmation prompt
//! covers the whole batch; the per-plugin loop uses the `harness use`
//! forward-progress pattern. A single literal id behaves exactly as before.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin disable`"; issue #314.

use std::io::Write;

use serde::Serialize;
use tracing::info;

use crate::cli::PluginDisableArgs;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, DisableOutcome};
use crate::workspace::ResolvedScope;

use super::{discoverable_plugin_ids, open_index_for_read, registry_seeds, resolve_plugin_dir};

pub fn run(args: PluginDisableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    // ---- selection (issue #314) -------------------------------------------
    // Resolve variadic ids / globs / --catalog against the discoverable
    // candidate set. Slash-qualified literals are passed through without an
    // existence check here — the per-plugin `resolve_plugin_dir` owns that.
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let workspace_name = scope.scope.name();
    let candidates = discoverable_plugin_ids(&conn, &paths, workspace_name.as_str())?;
    drop(conn);
    let resolution =
        crate::plugin::selector::resolve(&args.ids, &candidates, args.catalog.as_deref());

    if resolution.matched.is_empty() {
        let first = resolution
            .errors
            .into_iter()
            .next()
            .map(crate::plugin::SelectorError::into_tome_error)
            .unwrap_or_else(|| TomeError::Usage("no plugins selected".to_owned()));
        return Err(first);
    }

    let mut first_error: Option<TomeError> = resolution
        .errors
        .into_iter()
        .next()
        .map(crate::plugin::SelectorError::into_tome_error);

    // ---- confirmation (ONE prompt for the whole batch) --------------------
    // `--force`/`prompt::non_interactive()` skip the prompt (as before). A
    // non-TTY without `--force` refuses with the documented pointer + exit 54
    // (`NotATerminal`). Interactive: a SINGLE resolved id keeps the exact
    // "Disable {id}?" string; MULTIPLE ids ask once naming them.
    if !args.force && !crate::presentation::prompt::non_interactive() {
        if !(output::stdin_is_tty() && output::stdout_is_tty()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "Disable requires confirmation. Re-run with --force to skip the prompt."
            );
            return Err(TomeError::NotATerminal);
        }

        let question = disable_prompt(&resolution.matched);
        if !crate::presentation::prompt::confirm(&question, false)? {
            // User declined — clean exit, no state change (matches the
            // interactive flow's "decline, no error" semantics).
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: disable declined.");
            }
            return Ok(());
        }
    }

    // ---- per-plugin forward-progress loop ---------------------------------
    // A plugin already disabled surfaces `PluginAlreadyInState` (exit 21). For a
    // batch/glob spanning mixed states we treat that as a benign skip; a SINGLE
    // explicit id still surfaces exit 21.
    let single_explicit = args.ids.len() == 1 && !args.ids[0].contains('*');
    let mut successes: Vec<DisableOutcome> = Vec::new();

    for id in &resolution.matched {
        match disable_one(id, &paths, scope, mode) {
            Ok(outcome) => successes.push(outcome),
            Err(TomeError::PluginAlreadyInState { plugin, state }) if !single_explicit => {
                info!(
                    plugin = %plugin,
                    state = ?state,
                    "plugin already disabled; skipping in batch disable",
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %id,
                    error = %e,
                    "plugin disable: plugin failed; continuing",
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    if successes.is_empty() {
        return match first_error {
            Some(e) => Err(e),
            None => {
                if mode == Mode::Human {
                    let mut out = std::io::stdout().lock();
                    writeln!(
                        out,
                        "Nothing to disable — all selected plugins already disabled."
                    )?;
                }
                Ok(())
            }
        };
    }

    // ---- batch-wide post-processing (ONCE) --------------------------------
    // FR-381 + FR-385: regenerate cached summaries AFTER the workspace_skills
    // rows are deleted — once for the whole batch.
    crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;

    // --sync (#280): propagate to bound harnesses inline, ONCE for the batch.
    let projects_synced = if args.sync {
        for outcome in &successes {
            emit_disable_success(&outcome.plugin, outcome, mode, true)?;
        }
        let report = crate::commands::sync::sync_bound_projects(scope.scope.name(), &paths)?;
        Some(report.projects.len())
    } else {
        None
    };

    match mode {
        Mode::Human => {
            if let Some(n) = projects_synced {
                super::emit_synced_confirmation(n)?;
            } else {
                for outcome in &successes {
                    emit_disable_success(&outcome.plugin, outcome, mode, false)?;
                }
            }
        }
        Mode::Json => {
            for outcome in &successes {
                emit_json(&outcome.plugin, outcome, projects_synced)?;
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Disable one plugin id: existence check + `lifecycle::disable`. Returns the
/// [`DisableOutcome`] on success. The confirmation prompt is owned by the
/// caller (ONE prompt for the whole batch), so this does no prompting.
fn disable_one(
    id: &PluginId,
    paths: &Paths,
    scope: &ResolvedScope,
    mode: Mode,
) -> Result<DisableOutcome, TomeError> {
    // Surface CatalogNotFound / PluginNotFound before mutating anything — a
    // typo on the address keeps the pre-#314 exit code.
    let conn = open_index_for_read(paths, &scope.scope)?;
    let _ = resolve_plugin_dir(id, &conn, scope.scope.name().as_str(), paths)?;
    drop(conn);

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();

    // Banner — human mode only. JSON stdout stays byte-stable.
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Disabling {}…", id)?;
    }

    let outcome = lifecycle::disable(
        id,
        paths,
        &scope.scope,
        embedder_seed,
        reranker_seed,
        summariser_seed,
    )?;

    crate::telemetry::emit(crate::telemetry::event::PluginActionEvent {
        action: crate::telemetry::event::PluginAction::Disabled,
    });

    // FR-052: ALONGSIDE the anonymous event above, emit the catalog-attributed
    // `catalog.<id>.plugin_disabled` ONLY when the plugin's catalog resolves —
    // by SOURCE, at emit time — to an allowlisted catalog. The version read is
    // still valid post-disable: disable removes the `workspace_skills` junction
    // but RETAINS the `skills` rows, so `plugin_version` is still in the index.
    // Best-effort throughout.
    if let Some(catalog_id) = crate::telemetry::resolve_attribution(scope, &id.catalog) {
        crate::telemetry::emit(crate::telemetry::event::PluginDisabled {
            catalog: catalog_id,
            plugin_name: id.plugin.clone(),
            plugin_version: super::attributed_plugin_version(paths, &scope.scope, id),
        });
    }

    Ok(outcome)
}

/// Build the confirmation question for a disable batch. A single resolved id
/// keeps the exact historical `"Disable {id}?"` string (so the single-id UX is
/// byte-identical); multiple ids ask once, naming them.
fn disable_prompt(ids: &[PluginId]) -> String {
    match ids {
        [only] => format!("Disable {only}?"),
        many => {
            let names: Vec<String> = many.iter().map(|id| id.to_string()).collect();
            format!("Disable {} plugins ({})?", many.len(), names.join(", "))
        }
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

    use super::{DisableRecord, disable_prompt, write_disable_human};
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

    /// #314: a single resolved id keeps the exact historical prompt string so
    /// the single-id UX is byte-identical.
    #[test]
    fn disable_prompt_single_id_is_historical_string() {
        let ids = vec![sample_id()];
        assert_eq!(disable_prompt(&ids), "Disable acme/widgets?");
    }

    /// #314: multiple ids ask once, naming them.
    #[test]
    fn disable_prompt_multiple_ids_names_them() {
        let ids = vec![
            PluginId::from_str("acme/a").unwrap(),
            PluginId::from_str("acme/b").unwrap(),
        ];
        assert_eq!(disable_prompt(&ids), "Disable 2 plugins (acme/a, acme/b)?");
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
