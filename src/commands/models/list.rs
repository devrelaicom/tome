//! `tome models list` — render the registry plus on-disk state. Without
//! `--verify`, the check is cheap (existence + size). With `--verify`, every
//! installed primary artefact is rehashed against its pinned SHA-256.
//!
//! Spec: `contracts/models-commands.md` §"`tome models list`".

use std::io::Write;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::cli::ModelsListArgs;
use crate::embedding::download::sha256_file;
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelKind, ModelManifest};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, tables};

use super::{ModelState, cheap_state, human_mb, primary_file_path};

pub fn run(args: ModelsListArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let mut records: Vec<ListRecord> = Vec::with_capacity(MODEL_REGISTRY.len());

    for entry in MODEL_REGISTRY {
        let (mut state, manifest) = cheap_state(&paths, entry)?;
        if args.verify && matches!(state, ModelState::Ok) {
            // Rehash the primary artefact. The full file is read once;
            // memory stays bounded by `STREAM_CHUNK_SIZE`.
            if let Some(p) = primary_file_path(&paths, entry)? {
                let observed = sha256_file(&p)?;
                if !observed.eq_ignore_ascii_case(entry.sha256) {
                    state = ModelState::ChecksumMismatched;
                }
            }
        }
        records.push(ListRecord::new(entry, state, manifest.as_ref(), &paths)?);
    }

    match mode {
        Mode::Human => emit_human(&records),
        Mode::Json => emit_json(&records),
    }
}

fn emit_human(records: &[ListRecord]) -> Result<(), TomeError> {
    let mut t = tables::new_table();
    t.set_header(vec![
        "Name", "Version", "Kind", "Size", "State", "Path", "Licence",
    ]);
    for r in records {
        let state_cell = match r.state_enum {
            ModelState::Ok => Cell::new(colour::success(r.state)),
            _ => Cell::new(colour::error(r.state)),
        };
        t.add_row(vec![
            Cell::new(&r.name),
            Cell::new(&r.version),
            Cell::new(r.kind),
            Cell::new(&r.size).set_alignment(CellAlignment::Right),
            state_cell,
            Cell::new(&r.path),
            Cell::new(&r.licence),
        ]);
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{t}")?;
    Ok(())
}

fn emit_json(records: &[ListRecord]) -> Result<(), TomeError> {
    output::write_json(records)
}

#[derive(Serialize)]
struct ListRecord {
    name: String,
    version: String,
    kind: &'static str,
    size_bytes: u64,
    state: &'static str,
    /// String form of `state` re-emitted with the JSON record's canonical
    /// label. Mirrored separately because the table renderer needs both the
    /// `ModelState` discriminant (for colouring) and the string (for the
    /// human cell), and the JSON-only `state` field is the labelled one.
    #[serde(skip)]
    #[allow(dead_code)]
    state_enum: ModelState,
    size: String,
    path: String,
    licence: String,
    sha256: String,
    /// When present, the SHA-256 actually computed from disk (only set by
    /// `--verify`).
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256_observed: Option<String>,
}

impl ListRecord {
    fn new(
        entry: &ModelEntry,
        state: ModelState,
        manifest: Option<&ModelManifest>,
        paths: &Paths,
    ) -> Result<Self, TomeError> {
        let kind = match entry.kind {
            ModelKind::Embedder => "embedder",
            ModelKind::Reranker => "reranker",
        };
        let size_bytes = manifest.map(|m| m.size_bytes).unwrap_or(entry.size_bytes);
        let path = paths.model_path(entry.name)?.to_string_lossy().into_owned();
        Ok(Self {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind,
            size_bytes,
            state: state.as_str(),
            state_enum: state,
            size: human_mb(size_bytes),
            path,
            licence: entry.licence.to_owned(),
            sha256: entry.sha256.to_owned(),
            sha256_observed: None,
        })
    }
}
