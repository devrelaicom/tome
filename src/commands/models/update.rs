//! `tome models update [--include-registry]` — bring local model assets up to
//! date and, optionally, refresh the harness model-ID registry override from
//! models.dev.
//!
//! Base behaviour delegates to `models download` with default flags (no
//! `--force`, active profile only). With `--include-registry`, a fresh
//! `model-registry.json` is fetched from `https://models.dev/api.json`, parsed,
//! validated, and atomically written to `~/.tome/cache/model-registry.json`.

use crate::cli::{ModelsDownloadArgs, ModelsUpdateArgs};
use crate::error::TomeError;
use crate::model_registry::{self, RegistryInfo, RegistrySource};
use crate::output::Mode;
use crate::paths::Paths;

pub fn run(args: ModelsUpdateArgs, mode: Mode) -> Result<(), TomeError> {
    // Base: ensure the active profile's models are present (re-download any
    // missing; never force-re-download already-installed files).
    super::download::run(
        ModelsDownloadArgs {
            force: false,
            all: false,
        },
        mode,
    )?;

    if args.include_registry {
        let paths = Paths::resolve()?;
        let fetched_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| {
                TomeError::Io(std::io::Error::other(format!(
                    "model registry timestamp: {e}"
                )))
            })?;
        let info = model_registry::refresh_override(&paths, &fetched_at, fetch_models_dev)?;
        report_registry(&info, mode)?;
    }
    Ok(())
}

/// Production fetcher: HTTP GET → raw bytes. Errors are scrubbed via
/// `catalog::git::scrub_to_string` before being wrapped in `TomeError::Io`.
fn fetch_models_dev(url: &str) -> Result<Vec<u8>, TomeError> {
    let resp = reqwest::blocking::get(url).map_err(|e| {
        TomeError::Io(std::io::Error::other(crate::catalog::git::scrub_to_string(
            format!("model registry fetch failed: {e}").as_bytes(),
        )))
    })?;
    if !resp.status().is_success() {
        return Err(TomeError::Io(std::io::Error::other(format!(
            "model registry fetch: HTTP {}",
            resp.status()
        ))));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| TomeError::Io(std::io::Error::other(format!("read response body: {e}"))))
}

/// Emit a brief human-readable or JSON summary of the refresh result.
fn report_registry(info: &RegistryInfo, mode: Mode) -> Result<(), TomeError> {
    let source_label = match info.source {
        RegistrySource::Override => "override",
        RegistrySource::Baked => "baked",
    };
    match mode {
        Mode::Json => {
            // Emit a minimal JSON record consistent with the output module.
            let record = serde_json::json!({
                "event": "registry_refreshed",
                "source": source_label,
                "model_count": info.model_count,
                "fetched_at": info.fetched_at,
            });
            crate::output::write_json(&record)?;
        }
        Mode::Human => {
            println!(
                "Model registry override refreshed: {} models (fetched {}).",
                info.model_count, info.fetched_at
            );
        }
    }
    Ok(())
}
