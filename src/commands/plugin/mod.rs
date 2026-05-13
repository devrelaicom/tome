//! Dispatcher for `tome plugin <subcommand>` plus shared helpers.
//!
//! Slice 1b of Phase 3 (User Story 1). Adds the non-interactive surface
//! (`enable`, `list`, `show`) over the already-merged `plugin::lifecycle`
//! library. The `disable` and bare-interactive forms belong to US2.

mod disable;
mod enable;
mod interactive;
mod list;
mod show;

use crate::cli::PluginCommand;
use crate::error::TomeError;
use crate::output::Mode;

pub fn run(cmd: PluginCommand, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        PluginCommand::Enable(args) => enable::run(args, mode),
        PluginCommand::Disable(args) => disable::run(args, mode),
        PluginCommand::List(args) => list::run(args, mode),
        PluginCommand::Show(args) => show::run(args, mode),
    }
}

/// Bare `tome plugin` — interactive catalog → plugin → action browse flow.
/// Spec: `contracts/plugin-commands.md` §"`tome plugin` (no subcommand —
/// interactive)"; FR-050 / FR-051.
pub fn run_interactive(mode: Mode) -> Result<(), TomeError> {
    interactive::run(mode)
}

// ---------------------------------------------------------------------------
// Helpers shared across enable / list / show.
// ---------------------------------------------------------------------------

use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelManifest};
use crate::paths::Paths;

/// Returns `Ok(true)` iff a parseable `manifest.json` for `entry` exists
/// under `paths.models_dir`. Mirror of the private helper in
/// `plugin::lifecycle::model_manifest_ok`; duplicated here so the CLI layer
/// can decide whether to prompt-and-download before constructing the
/// embedder. A read or parse failure is treated as "not installed".
pub(crate) fn model_manifest_ok(paths: &Paths, entry: &ModelEntry) -> bool {
    let Ok(manifest_path) = paths.model_manifest(entry.name) else {
        return false;
    };
    if !manifest_path.is_file() {
        return false;
    }
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return false;
    };
    serde_json::from_slice::<ModelManifest>(&bytes).is_ok()
}

/// Enumerate registry entries whose on-disk manifest is missing or
/// unreadable. Returned in registry order.
pub(crate) fn missing_models(paths: &Paths) -> Vec<&'static ModelEntry> {
    MODEL_REGISTRY
        .iter()
        .filter(|e| !model_manifest_ok(paths, e))
        .collect()
}

/// `MetaSeed` values matching the `MODEL_REGISTRY` embedder / reranker.
/// Wrapping access keeps the CLI from hard-coding indices.
pub fn registry_seeds() -> (crate::index::MetaSeed, crate::index::MetaSeed) {
    use crate::embedding::registry::ModelKind;
    let mut embedder = None;
    let mut reranker = None;
    for entry in MODEL_REGISTRY {
        match entry.kind {
            ModelKind::Embedder if embedder.is_none() => {
                embedder = Some(crate::index::MetaSeed {
                    name: entry.name.to_owned(),
                    version: entry.version.to_owned(),
                });
            }
            ModelKind::Reranker if reranker.is_none() => {
                reranker = Some(crate::index::MetaSeed {
                    name: entry.name.to_owned(),
                    version: entry.version.to_owned(),
                });
            }
            _ => {}
        }
    }
    (
        embedder.expect("MODEL_REGISTRY must declare exactly one embedder"),
        reranker.expect("MODEL_REGISTRY must declare exactly one reranker"),
    )
}

/// Pick the embedder entry from the registry.
pub(crate) fn embedder_entry() -> &'static ModelEntry {
    use crate::embedding::registry::ModelKind;
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, ModelKind::Embedder))
        .expect("MODEL_REGISTRY must declare exactly one embedder")
}

/// Pick the reranker entry from the registry. Companion to
/// [`embedder_entry`]. Both `enable` and `query` need this — keeping them in
/// one place avoids `MODEL_REGISTRY` scanning drift between sites.
pub(crate) fn reranker_entry() -> &'static ModelEntry {
    use crate::embedding::registry::ModelKind;
    MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, ModelKind::Reranker))
        .expect("MODEL_REGISTRY must declare exactly one reranker")
}

/// Render `bytes` as a human-readable MiB string with no fractional digits.
/// Inline helper so we avoid pulling in `humansize` or similar (T074 says
/// no new dependencies).
pub(crate) fn human_mb(bytes: u64) -> String {
    let mb = (bytes as f64 / 1_048_576.0).round() as u64;
    format!("{mb} MB")
}

/// Human-relative duration from `then` to "now", bucketed as:
/// * < 60s   → "just now"
/// * < 60m   → "Xm ago"
/// * < 24h   → "Xh ago"
/// * else    → "Xd ago"
///
/// Falls back to "—" on a parse failure for the input timestamp string.
pub(crate) fn human_relative(then_rfc3339: &str) -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    let Ok(then) = OffsetDateTime::parse(then_rfc3339, &Rfc3339) else {
        return "—".to_owned();
    };
    let now = OffsetDateTime::now_utc();
    let delta = now - then;
    let secs = delta.whole_seconds();
    if secs < 0 {
        return "just now".to_owned();
    }
    if secs < 60 {
        return "just now".to_owned();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

/// Resolve a plugin directory from `<catalog>/<plugin>`. Single source of
/// truth lives in [`crate::plugin::lifecycle::resolve_plugin_dir`] — re-exported
/// here so the CLI handlers don't reach across module boundaries for it.
pub(crate) use crate::plugin::lifecycle::resolve_plugin_dir;

/// Open the index DB read-only-ish using the registry-derived seeds. We
/// re-use [`crate::index::open`] which is idempotent on a re-open.
pub(crate) fn open_index_for_read(paths: &Paths) -> Result<rusqlite::Connection, TomeError> {
    let (embedder, reranker) = registry_seeds();
    crate::index::open(
        &paths.index_db,
        &crate::index::OpenOptions { embedder, reranker },
    )
}

/// Per-plugin index aggregate used by `list` and `show`. None of the fields
/// require an index to exist on disk; absent rows collapse to `(0, 0, None)`.
#[derive(Debug, Clone, Default)]
pub(crate) struct IndexAggregate {
    pub total: i64,
    pub enabled: i64,
    pub last_indexed_at: Option<String>,
}

/// Aggregate the `skills` rows for one plugin. Returns zero counts on a
/// fresh index (no row inserted yet).
pub(crate) fn aggregate_for_plugin(
    conn: &rusqlite::Connection,
    catalog: &str,
    plugin: &str,
) -> Result<IndexAggregate, TomeError> {
    let result: (i64, i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(enabled), 0),
                    MAX(indexed_at)
             FROM skills
             WHERE catalog = ?1 AND plugin = ?2",
            rusqlite::params![catalog, plugin],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("aggregate_for_plugin: {e}")))?;
    Ok(IndexAggregate {
        total: result.0,
        enabled: result.1,
        last_indexed_at: result.2,
    })
}

/// Re-export of the catalog-manifest reader. Lives in `catalog::manifest`
/// next to the parser; surfaced here so the CLI handlers can keep using a
/// short path without crossing module boundaries.
pub(crate) use crate::catalog::manifest::read_catalog_manifest;
