//! `tome models download` — fetch every registered model that is missing.
//! With `--force`, re-download every model whether or not the on-disk
//! manifest already records a complete install.
//!
//! Spec: `contracts/models-commands.md` §"`tome models download`".

use std::io::Write;
use std::time::Instant;

use serde::Serialize;
use tracing::info;

use crate::cli::ModelsDownloadArgs;
use crate::embedding::download::download_model;
use crate::embedding::registry::MODEL_REGISTRY;
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress};

use super::{ModelState, cheap_state, human_mb};

pub fn run(args: ModelsDownloadArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    std::fs::create_dir_all(&paths.models_dir).map_err(TomeError::Io)?;

    let mut records: Vec<DownloadRecord> = Vec::new();

    for entry in MODEL_REGISTRY {
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
