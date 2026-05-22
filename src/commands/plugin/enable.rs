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

use crate::catalog::store;
use crate::cli::PluginEnableArgs;
use crate::embedding::download::download_model;
use crate::embedding::fastembed::FastembedEmbedder;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::{colour, progress, prompt};
use crate::workspace::ResolvedScope;

use super::{embedder_entry, human_mb, missing_models, registry_seeds, resolve_plugin_dir};

pub fn run(args: PluginEnableArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 reintroduces workspace-aware view.
    let config = store::load(&paths.global_config_file)?;

    // Pre-check catalog + plugin existence so we can surface the right exit
    // code before doing any model work. Lifecycle re-checks this internally;
    // duplicating one cheap directory probe avoids wasting a multi-MB
    // download on an obvious typo.
    let _ = resolve_plugin_dir(&id, &config)?;

    // Model-presence handling — T074 UI side. The lifecycle's
    // `allow_model_download` boolean is always set to false because we own
    // the download path here. Lifecycle re-checks the manifests after we
    // return.
    ensure_models_or_prompt(&paths, &args, mode)?;

    // Construct the real embedder. Reranker isn't loaded on enable (the
    // query path will load it on demand).
    let embedder_meta = embedder_entry();
    let embedder_dir = paths.model_path(embedder_meta.name)?;
    let embedder = FastembedEmbedder::load(embedder_meta, &embedder_dir)?;

    let (embedder_seed, reranker_seed) = registry_seeds();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &scope.scope,
        config: &config,
        embedder: &embedder,
        embedder_seed,
        reranker_seed,
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
    let missing = missing_models(paths);
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
