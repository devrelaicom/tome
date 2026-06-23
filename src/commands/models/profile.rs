//! `tome models profile [show | set <small|medium|large>]` — inspect or
//! switch the active model profile.
//!
//! A profile selects which embedder + reranker (from `MODEL_REGISTRY`) Tome
//! uses; the summariser is profile-independent. The active profile is a
//! GLOBAL property persisted in the index `meta` table (FR-021 — models are
//! not per-scope), keyed by [`MetaKey::ModelProfile`].
//!
//! `show` prints the active profile plus its embedder/reranker and each
//! model's on-disk install state. `set` writes the new profile and, when the
//! embedder identity changes, prints a clear "run `tome reindex`" notice —
//! the switch never auto-reindexes (the existing drift→reindex mechanism is
//! the single resolver, B1). When the new reranker is not yet downloaded it
//! hints `tome models download`.
//!
//! Spec: data-model.md §8 (`MetaKey::ModelProfile`), FR-021.

use std::io::Write;

use serde::Serialize;

use crate::cli::ModelsProfileArgs;
use crate::embedding::profile::{Profile, embedder_for, reranker_for};
use crate::embedding::registry::ModelEntry;
use crate::error::TomeError;
use crate::index::meta::{self, MetaKey};
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::colour;

use super::{ModelState, cheap_state};

pub fn run(args: ModelsProfileArgs, mode: Mode) -> Result<(), TomeError> {
    match args.tier {
        Some(tier) => set(&tier, mode),
        None => show(mode),
    }
}

/// `tome models profile` (no tier) — report the active profile and its models.
fn show(mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let active = read_active_profile(&paths)?;
    let record = ProfileShowRecord::build(&paths, active)?;
    match mode {
        Mode::Human => emit_show_human(&record),
        Mode::Json => output::write_json(&record),
    }
}

/// `tome models profile set <tier>` — switch the active profile.
fn set(tier: &str, mode: Mode) -> Result<(), TomeError> {
    // clap's `value_parser` already restricts `tier` to the three valid
    // strings, so `from_tier_str` cannot fail here; the `?`-style fallback
    // keeps the function total without an unreachable panic.
    let new_profile = Profile::from_tier_str(tier).ok_or_else(|| {
        TomeError::Usage(format!(
            "unknown model profile `{tier}` (expected small/medium/large)"
        ))
    })?;

    let paths = Paths::resolve()?;

    // The profile lives in the index `meta`, so the DB must exist to record
    // it. Bootstrap one if absent (mirrors the lifecycle commands' first-touch
    // behaviour) using the DEFAULT-profile seeds — the active-profile row we
    // then write is what actually takes effect.
    let (embedder, reranker, summariser) = crate::commands::plugin::registry_seeds();
    let conn = crate::index::open(
        &paths.index_db,
        &crate::index::OpenOptions {
            embedder,
            reranker,
            summariser,
            profile: None,
        },
    )?;
    let lock = crate::index::acquire_lock(&paths.index_lock)?;

    // The previously-stored embedder identity, read BEFORE the write so we can
    // detect whether the switch changes the embedder (and therefore the
    // embedding dimension → a reindex is required).
    let stored_embedder_name = meta::read(&conn, MetaKey::EmbedderName)?.unwrap_or_default();

    let result = meta::write(&conn, MetaKey::ModelProfile, new_profile.as_str());
    match result {
        Ok(()) => lock.release()?,
        Err(e) => {
            drop(lock);
            return Err(e);
        }
    }

    let new_embedder = embedder_for(new_profile);
    let new_reranker = reranker_for(new_profile);

    // Embedder change → the stored vectors are now the wrong dimension. We do
    // NOT auto-reindex; the drift→reindex path (guard_embedder_drift) is the
    // single resolver. Surface a clear notice with the old/new dims.
    let embedder_changed =
        !stored_embedder_name.is_empty() && stored_embedder_name != new_embedder.name;
    let prev_dim = embedder_dim(&stored_embedder_name);
    let new_dim = new_embedder.embedding_dim;

    // Reranker change + not yet downloaded → hint a download. Embedder is
    // covered by the reindex flow (which loads/downloads as needed); the
    // reranker has no such trigger, so the explicit hint is the only nudge.
    let (reranker_state, _) = cheap_state(&paths, new_reranker)?;
    let reranker_missing = !matches!(reranker_state, ModelState::Ok);

    let record = ProfileSetRecord {
        profile: new_profile.as_str(),
        embedder: new_embedder.name.to_owned(),
        reranker: new_reranker.name.to_owned(),
        embedder_changed,
        prev_embedder_dim: if embedder_changed { prev_dim } else { None },
        new_embedder_dim: if embedder_changed { new_dim } else { None },
        reindex_required: embedder_changed,
        reranker_download_hint: reranker_missing,
    };

    match mode {
        Mode::Human => emit_set_human(&record),
        Mode::Json => output::write_json(&record),
    }
}

/// Read the active profile from the index `meta`, falling back to the default
/// when the DB is absent (a fresh install reports the default the bootstrap
/// will stamp). Read-only — `show` never mutates.
fn read_active_profile(paths: &Paths) -> Result<Profile, TomeError> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        meta::active_profile(&conn)
    } else {
        Ok(Profile::DEFAULT)
    }
}

