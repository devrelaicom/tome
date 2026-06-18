//! `tome models download | list | remove` — explicit model artefact
//! management. Library-side download / install / atomic-rename lives in
//! `embedding::download`; the registry of pinned hashes lives in
//! `embedding::registry`. The handlers here orchestrate user-facing
//! presentation (prompts, progress, tables, NDJSON / JSON output).
//!
//! Spec: `contracts/models-commands.md`, FR-018..FR-024.

mod download;
mod list;
mod profile;
mod remove;

use std::path::PathBuf;

use crate::cli::ModelsCommand;
use crate::embedding::registry::{ModelEntry, ModelManifest};
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(cmd: ModelsCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // Models live under `data_dir` and are deliberately shared across
    // workspace + global scopes (FR-021: no per-scope models). The scope
    // is threaded for signature uniformity with the rest of the
    // commands; download / list / remove behaviour is independent of
    // it.
    let _ = scope;
    match cmd {
        ModelsCommand::Download(args) => download::run(args, mode),
        ModelsCommand::List(args) => list::run(args, mode),
        ModelsCommand::Remove(args) => remove::run(args, mode),
        ModelsCommand::Profile(args) => profile::run(args, mode),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// On-disk classification of a registered model's install state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState {
    /// Manifest + every declared file exists with the recorded size.
    Ok,
    /// No manifest on disk.
    Missing,
    /// Manifest present but at least one declared file is missing or has a
    /// wrong size.
    Corrupt,
    /// Manifest + files present and sized correctly, but the on-disk content
    /// SHA-256 disagrees with the manifest. Only produced when `--verify`
    /// is passed to `tome models list`.
    ChecksumMismatched,
}

impl ModelState {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelState::Ok => "ok",
            ModelState::Missing => "missing",
            ModelState::Corrupt => "corrupt",
            ModelState::ChecksumMismatched => "checksum_mismatched",
        }
    }
}

/// Read a model's `manifest.toml` from disk. Returns `Ok(Some(_))` when the
/// manifest exists and parses; `Ok(None)` when missing; `Err` on a strict
/// parse failure (manifest is Tome-owned, so unknown fields are rejected).
pub fn read_manifest(
    paths: &Paths,
    entry: &ModelEntry,
) -> Result<Option<ModelManifest>, TomeError> {
    let manifest_path = paths.model_manifest(entry.name)?;
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&manifest_path).map_err(TomeError::Io)?;
    let manifest = ModelManifest::from_toml_slice(&manifest_path, &bytes)?;
    Ok(Some(manifest))
}

/// Cheap install-state check: existence + size match. Returns the
/// derived state plus the manifest if one was read.
pub fn cheap_state(
    paths: &Paths,
    entry: &ModelEntry,
) -> Result<(ModelState, Option<ModelManifest>), TomeError> {
    let manifest = match read_manifest(paths, entry)? {
        Some(m) => m,
        None => return Ok((ModelState::Missing, None)),
    };
    let model_dir = paths.model_path(entry.name)?;
    for file in &manifest.files {
        let p = model_dir.join(file);
        match std::fs::metadata(&p) {
            Ok(md) if md.is_file() => continue,
            _ => return Ok((ModelState::Corrupt, Some(manifest))),
        }
    }
    // Size check applies to the primary artefact (the first file listed); the
    // others are tokenizer / config files that do not have a pinned size in
    // the registry. The manifest's `size_bytes` echoes the registry's pinned
    // primary size — verify the primary file matches.
    if let Some(primary) = manifest.files.first() {
        let primary_path = model_dir.join(primary);
        let actual = std::fs::metadata(&primary_path)
            .map_err(TomeError::Io)?
            .len();
        if actual != manifest.size_bytes {
            return Ok((ModelState::Corrupt, Some(manifest)));
        }
    }
    Ok((ModelState::Ok, Some(manifest)))
}

/// Resolve the path to a model's primary artefact (the first file in
/// `entry.files`, by convention the `.onnx` weight). Returns `None` when the
/// registry entry declares no files (no current entries do, but the guard
/// keeps the call site total).
pub fn primary_file_path(paths: &Paths, entry: &ModelEntry) -> Result<Option<PathBuf>, TomeError> {
    let Some(primary) = entry.files.first() else {
        return Ok(None);
    };
    Ok(Some(paths.model_path(entry.name)?.join(primary)))
}

pub use crate::presentation::format::human_mb;
