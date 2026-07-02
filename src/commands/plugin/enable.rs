//! `tome plugin enable <catalog>/<plugin> [<catalog>/<plugin> ...]`.
//!
//! Composes the `plugin::lifecycle::enable` orchestrator with the
//! model-presence prompt (T074 UI side) and the table / colour / NDJSON
//! presentation contract.
//!
//! Issue #314 widened this from a single `<catalog>/<plugin>` to a variadic
//! selection of ids, `*` wildcard globs, and a `--catalog` scope for bare/glob
//! names. Selection is resolved once via `plugin::selector::resolve` against the
//! discoverable candidate set; the one-time setup (paths / config / embedder /
//! drift guard / model-download prompt) is hoisted OUT of the per-plugin loop,
//! which then processes each matched id with the `harness use` forward-progress
//! pattern (a per-id failure is recorded + the loop continues; the FIRST
//! failure's exit code is the process exit). A single literal id behaves exactly
//! as before: one JSON record, the same human lines, the same exit codes.
//!
//! Spec: `contracts/plugin-commands.md` §1; issue #314.

use std::io::Write;

use serde::Serialize;
use tracing::info;

use crate::cli::PluginEnableArgs;
use crate::embedding::download::download_model;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::{colour, progress, prompt};
use crate::workspace::ResolvedScope;

use super::{
    discoverable_plugin_ids, human_mb, missing_models_for_profile, open_index_for_read,
    registry_seeds, resolve_plugin_dir,
};

