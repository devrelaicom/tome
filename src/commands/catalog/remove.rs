//! `tome catalog remove`. See `contracts/catalog-remove.md`.
//!
//! Cache removal is best-effort: failures are logged at WARN and do not
//! propagate. The registry write is the source-of-truth atomicity guarantee.

use std::io::{BufRead, Write};

use serde::Serialize;
use tracing::warn;

use crate::catalog::store;
use crate::cli::CatalogRemoveArgs;
use crate::config::CatalogEntry;
use crate::error::TomeError;
use crate::output;
use crate::output::Mode;
use crate::paths::Paths;

pub fn run(args: CatalogRemoveArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let mut config = store::load(&paths.config_file)?;

    let entry = config
        .catalogs
        .get(&args.name)
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?
        .clone();

    if !args.force {
        if !output::stdin_is_tty() {
            return Err(TomeError::Usage(
                "'tome catalog remove' requires --force in non-interactive contexts".into(),
            ));
        }
        if !prompt_yes_no(&format!(
            "Remove catalog '{}' and its local cache at {}? [y/N]",
            entry.name,
            entry.path.display()
        ))? {
            // Declined — exit 0 with no mutation.
            return Ok(());
        }
    }

    config.catalogs.remove(&args.name);
    store::save(&paths.config_file, &config)?;

    if let Err(e) = std::fs::remove_dir_all(&entry.path) {
        warn!(
            cache_path = %entry.path.display(),
            error = %e,
            "cache directory could not be removed; registry already updated"
        );
    }

    emit(mode, &entry)?;
    Ok(())
}

fn prompt_yes_no(prompt: &str) -> Result<bool, TomeError> {
    let mut stderr = std::io::stderr().lock();
    write!(stderr, "{} ", prompt)?;
    stderr.flush()?;
    drop(stderr);

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

#[derive(Serialize)]
struct RemovedEnvelope<'a> {
    removed: RemovedRecord<'a>,
}

#[derive(Serialize)]
struct RemovedRecord<'a> {
    name: &'a str,
    url: &'a str,
    cache_path: String,
}

fn emit(mode: Mode, entry: &CatalogEntry) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Removed catalog `{}` (cache cleared at {}).",
                entry.name,
                entry.path.display()
            )?;
        }
        Mode::Json => {
            let env = RemovedEnvelope {
                removed: RemovedRecord {
                    name: &entry.name,
                    url: &entry.url,
                    cache_path: entry.path.display().to_string(),
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}
