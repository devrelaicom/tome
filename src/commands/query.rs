//! `tome query <text>` — KNN search over enabled skills with an optional
//! cross-encoder reranker stage.
//!
//! Slice 2 of Phase 3 (User Story 1). The lifecycle and `tome plugin …`
//! commands shipped in slice 1; this slice composes the read-only side:
//! open the index, embed the query, KNN, optionally rerank, render.
//!
//! Spec: `contracts/query.md`. No model download is offered here — the user
//! should have installed models via `tome plugin enable` first, where the
//! TTY prompt belongs. Query is meant to be fast; surfacing a multi-MB
//! download behind a `tome query` is hostile UX.

use std::io::Write;
use std::path::PathBuf;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::catalog::store;
use crate::cli::QueryArgs;
use crate::config::Config;
use crate::embedding::fastembed::{FastembedEmbedder, FastembedReranker};
use crate::embedding::{Embedder, Reranker, Scored};
use crate::error::TomeError;
use crate::index::meta::{self, DriftStatus, ModelIdent};
use crate::index::query::{QueryFilters, knn};
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress, tables};

use super::plugin::{
    embedder_entry, missing_models, open_index_for_read, read_catalog_manifest, reranker_entry,
};

/// Either the reranker's raw logit ("reranked") or 1.0 − cosine distance
/// ("embedding-similarity"). The string is duplicated at the top level and
/// per result in the JSON envelope so consumers can pick either form.
const SCORING_RERANKED: &str = "reranked";
const SCORING_SIMILARITY: &str = "embedding-similarity";

pub fn run(args: QueryArgs, mode: Mode) -> Result<(), TomeError> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err(TomeError::Usage("query text is empty".into()));
    }

    let paths = Paths::resolve()?;
    let config = store::load(&paths.config_file)?;

    // Validate filter flags before any model / DB work — these are cheap
    // catalog-manifest reads and fail fast on typos.
    validate_filters(&args, &config)?;

    let conn = open_index_for_read(&paths)?;

    // Drift detection. Embedder drift hard-fails (vectors are stale);
    // reranker drift only degrades quality, so we keep the value and
    // surface it as a warning later.
    let reranker_drift = check_drift(&conn)?;

    // Model presence — embedder always required, reranker required unless
    // `--no-rerank`.
    let embedder_meta = embedder_entry();
    let missing = missing_models(&paths);
    if missing.iter().any(|e| e.name == embedder_meta.name) {
        return Err(TomeError::ModelMissing {
            model: embedder_meta.name.to_owned(),
        });
    }
    let reranker_meta = reranker_entry();
    if !args.no_rerank && missing.iter().any(|e| e.name == reranker_meta.name) {
        return Err(TomeError::ModelMissing {
            model: reranker_meta.name.to_owned(),
        });
    }

    // Load models. Wrap each in a spinner; non-TTY stderr → hidden draw
    // target (see presentation::progress::target).
    let embedder = {
        let pb = progress::spinner(format!("loading embedder ({})", embedder_meta.name));
        let result = FastembedEmbedder::load(embedder_meta, &paths.model_path(embedder_meta.name)?);
        pb.finish_and_clear();
        result?
    };
    let reranker = if args.no_rerank {
        None
    } else {
        let pb = progress::spinner(format!("loading reranker ({})", reranker_meta.name));
        let result = FastembedReranker::load(reranker_meta, &paths.model_path(reranker_meta.name)?);
        pb.finish_and_clear();
        Some(result?)
    };

    // Embed the query text as-is — FR-014's name/description composition
    // applies only to skill ingestion.
    let query_vec = embedder.embed(text)?;

    // Pull candidates. Reranking benefits from a wider pool — 4× per the
    // contract — and we trim back after.
    let candidate_k: u32 = if reranker.is_some() {
        args.top_k.saturating_mul(4).max(args.top_k)
    } else {
        args.top_k
    };
    let filters = QueryFilters {
        catalog: args.catalog.as_deref(),
        plugin: args.plugin.as_deref(),
    };
    let candidates = knn(&conn, &query_vec, candidate_k, &filters)?;

    // Score + sort. With a reranker, scores come from the cross-encoder;
    // without, we treat 1.0 − distance as cosine similarity.
    let scored: Vec<Scored> = match &reranker {
        Some(r) => r.rerank(text, candidates)?,
        None => {
            let mut s: Vec<Scored> = candidates
                .into_iter()
                .map(|c| Scored {
                    score: 1.0 - c.distance,
                    candidate: c,
                })
                .collect();
            s.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            s
        }
    };

    let top_k = args.top_k as usize;
    let mut trimmed: Vec<Scored> = scored.into_iter().take(top_k).collect();

    // Default threshold depends on the scoring mode. The contract distinguishes
    // reranker logits (default 0.0) from cosine similarity (default 0.5).
    let default_threshold = if reranker.is_some() { 0.0_f32 } else { 0.5_f32 };
    let threshold = args.min_score.unwrap_or(default_threshold);

    if args.strict {
        trimmed.retain(|s| s.score >= threshold);
        if trimmed.is_empty() {
            return Err(TomeError::QueryNoResultsStrict { threshold });
        }
    }

    // Even without `--strict`, the JSON `threshold_passed` field reflects
    // whether every returned row meets the (possibly default) threshold.
    let threshold_passed = trimmed.iter().all(|s| s.score >= threshold);
    let scoring = if reranker.is_some() {
        SCORING_RERANKED
    } else {
        SCORING_SIMILARITY
    };

    let home = std::env::var_os("HOME").map(PathBuf::from);

    match mode {
        Mode::Human => emit_human(
            &trimmed,
            scoring,
            reranker_drift.as_deref(),
            home.as_deref(),
        ),
        Mode::Json => emit_json(
            &trimmed,
            scoring,
            threshold_passed,
            reranker_drift.as_deref(),
        ),
    }
}