pub fn run(args: PluginEnableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 reintroduces workspace-aware view.
    // LifecycleDeps.config is vestigial (the field is never read by the
    // lifecycle); use the default for that slot. Phase 12: the GLOBAL config is
    // loaded strictly (`cfg`) so the embedder can resolve remote-vs-bundled and
    // the drift guard compares the right identity — a malformed config is a
    // loud exit 5, not a silent fallback to bundled.
    let config = crate::config::Config::default();
    let cfg = crate::config::load(&paths)?;

    // ---- selection (issue #314) -------------------------------------------
    // Resolve the variadic ids / globs / --catalog against the discoverable
    // candidate set ONCE, before any model or index work. A slash-qualified
    // literal id is passed through without an existence check here — the
    // per-plugin `resolve_plugin_dir` below owns that (preserving the pre-#314
    // exit codes for a typo'd id).
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let workspace_name = scope.scope.name();
    let candidates = discoverable_plugin_ids(&conn, &paths, workspace_name.as_str())?;
    let resolution =
        crate::plugin::selector::resolve(&args.ids, &candidates, args.catalog.as_deref());

    // Nothing matched → the whole command fails loud with the FIRST mapped
    // error (a bad glob / ambiguous / not-found), never a silent success.
    if resolution.matched.is_empty() {
        let first = resolution
            .errors
            .into_iter()
            .next()
            .map(crate::plugin::SelectorError::into_tome_error)
            // Unreachable: clap requires ≥1 id, so an empty-match batch always
            // has ≥1 error. Fall back to a generic usage error defensively.
            .unwrap_or_else(|| TomeError::Usage("no plugins selected".to_owned()));
        drop(conn);
        return Err(first);
    }

    // Matched non-empty but some tokens failed → forward-progress: proceed with
    // the matches and remember the first error to surface at the end.
    let mut first_error: Option<TomeError> = resolution
        .errors
        .into_iter()
        .next()
        .map(crate::plugin::SelectorError::into_tome_error);

    // ---- one-time setup (hoisted out of the per-plugin loop) --------------
    // B4: resolve the ACTIVE profile's embedder (not the hard-coded default).
    let embedder_meta = crate::index::meta::active_embedder(&conn)?;

    // Phase 12 / US2: the drift-guard + meta seed must reflect the ACTIVE
    // embedder identity — remote (`"<provider>/<model>"`/`"external"`) when an
    // `[embedding]` provider is configured, else the active-profile registry
    // identity. So switching the embedding model HARD-fails this write path
    // (41/42) rather than landing mismatched vectors.
    let active_embedder_seed = crate::embedding::embedder_seed(&cfg, embedder_meta)?;

    // B3: refuse a partial re-embed under embedder drift BEFORE any model work.
    // Global to the batch — the embedder identity is plugin-independent, so it
    // runs ONCE for the whole selection.
    crate::index::meta::guard_embedder_drift(
        &conn,
        &crate::index::meta::ModelIdent {
            name: active_embedder_seed.name.clone(),
            version: active_embedder_seed.version.clone(),
        },
    )?;

    // Phase 12: is the embedder remote? On the remote path the local-model
    // download prompt is skipped (there is no embedder model to fetch — the
    // reranker is loaded lazily by `query`, not here).
    let remote_embedding =
        crate::provider::resolve(&cfg, crate::provider::Capability::Embedding)?.is_some();

    let persisted_dim = if remote_embedding {
        crate::index::read_embedder_dimension(&conn)?
    } else {
        None
    };
    drop(conn);

    // Model-presence handling — T074 UI side, ONCE for the whole batch (the
    // missing-model set is plugin-independent). Skipped on the remote path.
    if !remote_embedding {
        ensure_models_or_prompt(&paths, &args, mode)?;
    }

    // Construct the embedder ONCE: remote when `[embedding]` is configured, else
    // the bundled active-profile model. The drift guard above already proved the
    // configured embedder matches the stored identity.
    let embedder = crate::embedding::build_embedder(&cfg, &paths, embedder_meta, persisted_dim)?;

    let (_e_seed, reranker_seed, summariser_seed) = registry_seeds();

    // ---- per-plugin forward-progress loop ---------------------------------
    // Each matched id: existence check (resolve_plugin_dir) + lifecycle::enable
    // + --tier set. On a per-id Err, warn + continue, capturing the FIRST error.
    // A plugin already enabled surfaces `PluginAlreadyInState` (exit 21) — for
    // a glob that spans mixed states we treat that specific benign case as a
    // skip (no record, no error captured) so a wildcard re-run over an
    // already-enabled plugin isn't a fatal error; an EXPLICIT single-id enable
    // of an already-enabled plugin still surfaces exit 21 (see below).
    let single_explicit = args.ids.len() == 1 && !args.ids[0].contains('*');
    let mut successes: Vec<lifecycle::EnableOutcome> = Vec::new();

    for id in &resolution.matched {
        match enable_one(
            id,
            &paths,
            scope,
            &config,
            embedder.as_ref(),
            active_embedder_seed.clone(),
            reranker_seed.clone(),
            summariser_seed.clone(),
            args.tier,
            mode,
        ) {
            Ok(outcome) => successes.push(outcome),
            Err(TomeError::PluginAlreadyInState { plugin, state }) if !single_explicit => {
                // Benign for a batch/glob: the plugin is already in the desired
                // state, so skip it without failing the whole run. A single
                // explicit id falls through to the general arm below (exit 21).
                info!(
                    plugin = %plugin,
                    state = ?state,
                    "plugin already enabled; skipping in batch enable",
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %id,
                    error = %e,
                    "plugin enable: plugin failed; continuing",
                );
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    // If nothing succeeded, surface the first error (loud, non-zero exit) and
    // emit no records. `first_error` is Some in that case (either a selector
    // error or the first per-plugin failure).
    if successes.is_empty() {
        return match first_error {
            Some(e) => Err(e),
            // Every matched id was a benign already-enabled skip (glob path):
            // nothing to do, clean success. Note it in human mode.
            None => {
                if mode == Mode::Human {
                    let mut out = std::io::stdout().lock();
                    writeln!(
                        out,
                        "Nothing to enable — all selected plugins already enabled."
                    )?;
                }
                Ok(())
            }
        };
    }

    // ---- batch-wide post-processing (ONCE) --------------------------------
    // FR-380 + FR-385: regenerate cached summaries AFTER the workspace_skills
    // mutations commit — once for the whole batch, not per-id.
    crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;

    // --sync (#280): propagate the change to bound harnesses inline, ONCE for
    // the whole batch, reusing the SAME path `tome sync --all` uses. It runs
    // AFTER the success lines print (so partial progress is visible even if the
    // sync errors).
    let projects_synced = if args.sync {
        // Print each success line NOW so the user always sees the enables landed.
        for outcome in &successes {
            emit_enable_success(&outcome.plugin, outcome, mode, true)?;
        }
        let report = crate::commands::sync::sync_bound_projects(scope.scope.name(), &paths)?;
        Some(report.projects.len())
    } else {
        None
    };

    // ---- emit --------------------------------------------------------------
    match mode {
        Mode::Human => {
            if let Some(n) = projects_synced {
                // Success lines already printed above; confirm what was applied.
                super::emit_synced_confirmation(n)?;
            } else {
                // Per-plugin success lines, then ONE batch `next:` reminder.
                for outcome in &successes {
                    emit_enable_success(&outcome.plugin, outcome, mode, false)?;
                }
            }
        }
        Mode::Json => {
            // NDJSON: one record per SUCCESSFULLY-processed plugin (exactly like
            // `plugin list`). A single successful id ⇒ one object ⇒ byte-identical
            // to the pre-#314 record. When `--sync`, the batch count trails EACH
            // record (single-id `--sync` pin stays green).
            for outcome in &successes {
                emit_json(&outcome.plugin, outcome, projects_synced)?;
            }
        }
    }

    // Forward-progress: records emitted, now surface the first failure's code.
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Enable a single plugin id: existence check + `lifecycle::enable` + optional
/// `--tier` bulk-set. Returns the [`lifecycle::EnableOutcome`] on success. The
/// one-time setup (embedder, seeds, drift guard, model prompt) is done by the
/// caller and passed in, so this is called once per matched id.
///
/// `mode` drives only the per-plugin banner (`Enabling {id}…`); the success
/// lines + JSON records are emitted by the caller after the loop.
#[allow(clippy::too_many_arguments)]
fn enable_one(
    id: &PluginId,
    paths: &Paths,
    scope: &ResolvedScope,
    config: &crate::config::Config,
    embedder: &dyn crate::embedding::Embedder,
    embedder_seed: crate::index::MetaSeed,
    reranker_seed: crate::index::MetaSeed,
    summariser_seed: crate::index::MetaSeed,
    tier: Option<u8>,
    mode: Mode,
) -> Result<lifecycle::EnableOutcome, TomeError> {
    // Pre-check catalog + plugin existence so a typo surfaces the right exit
    // code (CatalogNotFound / PluginNotFound). Lifecycle re-checks this
    // internally; this cheap probe keeps the exit-code surface identical to the
    // pre-#314 single-id path.
    let conn = open_index_for_read(paths, &scope.scope)?;
    let _ = resolve_plugin_dir(id, &conn, scope.scope.name().as_str(), paths)?;
    drop(conn);

    let deps = LifecycleDeps {
        paths,
        scope: &scope.scope,
        config,
        embedder,
        embedder_seed,
        reranker_seed: reranker_seed.clone(),
        summariser_seed: summariser_seed.clone(),
        allow_model_download: false,
    };

    // Banner — human mode only. Skipping it in JSON keeps stdout byte-stable;
    // the structured record is the contract there.
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Enabling {}…", id)?;
    }

    let outcome = lifecycle::enable(id, &deps)?;

    // Warnings on stderr — one per line, prefixed.
    if !outcome.warnings.is_empty() {
        let mut err = std::io::stderr().lock();
        for w in &outcome.warnings {
            let _ = writeln!(err, "{} {}", colour::warning("warning:"), w);
        }
    }

    // --tier bulk-sets every skill/command of this plugin to the requested
    // routing tier. The UPDATE runs under the advisory write lock on a
    // writable connection (FR-040) — same idiom as `commands/tier/set.rs`.
    // We apply this BEFORE regenerate_for_trigger (in the caller) so RULES.md is
    // recomposed with the new tiers.
    if let Some(tier) = tier {
        let (e_seed, r_seed, s_seed) = registry_seeds();
        let tier_conn = crate::index::open(
            &paths.index_db,
            &crate::index::OpenOptions {
                embedder: e_seed,
                reranker: r_seed,
                summariser: s_seed,
                profile: None,
            },
        )?;
        let tier_lock = crate::index::acquire_lock(&paths.index_lock)?;

        let tier_result = crate::index::skills::set_tier_for_plugin(
            &tier_conn,
            scope.scope.name().as_str(),
            &id.catalog,
            &id.plugin,
            tier,
        );

        match tier_result {
            Ok(_) => {
                tier_lock.release()?;
            }
            Err(e) => {
                drop(tier_lock);
                return Err(e);
            }
        }
    }

    crate::telemetry::emit(crate::telemetry::event::PluginActionEvent {
        action: crate::telemetry::event::PluginAction::Enabled,
    });

    // FR-052: ALONGSIDE the anonymous event above, emit the catalog-attributed
    // `catalog.<id>.plugin_enabled` ONLY when the plugin's catalog resolves — at
    // emit time, by SOURCE not name — to an allowlisted catalog. Best-effort
    // throughout: the attribution + version reads are read-only, never lock,
    // never fail the command.
    if let Some(catalog_id) = crate::telemetry::resolve_attribution(scope, &id.catalog) {
        crate::telemetry::emit(crate::telemetry::event::PluginEnabled {
            catalog: catalog_id,
            plugin_name: id.plugin.clone(),
            plugin_version: super::attributed_plugin_version(paths, &scope.scope, id),
        });
    }

    Ok(outcome)
}

/// Print the human success line + `next:` reminder (or, in JSON mode, nothing —
/// the structured record is the contract there). `sync_will_run` suppresses the
/// "or `tome sync` to apply" clause when `--sync` is about to apply the change
/// itself (avoids a contradictory "already applied / run tome sync" pair).
fn emit_enable_success(
    id: &PluginId,
    outcome: &lifecycle::EnableOutcome,
    mode: Mode,
    sync_will_run: bool,
) -> Result<(), TomeError> {
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        write_enable_human(&mut out, id, outcome, sync_will_run)?;
    }
    Ok(())
}

/// Probe each model's on-disk manifest and prompt the user for a download
/// when anything is missing. Returns `Ok(())` once every required model is
/// present (or the user said yes and the downloads finished).
///
/// Refusal contract:
/// * `--yes` skips the prompt and proceeds.
/// * On a TTY the user is asked once for ALL missing models.
/// * Off a TTY without `--yes` → `ModelMissing` (exit 30).
/// * User says no → `Interrupted` (exit 8), the "user-initiated abort" code.
///
/// Runs ONCE for the whole batch — the missing-model set is plugin-independent.
fn ensure_models_or_prompt(
    paths: &Paths,
    args: &PluginEnableArgs,
    mode: Mode,
) -> Result<(), TomeError> {
    // B2: only prompt for the ACTIVE profile's embedder + reranker, not every
    // profile's models in the registry. Resolve the active profile from the
    // index `meta`; on a fresh install (no DB yet) fall back to the default
    // profile, which is exactly what the bootstrap will stamp.
    let missing = if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        missing_models_for_profile(paths, &conn)?
    } else {
        use crate::embedding::profile::{Profile, embedder_for, reranker_for};
        [
            embedder_for(Profile::DEFAULT),
            reranker_for(Profile::DEFAULT),
        ]
        .into_iter()
        .filter(|e| !super::model_manifest_ok(paths, e))
        .collect::<Vec<_>>()
    };
    if missing.is_empty() {
        return Ok(());
    }

    // `prompt::non_interactive()` also reads a persistently-exported
    // `TOME_NONINTERACTIVE=1`, so under that env this model-download prompt (and
    // every other prompt, including the `tome plugin` TUI's) auto-confirms —
    // intended per the "env auto-confirms every prompt" semantics.
    let confirmed = if args.yes || prompt::non_interactive() {
        true
    } else if output::stdin_is_tty() && output::stdout_is_tty() {
        let mut prompt_text = String::from("Tome needs to download:\n");
        let mut total: u64 = 0;
        for entry in &missing {
            prompt_text.push_str(&format!(
                "  - {} (~{}, {}) — {:?}\n",
                entry.name,
                human_mb(entry.size_bytes),
                entry.licence,
                entry.kind,
            ));
            total = total.saturating_add(entry.size_bytes);
        }
        prompt_text.push_str(&format!("Total: ~{}\nProceed?", human_mb(total)));
        prompt::confirm(&prompt_text, true)?
    } else {
        return Err(TomeError::ModelMissing {
            model: missing[0].name.to_owned(),
        });
    };

    if !confirmed {
        if mode == Mode::Human {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "Aborted: model download declined.");
        }
        return Err(TomeError::Interrupted);
    }

    // Download each missing model. We wrap each in an indeterminate spinner.
    // F6 added a byte-progress callback to `download_model`; this site keeps
    // the spinner (matches the precedent for the embedder + reranker).
    for entry in missing {
        let pb = progress::spinner(format!(
            "downloading {} (~{})",
            entry.name,
            human_mb(entry.size_bytes)
        ));
        let result = download_model(entry, &paths.models_dir, None);
        pb.finish_and_clear();
        result?;
        info!(model = entry.name, "model artefact installed");
    }

    Ok(())
}

