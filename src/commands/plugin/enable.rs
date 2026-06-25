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

    match mode {
        Mode::Human => emit_human(&id, &outcome),
        Mode::Json => emit_json(&id, &outcome),
    }
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

fn emit_human(id: &PluginId, outcome: &lifecycle::EnableOutcome) -> Result<(), TomeError> {
    let secs = outcome.duration.as_secs_f64();
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} {} skills indexed ({} newly embedded) in {:.1}s",
        colour::success("✓"),
        outcome.summary.total_skills,
        outcome.summary.newly_embedded,
        secs,
    )?;
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
}

fn emit_json(id: &PluginId, outcome: &lifecycle::EnableOutcome) -> Result<(), TomeError> {
    let duration_ms = outcome.duration.as_millis().min(u128::from(u64::MAX)) as u64;
    let record = EnableRecord {
        plugin: id.to_string(),
        status: "enabled",
        skills_indexed: outcome.summary.total_skills,
        skills_newly_embedded: outcome.summary.newly_embedded,
        duration_ms,
    };
    output::write_json(&record)
}
