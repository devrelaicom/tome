//! `tome plugin enable <catalog>/<plugin>`.
//!
//! Composes the `plugin::lifecycle::enable` orchestrator with the
//! model-presence prompt (T074 UI side) and the table / colour / NDJSON
//! presentation contract.
//!
//! Spec: `contracts/plugin-commands.md` §1.

use std::io::Write;
use std::str::FromStr;

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
    human_mb, missing_models_for_profile, open_index_for_read, registry_seeds, resolve_plugin_dir,
};

pub fn run(args: PluginEnableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 reintroduces workspace-aware view.
    // LifecycleDeps.config is vestigial (the field is never read by the
    // lifecycle); use the default for that slot. Phase 12: the GLOBAL config is
    // loaded strictly (`cfg`) so the embedder can resolve remote-vs-bundled and
    // the drift guard compares the right identity — a malformed config is a
    // loud exit 5, not a silent fallback to bundled.
    let config = crate::config::Config::default();
    let cfg = crate::config::load(&paths)?;

    // Pre-check catalog + plugin existence so we can surface the right exit
    // code before doing any model work. Lifecycle re-checks this internally;
    // duplicating one cheap directory probe avoids wasting a multi-MB
    // download on an obvious typo. Resolution reads the catalog enrolment from
    // the DB (F11b), so we open a read-only handle here.
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let _ = resolve_plugin_dir(&id, &conn, scope.scope.name().as_str(), &paths)?;

    // B4: resolve the ACTIVE profile's embedder (not the hard-coded default).
    let embedder_meta = crate::index::meta::active_embedder(&conn)?;

    // Phase 12 / US2: the drift-guard + meta seed must reflect the ACTIVE
    // embedder identity — remote (`"<provider>/<model>"`/`"external"`) when an
    // `[embedding]` provider is configured, else the active-profile registry
    // identity. So switching the embedding model HARD-fails this write path
    // (41/42) rather than landing mismatched vectors.
    let active_embedder_seed = crate::embedding::embedder_seed(&cfg, embedder_meta)?;

    // B3: refuse a partial re-embed under embedder drift BEFORE any model work.
    // If the configured (remote-or-bundled) embedder no longer matches the
    // embedder stamped in `meta`, enabling a plugin would land a new-dimension
    // vector in a table of old-dimension vectors. Direct the user at
    // `tome reindex`.
    crate::index::meta::guard_embedder_drift(
        &conn,
        &crate::index::meta::ModelIdent {
            name: active_embedder_seed.name.clone(),
            version: active_embedder_seed.version.clone(),
        },
    )?;
    drop(conn);

    // Phase 12: is the embedder remote? On the remote path the local-model
    // download prompt is skipped (there is no embedder model to fetch — the
    // reranker is loaded lazily by `query`, not here).
    let remote_embedding =
        crate::provider::resolve(&cfg, crate::provider::Capability::Embedding)?.is_some();

    // Model-presence handling — T074 UI side. The lifecycle's
    // `allow_model_download` boolean is always set to false because we own
    // the download path here. Lifecycle re-checks the manifests after we
    // return. Skipped on the remote path (no local embedder model).
    if !remote_embedding {
        ensure_models_or_prompt(&paths, &args, mode)?;
    }

    // Construct the embedder: remote when `[embedding]` is configured, else the
    // bundled active-profile model. Reranker isn't loaded on enable (the query
    // path loads it on demand). The drift guard above has already proven the
    // configured embedder matches the stored identity, so a remote embedder's
    // index-time validation asserts the persisted dimension; pass it as the seed.
    let persisted_dim = if remote_embedding {
        let conn = open_index_for_read(&paths, &scope.scope)?;
        crate::index::read_embedder_dimension(&conn)?
    } else {
        None
    };
    let embedder = crate::embedding::build_embedder(&cfg, &paths, embedder_meta, persisted_dim)?;

    let (_e_seed, reranker_seed, summariser_seed) = registry_seeds();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &scope.scope,
        config: &config,
        embedder: embedder.as_ref(),
        embedder_seed: active_embedder_seed,
        reranker_seed,
        summariser_seed,
        allow_model_download: false,
    };

    // Banner — human mode only. Skipping it in JSON keeps stdout
    // byte-stable; the structured record is the contract there.
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Enabling {}…", id)?;
    }

    let outcome = lifecycle::enable(&id, &deps)?;

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
    // We apply this BEFORE regenerate_for_trigger so RULES.md is recomposed
    // with the new tiers (regenerate_for_trigger → regen_summary::regen →
    // write_workspace_rules reads tiers from the DB).
    if let Some(tier) = args.tier {
        let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
        let tier_conn = crate::index::open(
            &paths.index_db,
            &crate::index::OpenOptions {
                embedder: embedder_seed,
                reranker: reranker_seed,
                summariser: summariser_seed,
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

    // FR-380 + FR-385: regenerate cached summaries AFTER the
    // workspace_skills mutation commits. The lifecycle transaction
    // above already committed; if the summariser fails, exit 24
    // surfaces and the prior `[summaries]` cache survives.
    crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;

    crate::telemetry::emit(crate::telemetry::event::PluginActionEvent {
        action: crate::telemetry::event::PluginAction::Enabled,
    });

    // FR-052: ALONGSIDE the anonymous event above, emit the catalog-attributed
    // `catalog.<id>.plugin_enabled` ONLY when the plugin's catalog resolves — at
    // emit time, by SOURCE not name — to an allowlisted catalog. `None` ⇒ the
    // anonymous event already fired and we add nothing (a name collision with a
    // non-allowlisted source stays anonymous). Best-effort throughout: the
    // attribution read and the version read are read-only, never lock, never
    // fail the command.
    if let Some(catalog_id) = crate::telemetry::resolve_attribution(scope, &id.catalog) {
        crate::telemetry::emit(crate::telemetry::event::PluginEnabled {
            catalog: catalog_id,
            plugin_name: id.plugin.clone(),
            plugin_version: super::attributed_plugin_version(&paths, &scope.scope, &id),
        });
    }

    // --sync (#280): propagate the change to bound harnesses inline, reusing the
    // SAME path `tome sync --all` uses (`commands::sync::sync_bound_projects` →
    // `sync_all` → `sync_project`), so this inherits every writer safety and the
    // forward-progress fan-out. It runs AFTER the state change + summary trigger
    // above have committed: a sync failure here surfaces the underlying
    // `sync_project` exit code but the enable itself is already durable and IS
    // reported first (the success line prints before we propagate the error).
    let projects_synced = if args.sync {
        // Print the enable success line NOW so the user always sees that the
        // enable landed, even if the follow-up sync errors out below.
        emit_enable_success(&id, &outcome, mode, true)?;
        let report = crate::commands::sync::sync_bound_projects(scope.scope.name(), &paths)?;
        Some(report.projects.len())
    } else {
        None
    };

    match (mode, projects_synced) {
        // --sync succeeded: the success line already printed above; now confirm
        // what was applied (human, via the shared SSOT) / carry the count (json).
        (Mode::Human, Some(n)) => super::emit_synced_confirmation(n),
        (Mode::Json, Some(n)) => emit_json(&id, &outcome, Some(n)),
        // No --sync: normal success emit with the "run `tome sync`" reminder.
        (Mode::Human, None) => emit_enable_success(&id, &outcome, mode, false),
        (Mode::Json, None) => emit_json(&id, &outcome, None),
    }
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
/// * User says no → clean exit with a stderr note (`Ok(())` returned but
///   the rest of the enable flow short-circuits via early return inside
///   this function — handled by the caller observing `Ok(())` + zero work
///   done by lifecycle? No: we need to actually NOT call lifecycle. So we
///   propagate the user's decline by returning `Err(Interrupted)`-style?
///   The contract isn't explicit; spec says "clean abort" — we return a
///   `TomeError::Interrupted` so exit code 8 surfaces, which is the right
///   "user-initiated abort" code. Implementation note in retro.
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

    let confirmed = if args.yes {
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