/// Write the human-mode success lines for `plugin enable` to `out`.
///
/// Split out from the locked-stdout emit path so the `next:` onboarding hint
/// (#281) is unit-testable against an in-memory sink without a real index /
/// model download — the `write<W: Write>` seam already used by `plugin show`'s
/// `write_entry_line`.
///
/// `sync_will_run` (#280): when `--sync` will apply the change inline, the
/// "or `tome sync` to apply to your harnesses" clause is suppressed so the
/// output never contradicts itself (telling the user to run a sync that is
/// already running).
fn write_enable_human<W: Write>(
    out: &mut W,
    id: &PluginId,
    outcome: &lifecycle::EnableOutcome,
    sync_will_run: bool,
) -> std::io::Result<()> {
    let secs = outcome.duration.as_secs_f64();
    writeln!(
        out,
        "{} {} skills indexed ({} newly embedded) in {:.1}s",
        colour::success("✓"),
        outcome.summary.total_skills,
        outcome.summary.newly_embedded,
        secs,
    )?;
    // Onboarding step hint (#281) — human mode only, mirroring the
    // `workspace init` `next:` line. Points the user at searching the freshly
    // indexed skills and, unless `--sync` is applying it for them, at
    // propagating the change to bound harnesses.
    if sync_will_run {
        writeln!(
            out,
            "  next:     `tome query <text>` to search these skills"
        )?;
    } else {
        writeln!(
            out,
            "  next:     `tome query <text>` to search these skills, or `tome sync` to apply to your harnesses",
        )?;
    }
    let _ = id; // referenced for consistency / future formatting
    Ok(())
}

