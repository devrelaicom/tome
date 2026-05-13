//! `tome models remove <name>` — delete an installed model.
//!
//! Spec: `contracts/models-commands.md` §"`tome models remove`".

use std::io::Write;

use serde::Serialize;

use crate::cli::ModelsRemoveArgs;
use crate::embedding::registry::{self};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, prompt};

pub fn run(args: ModelsRemoveArgs, mode: Mode) -> Result<(), TomeError> {
    let entry = registry::lookup(&args.name)
        .ok_or_else(|| TomeError::Usage(format!("unknown model `{}`", args.name)))?;

    let paths = Paths::resolve()?;
    let model_dir = paths.model_path(entry.name)?;
    let manifest_path = paths.model_manifest(entry.name)?;

    // Not installed → exit 30. The "not installed" predicate matches the
    // download path's: no manifest on disk means no install record.
    if !manifest_path.is_file() {
        return Err(TomeError::ModelMissing {
            model: entry.name.to_owned(),
        });
    }

    if !args.force {
        // Non-TTY without --force → exit 54 with the documented pointer.
        // Same pattern as `plugin disable`.
        if !(output::stdin_is_tty() && output::stdout_is_tty()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "Removal requires confirmation. Re-run with --force to skip the prompt."
            );
            return Err(TomeError::NotATerminal);
        }
        if !prompt::confirm(&format!("Remove {}?", entry.name), false)? {
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: removal declined.");
            }
            return Ok(());
        }
    }

    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Removing {}…", entry.name)?;
    }

    // Best-effort atomic: delete the manifest first so an interrupted remove
    // leaves the model in the same observable state as a never-installed one
    // (manifest absent → ModelState::Missing). Then delete the model
    // directory; if THAT fails the on-disk content is orphaned but
    // `tome status` will still classify the model as missing per FR-023.
    std::fs::remove_file(&manifest_path).map_err(TomeError::Io)?;
    if model_dir.is_dir() {
        std::fs::remove_dir_all(&model_dir).map_err(TomeError::Io)?;
    }

    let record = RemoveRecord {
        name: entry.name.to_owned(),
        status: "removed",
    };

    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(out, "{} removed {}", colour::success("✓"), entry.name)?;
            Ok(())
        }
        Mode::Json => output::write_json(&record),
    }
}

#[derive(Serialize)]
struct RemoveRecord {
    name: String,
    status: &'static str,
}
