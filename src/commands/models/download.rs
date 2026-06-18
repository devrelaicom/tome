//! `tome models download` — fetch the active profile's models if missing.
//! With `--all`, fetch every registered model. With `--force`, re-download
//! whether or not the on-disk manifest already records a complete install.
//!
//! The default target set is the ACTIVE profile's `{embedder, reranker,
//! summariser}` (the summariser is profile-independent). Scoping the default
//! to the active profile mirrors the enable path's `ensure_models_or_prompt`
//! (B2) so a small/medium/large install never pulls every tier's weights.
//!
//! Spec: `contracts/models-commands.md` §"`tome models download`", FR-021.

use std::io::Write;
use std::time::Instant;

use serde::Serialize;
use tracing::info;

use crate::cli::ModelsDownloadArgs;
use crate::embedding::download::download_model;
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress};

use super::{ModelState, cheap_state, human_mb};

pub fn run(args: ModelsDownloadArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    std::fs::create_dir_all(&paths.models_dir).map_err(TomeError::Io)?;

    let targets = resolve_targets(&paths, args.all)?;

    let mut records: Vec<DownloadRecord> = Vec::new();

    for entry in targets {
        let (state, _manifest) = cheap_state(&paths, entry)?;
        let already_installed = matches!(state, ModelState::Ok);

        if already_installed && !args.force {
            // Skipped — the manifest + files are consistent. Report and move
            // on.
            if mode == Mode::Human {
                let mut out = std::io::stdout().lock();
                writeln!(
                    out,
                    "{} {} ({}) — {} {}",
                    colour::dim("·"),
                    entry.name,
                    entry.version,
                    human_mb(entry.size_bytes),
                    colour::dim("skipped"),
                )?;
            }
            records.push(DownloadRecord {
                name: entry.name.to_owned(),
                version: entry.version.to_owned(),
                kind: kind_str(entry.kind),
                action: "skipped",
                size_bytes: entry.size_bytes,
                sha256_verified: true,
                duration_ms: 0,
            });
            continue;
        };

        // Re-download or first install.
        let action_label: &'static str = if already_installed {
            "redownloaded"
        } else {
            "downloaded"
        };

        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "{} ({}) — {}",
                entry.name,
                entry.version,
                human_mb(entry.size_bytes)
            )?;
        }

        // F6 added a byte-progress hook to `download_model`; US4.a
        // (T319) wires the determinate byte bar so big artefacts (the
        // ~400 MB Qwen summariser, the ~280 MB reranker) show real
        // progress + ETA + throughput. The bar still works for tiny
        // artefacts; `byte_bar(0, ...)` saturates rather than panicking
        // (covered by `presentation::progress::tests::bar_with_zero_total_does_not_panic`).
        let pb = progress::byte_bar(entry.size_bytes, format!("downloading {}", entry.name));
        let cb = |bytes_so_far: u64, _total: u64| {
            pb.set_position(bytes_so_far);
        };
        let started = Instant::now();
        let result = download_model(entry, &paths.models_dir, Some(&cb));
        pb.finish_and_clear();
        let elapsed = started.elapsed();

        // OUTCOME-bearing: emit `tome.model_download` per attempt with the REAL
        // outcome (Ok on success / Failed on error). `model_id` is the closed
        // `&'static str` registry id; `error_class` is the failure's category.
        match &result {
            Ok(_) => crate::telemetry::enqueue(crate::telemetry::event::ModelDownload {
                model_id: entry.name,
                outcome: crate::telemetry::event::Outcome::Ok,
                error_class: None,
            }),
            Err(e) => crate::telemetry::enqueue(crate::telemetry::event::ModelDownload {
                model_id: entry.name,
                outcome: crate::telemetry::event::Outcome::Failed,
                error_class: Some(e.category()),
            }),
        }

        result?;
        info!(model = entry.name, "model artefact installed");

        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "  {} {} · {:.1}s",
                colour::success("✓"),
                action_label,
                elapsed.as_secs_f64(),
            )?;
        }

        let duration_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        records.push(DownloadRecord {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind: kind_str(entry.kind),
            action: action_label,
            size_bytes: entry.size_bytes,
            sha256_verified: true,
            duration_ms,
        });
    }

    if mode == Mode::Json {
        let envelope = DownloadEnvelope { models: records };
        output::write_json(&envelope)?;
    }

    Ok(())
}

/// The set of registry entries `download` should fetch. With `all`, every
/// `MODEL_REGISTRY` entry; otherwise the active profile's `{embedder,
/// reranker, summariser}` (resolved from the index `meta`, falling back to
/// the default profile when no DB exists — exactly what the bootstrap will
/// stamp).
fn resolve_targets(paths: &Paths, all: bool) -> Result<Vec<&'static ModelEntry>, TomeError> {
    if all {
        return Ok(MODEL_REGISTRY.iter().collect());
    }

    use crate::embedding::profile::{Profile, embedder_for, reranker_for};
    let profile = if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_profile(&conn)?
    } else {
        Profile::DEFAULT
    };

    Ok(vec![
        embedder_for(profile),
        reranker_for(profile),
        crate::summarise::registry::summariser_entry(),
    ])
}

fn kind_str(kind: crate::embedding::registry::ModelKind) -> &'static str {
    use crate::embedding::registry::ModelKind;
    match kind {
        ModelKind::Embedder => "embedder",
        ModelKind::Reranker => "reranker",
        ModelKind::Summariser => "summariser",
    }
}

#[derive(Serialize)]
struct DownloadEnvelope {
    models: Vec<DownloadRecord>,
}

#[derive(Serialize)]
struct DownloadRecord {
    name: String,
    version: String,
    kind: &'static str,
    action: &'static str,
    size_bytes: u64,
    sha256_verified: bool,
    duration_ms: u64,
}