#[derive(Serialize)]
struct EnableRecord<'a> {
    plugin: String,
    status: &'a str,
    skills_indexed: u32,
    skills_newly_embedded: u32,
    duration_ms: u64,
    /// #280: number of bound projects the inline `--sync` propagated to.
    /// Absent (omitted from the wire shape) when `--sync` was not passed, so
    /// the byte-stable JSON pin for the no-`--sync` path is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    projects_synced: Option<usize>,
}

fn emit_json(
    id: &PluginId,
    outcome: &lifecycle::EnableOutcome,
    projects_synced: Option<usize>,
) -> Result<(), TomeError> {
    let duration_ms = outcome.duration.as_millis().min(u128::from(u64::MAX)) as u64;
    let record = EnableRecord {
        plugin: id.to_string(),
        status: "enabled",
        skills_indexed: outcome.summary.total_skills,
        skills_newly_embedded: outcome.summary.newly_embedded,
        duration_ms,
        projects_synced,
    };
    output::write_json(&record)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use super::write_enable_human;
    use crate::index::skills::EnableSummary;
    use crate::plugin::PluginId;
    use crate::plugin::lifecycle::EnableOutcome;

    fn sample_outcome() -> EnableOutcome {
        EnableOutcome {
            plugin: PluginId::from_str("acme/widgets").expect("valid id"),
            summary: EnableSummary {
                total_skills: 3,
                newly_embedded: 3,
            },
            duration: Duration::from_millis(1234),
            warnings: Vec::new(),
        }
    }

    /// #281: the human success output carries the onboarding `next:` hint and
    /// every command it names actually exists on the CLI surface. Without
    /// `--sync` (`sync_will_run == false`) the hint still points at `tome sync`.
    #[test]
    fn human_output_includes_onboarding_next_hint() {
        let outcome = sample_outcome();
        let mut buf: Vec<u8> = Vec::new();
        write_enable_human(&mut buf, &outcome.plugin, &outcome, false).expect("write");
        let text = String::from_utf8(buf).expect("utf8");

        assert!(
            text.contains("skills indexed"),
            "success line missing: {text}"
        );
        assert!(text.contains("next:"), "onboarding hint missing: {text}");
        assert!(
            text.contains("tome query"),
            "`tome query` not referenced: {text}",
        );
        assert!(
            text.contains("tome sync"),
            "`tome sync` not referenced: {text}",
        );
    }

    /// #280: when `--sync` will apply the change inline (`sync_will_run ==
    /// true`), the "or `tome sync` to apply" reminder is suppressed — the
    /// output must not tell the user to run a sync that is already running.
    #[test]
    fn human_output_suppresses_sync_reminder_when_sync_will_run() {
        let outcome = sample_outcome();
        let mut buf: Vec<u8> = Vec::new();
        write_enable_human(&mut buf, &outcome.plugin, &outcome, true).expect("write");
        let text = String::from_utf8(buf).expect("utf8");

        // The success line + a `next:` hint still print...
        assert!(
            text.contains("skills indexed"),
            "success line missing: {text}"
        );
        assert!(text.contains("next:"), "onboarding hint missing: {text}");
        assert!(
            text.contains("tome query"),
            "`tome query` not referenced: {text}",
        );
        // ...but the `tome sync` reminder is gone (it is being applied inline).
        assert!(
            !text.contains("tome sync"),
            "`tome sync` reminder must be suppressed when --sync runs: {text}",
        );
    }

    /// #281: the `--json` success record has no `next` field — the hint is
    /// human-mode only. #280: with no `--sync`, `projects_synced` is `None` and
    /// OMITTED, so the wire shape is byte-identical to the pre-#280 record.
    /// Full-string pin (not `contains`) so a field reorder / accidental
    /// inclusion is caught (mirrors disable's pins).
    #[test]
    fn json_record_has_no_next_hint() {
        let outcome = sample_outcome();
        let record = super::EnableRecord {
            plugin: outcome.plugin.to_string(),
            status: "enabled",
            skills_indexed: outcome.summary.total_skills,
            skills_newly_embedded: outcome.summary.newly_embedded,
            duration_ms: outcome.duration.as_millis() as u64,
            projects_synced: None,
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert_eq!(
            json,
            r#"{"plugin":"acme/widgets","status":"enabled","skills_indexed":3,"skills_newly_embedded":3,"duration_ms":1234}"#,
        );
    }

    /// #280: with `--sync`, the JSON record carries the additive
    /// `projects_synced` count as the LAST field. Full-string byte-stable pin
    /// enforces the field order (trailing `projects_synced`) so a reorder
    /// regression fails the test.
    #[test]
    fn json_record_carries_projects_synced_with_sync() {
        let outcome = sample_outcome();
        let record = super::EnableRecord {
            plugin: outcome.plugin.to_string(),
            status: "enabled",
            skills_indexed: outcome.summary.total_skills,
            skills_newly_embedded: outcome.summary.newly_embedded,
            duration_ms: outcome.duration.as_millis() as u64,
            projects_synced: Some(2),
        };
        let json = serde_json::to_string(&record).expect("serialize");
        assert_eq!(
            json,
            r#"{"plugin":"acme/widgets","status":"enabled","skills_indexed":3,"skills_newly_embedded":3,"duration_ms":1234,"projects_synced":2}"#,
        );
    }
}