/// Validate `--catalog` / `--plugin` against the on-disk catalog manifests.
/// Costs at most one TOML parse per registered catalog when a `--plugin`
/// filter is set; bounded and cheap relative to the query itself.
fn validate_filters(args: &QueryArgs, config: &Config) -> Result<(), TomeError> {
    if let Some(catalog) = args.catalog.as_deref()
        && !config.catalogs.contains_key(catalog)
    {
        return Err(TomeError::CatalogNotFound(catalog.to_owned()));
    }

    let Some(plugin) = args.plugin.as_deref() else {
        return Ok(());
    };

    let catalog_names: Vec<&str> = match args.catalog.as_deref() {
        Some(c) => vec![c],
        None => config.catalogs.keys().map(String::as_str).collect(),
    };

    for name in &catalog_names {
        let Some(entry) = config.catalogs.get(*name) else {
            continue;
        };
        let Some(manifest) = read_catalog_manifest(&entry.path) else {
            continue;
        };
        if manifest.plugins.iter().any(|p| p.name == plugin) {
            return Ok(());
        }
    }

    // Scope the error message: when both filters were given, the
    // `<catalog>/<plugin>` form is the precise identity. Otherwise the bare
    // plugin name is enough — there is no catalog scope to attach.
    let message = match args.catalog.as_deref() {
        Some(c) => format!("{c}/{plugin}"),
        None => plugin.to_owned(),
    };
    Err(TomeError::PluginNotFound(message))
}

/// Run drift detection. Embedder drift converts to a hard error; reranker
/// drift returns `Ok(Some(label))` for the caller to surface.
fn check_drift(conn: &rusqlite::Connection) -> Result<Option<String>, TomeError> {
    let e = embedder_entry();
    let r = reranker_entry();
    let embedder_ident = ModelIdent {
        name: e.name.to_owned(),
        version: e.version.to_owned(),
    };
    let reranker_ident = ModelIdent {
        name: r.name.to_owned(),
        version: r.version.to_owned(),
    };
    match meta::detect_drift(conn, &embedder_ident, &reranker_ident)? {
        DriftStatus::None => Ok(None),
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            Err(TomeError::EmbedderNameDrift { stored, configured })
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            Err(TomeError::EmbedderVersionDrift { stored, configured })
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            Ok(Some(format!("stored={stored}, configured={configured}")))
        }
    }
}

