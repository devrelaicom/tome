//! `tome models list` — render the registry plus on-disk state. Without
//! `--verify`, the check is cheap (existence + size). With `--verify`, every
//! installed primary artefact is rehashed against its pinned SHA-256.
//!
//! Each row is annotated with the profile(s) that reference it (the
//! `Profiles` column / JSON `profiles`) and whether the ACTIVE profile selects
//! it (the `*` marker / JSON `active`). The active profile is read from the
//! index `meta`, falling back to the default when no DB exists.
//!
//! Spec: `contracts/models-commands.md` §"`tome models list`", FR-021.

use std::io::Write;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::cli::ModelsListArgs;
use crate::embedding::download::sha256_file;
use crate::embedding::profile::{Profile, embedder_for, reranker_for};
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelKind, ModelManifest};
use crate::error::TomeError;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, tables};

use super::{ModelState, cheap_state, human_mb, primary_file_path};

pub fn run(args: ModelsListArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let active = active_profile(&paths)?;
    let active_set = active_model_names(active);
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
        let profiles = profiles_for(entry);
        let active = active_set.contains(&entry.name);
        records.push(ListRecord::new(
            entry,
            state,
            manifest.as_ref(),
            &paths,
            profiles,
            active,
        )?);
    }

    match mode {
        Mode::Human => emit_human(&records),
        Mode::Json => emit_json(&records),
    }
}

/// Active profile from the index `meta`; the default when no DB exists.
fn active_profile(paths: &Paths) -> Result<Profile, TomeError> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_profile(&conn)
    } else {
        Ok(Profile::DEFAULT)
    }
}

/// The `{embedder, reranker}` names the given profile selects. The summariser
/// is profile-independent, so it is never in the active set (it is always
/// referenced by every profile via [`profiles_for`]).
fn active_model_names(profile: Profile) -> [&'static str; 2] {
    [embedder_for(profile).name, reranker_for(profile).name]
}

/// Which profiles reference a registry entry. Embedders/rerankers belong to
/// exactly the profile whose `{embedder, reranker}` they are; the summariser
/// is shared, so every profile references it.
fn profiles_for(entry: &ModelEntry) -> Vec<&'static str> {
    if matches!(entry.kind, ModelKind::Summariser) {
        return Profile::ALL.iter().map(|p| p.as_str()).collect();
    }
    Profile::ALL
        .iter()
        .filter(|p| embedder_for(**p).name == entry.name || reranker_for(**p).name == entry.name)
        .map(|p| p.as_str())
        .collect()
}

fn emit_human(records: &[ListRecord]) -> Result<(), TomeError> {
    let mut t = tables::new_table();
    t.set_header(vec![
        "Name", "Version", "Kind", "Profiles", "Size", "State", "Path", "Licence",
    ]);
    for r in records {
        let state_cell = match r.state_enum {
            ModelState::Ok => Cell::new(colour::success(r.state)),
            _ => Cell::new(colour::error(r.state)),
        };
        // `*` marks the entry the ACTIVE profile selects; the column lists
        // every profile that references it.
        let profiles_label = if r.active {
            format!("{} {}", r.profiles.join(","), colour::success("*"))
        } else {
            r.profiles.join(",")
        };
        // Bold the name when it is in the active set so the selected models
        // stand out at a glance even without the colour `*`.
        let name_cell = if r.active {
            Cell::new(colour::bold(&r.name))
        } else {
            Cell::new(&r.name)
        };
        t.add_row(vec![
            name_cell,
            Cell::new(&r.version),
            Cell::new(r.kind),
            Cell::new(profiles_label),
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
    /// The profile(s) (small/medium/large) that reference this entry.
    profiles: Vec<&'static str>,
    /// True when the ACTIVE profile selects this entry (its embedder or
    /// reranker). The summariser is shared, so it is never `active`.
    active: bool,
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
        profiles: Vec<&'static str>,
        active: bool,
    ) -> Result<Self, TomeError> {
        let kind = match entry.kind {
            ModelKind::Embedder => "embedder",
            ModelKind::Reranker => "reranker",
            ModelKind::Summariser => "summariser",
        };
        let size_bytes = manifest.map(|m| m.size_bytes).unwrap_or(entry.size_bytes);
        let path = paths.model_path(entry.name)?.to_string_lossy().into_owned();
        Ok(Self {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind,
            profiles,
            active,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> &'static ModelEntry {
        crate::embedding::registry::lookup(name).expect("registered")
    }

    #[test]
    fn profiles_for_maps_each_embedder_and_reranker_to_its_one_profile() {
        assert_eq!(profiles_for(entry("bge-small-en-v1.5")), vec!["small"]);
        assert_eq!(profiles_for(entry("bge-base-en-v1.5")), vec!["medium"]);
        assert_eq!(profiles_for(entry("bge-large-en-v1.5")), vec!["large"]);
        assert_eq!(profiles_for(entry("bge-reranker-base")), vec!["small"]);
        assert_eq!(profiles_for(entry("bge-reranker-large")), vec!["medium"]);
        assert_eq!(profiles_for(entry("bge-reranker-v2-m3")), vec!["large"]);
    }

    #[test]
    fn profiles_for_marks_the_summariser_in_every_profile() {
        assert_eq!(
            profiles_for(entry("qwen2.5-0.5b-instruct")),
            vec!["small", "medium", "large"],
        );
    }

    #[test]
    fn active_model_names_are_the_profiles_embedder_and_reranker() {
        assert_eq!(
            active_model_names(Profile::Large),
            ["bge-large-en-v1.5", "bge-reranker-v2-m3"],
        );
        assert_eq!(
            active_model_names(Profile::Small),
            ["bge-small-en-v1.5", "bge-reranker-base"],
        );
    }

    #[test]
    fn every_registry_entry_is_referenced_by_at_least_one_profile() {
        // The brief's invariant: all entries map (Small references the legacy
        // bge-reranker-base, so there is no orphan `—` row).
        for entry in MODEL_REGISTRY {
            assert!(
                !profiles_for(entry).is_empty(),
                "entry `{}` is referenced by no profile",
                entry.name,
            );
        }
    }
}