/// Resolve a registered embedder's output dimension by name. `None` when the
/// name is unregistered (e.g. a legacy meta row) or names a non-embedder.
fn embedder_dim(name: &str) -> Option<u32> {
    crate::embedding::registry::lookup(name).and_then(|e| e.embedding_dim)
}

// ---------------------------------------------------------------------------
// `show` record + rendering
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProfileShowRecord {
    profile: &'static str,
    embedder: ModelLine,
    reranker: ModelLine,
}

#[derive(Serialize)]
struct ModelLine {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_dim: Option<u32>,
    licence: String,
    state: &'static str,
}

impl ModelLine {
    fn build(paths: &Paths, entry: &ModelEntry) -> Result<Self, TomeError> {
        let (state, _) = cheap_state(paths, entry)?;
        Ok(Self {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            embedding_dim: entry.embedding_dim,
            licence: entry.licence.to_owned(),
            state: state.as_str(),
        })
    }
}

impl ProfileShowRecord {
    fn build(paths: &Paths, profile: Profile) -> Result<Self, TomeError> {
        Ok(Self {
            profile: profile.as_str(),
            embedder: ModelLine::build(paths, embedder_for(profile))?,
            reranker: ModelLine::build(paths, reranker_for(profile))?,
        })
    }
}

fn emit_show_human(record: &ProfileShowRecord) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} {}",
        colour::label("active profile:"),
        colour::bold(record.profile),
    )?;
    write_model_line(&mut out, "embedder", &record.embedder)?;
    write_model_line(&mut out, "reranker", &record.reranker)?;
    Ok(())
}

fn write_model_line(out: &mut impl Write, role: &str, line: &ModelLine) -> Result<(), TomeError> {
    let state_label = if line.state == ModelState::Ok.as_str() {
        colour::success(line.state)
    } else {
        colour::warning(line.state)
    };
    let dim = match line.embedding_dim {
        Some(d) => format!(" · {d}-d"),
        None => String::new(),
    };
    writeln!(
        out,
        "  {} {} ({}){} · {} · {}",
        colour::label(&format!("{role}:")),
        line.name,
        line.version,
        dim,
        line.licence,
        state_label,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `set` record + rendering
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProfileSetRecord {
    profile: &'static str,
    embedder: String,
    reranker: String,
    /// True when the switch changes the embedder identity (and therefore the
    /// embedding dimension), so the stored vectors must be re-embedded.
    embedder_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_embedder_dim: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_embedder_dim: Option<u32>,
    /// Mirror of `embedder_changed` named for the caller's intent: a reindex
    /// is required before queries return correct results.
    reindex_required: bool,
    /// True when the new profile's reranker is not yet on disk.
    reranker_download_hint: bool,
}

fn emit_set_human(record: &ProfileSetRecord) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} {}",
        colour::label("active profile set to"),
        colour::bold(record.profile),
    )?;
    writeln!(out, "  embedder: {}", record.embedder)?;
    writeln!(out, "  reranker: {}", record.reranker)?;

    if record.embedder_changed {
        let dims = match (record.prev_embedder_dim, record.new_embedder_dim) {
            (Some(a), Some(b)) => format!(" (dim {a}→{b})"),
            _ => String::new(),
        };
        writeln!(
            out,
            "{} embedder changed{}; run `{}` to re-embed the index",
            colour::warning("!"),
            dims,
            colour::hint("tome reindex"),
        )?;
    }
    if record.reranker_download_hint {
        writeln!(
            out,
            "{} reranker `{}` is not downloaded; run `{}`",
            colour::hint("·"),
            record.reranker,
            colour::hint("tome models download"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedder_dim_resolves_embedder_dimension() {
        // The medium embedder is 768-d; a reranker has no dim.
        assert_eq!(embedder_dim("bge-base-en-v1.5"), Some(768));
        assert_eq!(embedder_dim("bge-reranker-large"), None);
        assert_eq!(embedder_dim("does-not-exist"), None);
    }

    #[test]
    fn profile_set_record_json_shape_is_stable() {
        // Field order == declaration order (serde_json preserve_order is on
        // crate-wide). Omitted dims when the embedder did not change.
        let r = ProfileSetRecord {
            profile: "small",
            embedder: "bge-small-en-v1.5".into(),
            reranker: "bge-reranker-base".into(),
            embedder_changed: false,
            prev_embedder_dim: None,
            new_embedder_dim: None,
            reindex_required: false,
            reranker_download_hint: false,
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"profile":"small","embedder":"bge-small-en-v1.5","reranker":"bge-reranker-base","embedder_changed":false,"reindex_required":false,"reranker_download_hint":false}"#,
        );
    }

    #[test]
    fn profile_set_record_includes_dims_on_embedder_change() {
        let r = ProfileSetRecord {
            profile: "large",
            embedder: "bge-large-en-v1.5".into(),
            reranker: "bge-reranker-v2-m3".into(),
            embedder_changed: true,
            prev_embedder_dim: Some(768),
            new_embedder_dim: Some(1024),
            reindex_required: true,
            reranker_download_hint: true,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v["embedder_changed"], true);
        assert_eq!(v["prev_embedder_dim"], 768);
        assert_eq!(v["new_embedder_dim"], 1024);
        assert_eq!(v["reindex_required"], true);
        assert_eq!(v["reranker_download_hint"], true);
    }
}