fn emit_human(
    results: &[Scored],
    scoring: &str,
    reranker_drift: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<(), TomeError> {
    // Stderr-only notices first so structured stdout stays clean even when
    // a banner / warning is rendered.
    if scoring == SCORING_SIMILARITY {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "(reranker disabled — showing embedding similarity)");
    }
    if let Some(drift) = reranker_drift {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(
            err,
            "{} reranker drift detected — {drift}",
            colour::warning("warning:")
        );
    }

    let mut out = std::io::stdout().lock();
    if results.is_empty() {
        writeln!(out, "No results.")?;
        return Ok(());
    }

    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Score").set_alignment(CellAlignment::Right),
        Cell::new("Catalog"),
        Cell::new("Plugin"),
        Cell::new("Skill"),
        Cell::new("Version"),
        Cell::new("Path"),
    ]);

    for s in results {
        let c = &s.candidate;
        table.add_row(vec![
            Cell::new(format_score(s.score)).set_alignment(CellAlignment::Right),
            Cell::new(&c.catalog),
            Cell::new(&c.plugin),
            Cell::new(&c.name),
            Cell::new(&c.plugin_version),
            Cell::new(shorten_home(&c.path, home)),
        ]);
    }

    writeln!(out, "{table}")?;
    Ok(())
}

fn emit_json(
    results: &[Scored],
    scoring: &'static str,
    threshold_passed: bool,
    reranker_drift: Option<&str>,
) -> Result<(), TomeError> {
    let rows: Vec<JsonResult<'_>> = results
        .iter()
        .map(|s| JsonResult {
            catalog: &s.candidate.catalog,
            plugin: &s.candidate.plugin,
            skill: &s.candidate.name,
            plugin_version: &s.candidate.plugin_version,
            score: s.score,
            // JSON keeps the full path — the contract spells this out
            // explicitly. The `~` shorthand is human-mode only.
            path: &s.candidate.path,
            scoring,
        })
        .collect();

    let env = JsonEnvelope {
        scoring,
        threshold_passed,
        results: rows,
        reranker_drift,
    };
    output::write_json(&env)
}

#[derive(Serialize)]
struct JsonEnvelope<'a> {
    scoring: &'a str,
    threshold_passed: bool,
    results: Vec<JsonResult<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reranker_drift: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonResult<'a> {
    catalog: &'a str,
    plugin: &'a str,
    skill: &'a str,
    plugin_version: &'a str,
    score: f32,
    path: &'a str,
    scoring: &'a str,
}

/// Format a score with four decimals. Reranker logits can be negative or
/// exceed 1; similarity scores live in `[-1, 1]`. The contract example
/// shows `{:.4}` so we stay consistent regardless.
fn format_score(score: f32) -> String {
    format!("{score:.4}")
}

/// Replace `$HOME` with `~` when `path` is under the user's home directory.
/// Falls back to the path verbatim on any non-prefix or missing-home case.
/// Inline so we avoid pulling in a new crate; the rule is the standard
/// shell shorthand.
fn shorten_home(path: &str, home: Option<&std::path::Path>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    let home_str = home.to_string_lossy();
    if home_str.is_empty() {
        return path.to_owned();
    }
    if let Some(rest) = path.strip_prefix(home_str.as_ref())
        && (rest.starts_with('/') || rest.is_empty())
    {
        return format!("~{rest}");
    }
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_score_uses_four_decimals_for_positive() {
        assert_eq!(format_score(3.123_456_7), "3.1235");
    }

    #[test]
    fn format_score_uses_four_decimals_for_negative_logits() {
        // Reranker logits can be < 0; padding still applies.
        assert_eq!(format_score(-1.2), "-1.2000");
    }

    #[test]
    fn format_score_handles_zero() {
        assert_eq!(format_score(0.0), "0.0000");
    }

    #[test]
    fn shorten_home_replaces_prefix() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice/.local/share/tome/foo/SKILL.md", Some(home));
        assert_eq!(got, "~/.local/share/tome/foo/SKILL.md");
    }

    #[test]
    fn shorten_home_leaves_unrelated_path_alone() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/etc/hosts", Some(home));
        assert_eq!(got, "/etc/hosts");
    }

    #[test]
    fn shorten_home_only_replaces_at_boundary() {
        // `/Users/alice-other` must NOT be shortened to `~-other`.
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice-other/foo", Some(home));
        assert_eq!(got, "/Users/alice-other/foo");
    }

    #[test]
    fn shorten_home_handles_exact_home() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice", Some(home));
        assert_eq!(got, "~");
    }

    #[test]
    fn shorten_home_returns_input_when_home_unset() {
        let got = shorten_home("/Users/alice/foo", None);
        assert_eq!(got, "/Users/alice/foo");
    }
}
