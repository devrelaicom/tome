//! Dispatcher for `tome plugin <subcommand>` plus shared helpers.
//!
//! Slice 1b of Phase 3 (User Story 1). Adds the non-interactive surface
//! (`enable`, `list`, `show`) over the already-merged `plugin::lifecycle`
//! library. The `disable` and bare-interactive forms belong to US2.

mod convert;
mod create;
mod disable;
mod enable;
mod interactive;
mod lint;
mod list;
mod show;

use crate::cli::PluginCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(cmd: PluginCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        PluginCommand::Enable(args) => enable::run(args, scope, mode),
        PluginCommand::Disable(args) => disable::run(args, scope, mode),
        PluginCommand::List(args) => list::run(args, scope, mode),
        PluginCommand::Show(args) => show::run(args, scope, mode),
        PluginCommand::Create(args) => create::run(args, scope, mode),
        PluginCommand::Convert(args) => convert::run(args, scope, mode),
        PluginCommand::Lint(args) => lint::run(args, scope, mode),
    }
}

/// Bare `tome plugin` — interactive catalog → plugin → action browse flow.
/// Spec: `contracts/plugin-commands.md` §"`tome plugin` (no subcommand —
/// interactive)"; FR-050 / FR-051.
pub fn run_interactive(scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    interactive::run(scope, mode)
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

/// `MetaSeed` values matching the `MODEL_REGISTRY` embedder / reranker /
/// summariser. Wrapping access keeps the CLI from hard-coding indices.
/// Phase 4 / F9 grows the tuple to three elements alongside the
/// `OpenOptions` summariser field; older callers that destructured the
/// two-tuple shape need a trivial update.
pub fn registry_seeds() -> (
    crate::index::MetaSeed,
    crate::index::MetaSeed,
    crate::index::MetaSeed,
) {
    use crate::embedding::registry::ModelKind;
    let mut embedder = None;
    let mut reranker = None;
    let mut summariser = None;
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
            ModelKind::Summariser if summariser.is_none() => {
                summariser = Some(crate::index::MetaSeed {
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
        summariser.expect("MODEL_REGISTRY must declare exactly one summariser"),
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

pub(crate) use crate::presentation::format::human_mb;

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
pub(crate) fn open_index_for_read(
    paths: &Paths,
    _scope: &crate::workspace::Scope,
) -> Result<rusqlite::Connection, TomeError> {
    // Phase 3 slice F5: read paths use the dedicated read-only handle
    // when possible. The bootstrap / migration / WAL pragmas in
    // `index::open` only matter for writers; consumers of this helper
    // (`tome plugin list`, `tome plugin show`, `tome query`, the
    // interactive flow) never mutate state, so a read-only handle is
    // correct *and* immune to the writer's lockfile by SQLite's MVCC
    // contract.
    //
    // Edge case: on a fresh install the DB file doesn't exist yet.
    // `open_read_only` (using `SQLITE_OPEN_READ_ONLY`) refuses to
    // create one. Phase 2's read paths got the file-on-first-touch
    // bootstrap for free because they used the write-capable
    // `index::open`. Preserve that behaviour: when the file is
    // missing, fall through to the bootstrap path once (which creates
    // an empty DB + meta seeds), then re-open read-only. Subsequent
    // reads take the fast path. The bootstrap connection is dropped
    // immediately — the read-only handle is what the caller actually
    // queries.
    let db_path = paths.index_db.clone();
    if !db_path.is_file() {
        let (embedder, reranker, summariser) = registry_seeds();
        let _bootstrap = crate::index::open(
            &db_path,
            &crate::index::OpenOptions {
                embedder,
                reranker,
                summariser,
            },
        )?;
    }
    crate::index::open_read_only(&db_path)
}

/// Per-plugin index aggregate used by `list` and `show`. None of the fields
/// require an index to exist on disk; absent rows collapse to `(0, 0, None)`.
#[derive(Debug, Clone, Default)]
pub(crate) struct IndexAggregate {
    pub total: i64,
    pub enabled: i64,
    pub last_indexed_at: Option<String>,
}

/// Phase 5 / US5.b — per-kind counts for one plugin in one workspace.
/// `plugin list` renders these as `(<n> skills, <m> commands)` per
/// `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin list`.
/// Both fields count enrolled-in-workspace entries (via the
/// `workspace_skills` junction) — same definition as the existing
/// `IndexAggregate.enabled` field, but split by kind.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PerKindCounts {
    pub skills: u32,
    pub commands: u32,
    /// Phase 6: agent-kind entries enrolled in the workspace. Always
    /// non-searchable, never prompts (entry-schema-p6.md).
    pub agents: u32,
}

pub(crate) fn per_kind_counts_for_plugin(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<PerKindCounts, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.kind, COUNT(*)
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.catalog = ?1 AND s.plugin = ?2 AND w.name = ?3
             GROUP BY s.kind",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare per_kind_counts: {e}"))
        })?;
    let rows = stmt
        .query_map(rusqlite::params![catalog, plugin, workspace_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query per_kind_counts: {e}"))
        })?;
    let mut out = PerKindCounts::default();
    for r in rows {
        let (kind_text, n) = r.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("collect per_kind_counts row: {e}"))
        })?;
        // R-m2 (US5.c): SQLite COUNT(*) is non-negative; the prior
        // `n.max(0)` defensive clamp is unreachable.
        let n_u32 = u32::try_from(n).unwrap_or(u32::MAX);
        // M-3 (Polish): canonical EntryKind dispatch over stringly-
        // typed match — surfaces schema drift as
        // IndexIntegrityCheckFailure rather than silently
        // undercounting via `_ => {}`.
        let kind = kind_text
            .parse::<crate::plugin::identity::EntryKind>()
            .map_err(|msg| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "unknown kind `{kind_text}` in per_kind_counts: {msg}"
                ))
            })?;
        match kind {
            crate::plugin::identity::EntryKind::Skill => out.skills = n_u32,
            crate::plugin::identity::EntryKind::Command => out.commands = n_u32,
            crate::plugin::identity::EntryKind::Agent => out.agents = n_u32,
        }
    }
    Ok(out)
}

/// Aggregate the `skills` rows for one plugin. Returns zero counts on a
/// fresh index (no row inserted yet). Phase 4 / F11a: `enabled` now means
/// "joined to the workspace named `workspace_name` via `workspace_skills`",
/// sourced from the resolved scope.
pub(crate) fn aggregate_for_plugin(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<IndexAggregate, TomeError> {
    let result: (i64, i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0),
                    MAX(indexed_at)
             FROM skills AS s
             LEFT JOIN workspace_skills AS ws
                    ON ws.skill_id = s.id
                   AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?3)
             WHERE s.catalog = ?1 AND s.plugin = ?2",
            rusqlite::params![catalog, plugin, workspace_name],
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
