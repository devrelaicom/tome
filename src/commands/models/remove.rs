//! `tome models remove [<name>...] [--all]` — delete installed model(s).
//!
//! Issue #315 widened this from a single positional to a variadic selection
//! plus `--all` (mirroring `models download --all`):
//! - **names given** → exactly those registered models.
//! - **`--all`** → every INSTALLED model (a manifest on disk = installed).
//! - **no names + no `--all`** → a usage error (exit 2).
//!
//! `--all` + names is a clap conflict.
//!
//! Destructive-op safety: the confirmation gate (`--force` / non-TTY refusal,
//! FR-021 / #305) fires ONCE for the WHOLE batch, naming the set. Forward-
//! progress (`first_error`): each model is removed in turn; a per-model failure
//! is recorded and the loop CONTINUES, surfacing the first error's exit code.
//!
//! Spec: `contracts/models-commands.md` §"`tome models remove`".

use std::io::Write;

use serde::Serialize;

use crate::cli::ModelsRemoveArgs;
use crate::embedding::registry::{self, MODEL_REGISTRY, ModelEntry};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, prompt};

pub fn run(args: ModelsRemoveArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    // 1. Resolve the target set. `--all` = every installed model; names = those
    //    (validated as registered; unknown → exit 2). Neither → usage error.
    let targets = resolve_targets(&args, &paths)?;

    // Empty target set (only reachable via `--all` with nothing installed) is a
    // clean no-op — skip the confirmation gate (no "Remove 0 models?" prompt)
    // and emit an empty `--json` envelope.
    if targets.is_empty() {
        if mode == Mode::Json {
            output::write_json(&RemoveEnvelope { models: Vec::new() })?;
        } else {
            let mut out = std::io::stdout().lock();
            writeln!(out, "No installed models to remove.")?;
        }
        return Ok(());
    }

    // 2. Destructive-op gate — ONCE for the whole batch (do NOT prompt N times).
    if !args.force && !prompt::non_interactive() {
        // Non-TTY without --force → exit 54 with the documented pointer. Same
        // pattern as `plugin disable`.
        if !(output::stdin_is_tty() && output::stdout_is_tty()) {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(
                err,
                "Removal requires confirmation. Re-run with --force to skip the prompt."
            );
            return Err(TomeError::NotATerminal);
        }
        let names: Vec<&str> = targets.iter().map(|e| e.name).collect();
        let msg = if names.len() == 1 {
            format!("Remove {}?", names[0])
        } else {
            format!("Remove {} models ({})?", names.len(), names.join(", "))
        };
        if !prompt::confirm(&msg, false)? {
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: removal declined.");
            }
            return Ok(());
        }
    }

    // 3. Remove each target, forward-progress.
    let mut records: Vec<RemoveRecord> = Vec::with_capacity(targets.len());
    let mut first_error: Option<TomeError> = None;
    for entry in targets {
        match remove_one(entry, &paths, mode) {
            Ok(record) => records.push(record),
            Err(e) => {
                tracing::warn!(model = entry.name, error = %e, "models remove: model failed; continuing");
                records.push(RemoveRecord {
                    name: entry.name.to_owned(),
                    status: "failed",
                });
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    if mode == Mode::Json {
        // A SINGLE record serialises as the bare `RemoveRecord`
        // (`{"name":..,"status":..}`) so the pre-#315 single-name `--json` shape
        // is BYTE-IDENTICAL; a multi / `--all` run serialises as the
        // `{"models":[..]}` envelope (mirrors `models download`). The empty
        // `--all` case short-circuited earlier, so `records` is never empty here.
        if let [single] = records.as_slice() {
            output::write_json(single)?;
        } else {
            output::write_json(&RemoveEnvelope { models: records })?;
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The registry entries `remove` should delete.
///
/// - `--all` → every INSTALLED model (manifest on disk). An empty install set
///   yields an empty selection (a whole no-op).
/// - names → each `lookup`'d (unknown → exit 2, before any prompt/write) then
///   order-preserving deduped. A registered-but-uninstalled name → exit 30
///   (`ModelMissing`), matching the single-name path.
/// - neither → exit 2 (usage): no "all installed" default for a destructive op.
fn resolve_targets(
    args: &ModelsRemoveArgs,
    paths: &Paths,
) -> Result<Vec<&'static ModelEntry>, TomeError> {
    if args.all {
        return Ok(MODEL_REGISTRY
            .iter()
            .filter(|e| is_installed(e, paths))
            .collect());
    }
    if args.names.is_empty() {
        return Err(TomeError::Usage(
            "name at least one model to remove, or pass --all to remove every installed model"
                .into(),
        ));
    }

    let mut out: Vec<&'static ModelEntry> = Vec::with_capacity(args.names.len());
    for raw in &args.names {
        let entry = registry::lookup(raw)
            .ok_or_else(|| TomeError::Usage(format!("unknown model `{raw}`")))?;
        // Not installed → exit 30, matching the pre-#315 single-name behaviour.
        // The "not installed" predicate matches the download path's: no manifest
        // on disk means no install record.
        if !is_installed(entry, paths) {
            return Err(TomeError::ModelMissing {
                model: entry.name.to_owned(),
            });
        }
        if !out.iter().any(|e| e.name == entry.name) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// A model is "installed" iff its manifest is on disk. Path resolution failure
/// (a bad model name in the registry — not user-reachable) is treated as
/// not-installed for the `--all` scan.
fn is_installed(entry: &ModelEntry, paths: &Paths) -> bool {
    paths
        .model_manifest(entry.name)
        .map(|p| p.is_file())
        .unwrap_or(false)
}

/// Delete ONE model's manifest + directory. The confirmation gate is already
/// cleared by the caller.
fn remove_one(entry: &ModelEntry, paths: &Paths, mode: Mode) -> Result<RemoveRecord, TomeError> {
    let model_dir = paths.model_path(entry.name)?;
    let manifest_path = paths.model_manifest(entry.name)?;

    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Removing {}…", entry.name)?;
    }

    // Best-effort atomic: delete the manifest first so an interrupted remove
    // leaves the model in the same observable state as a never-installed one
    // (manifest absent → ModelState::Missing). Then delete the model directory;
    // if THAT fails the on-disk content is orphaned but `tome status` will still
    // classify the model as missing per FR-023.
    std::fs::remove_file(&manifest_path).map_err(TomeError::Io)?;
    if model_dir.is_dir() {
        std::fs::remove_dir_all(&model_dir).map_err(TomeError::Io)?;
    }

    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{} removed {}", colour::success("✓"), entry.name)?;
    }

    Ok(RemoveRecord {
        name: entry.name.to_owned(),
        status: "removed",
    })
}

#[derive(Serialize)]
struct RemoveEnvelope {
    models: Vec<RemoveRecord>,
}

#[derive(Serialize)]
struct RemoveRecord {
    name: String,
    status: &'static str,
}
