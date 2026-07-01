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

/// Returns `Ok(true)` iff a parseable `manifest.toml` for `entry` exists
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
    ModelManifest::from_toml_slice(&manifest_path, &bytes).is_ok()
}

/// Enumerate registry entries whose on-disk manifest is missing or
/// unreadable. Returned in registry order.
///
/// NOTE: this walks the WHOLE registry. Consumers that gate on "the models
/// THIS index actually needs" (B2) must use [`missing_models_for_profile`]
/// instead — after Phase 2 the registry carries every profile's embedder +
/// reranker, so the whole-registry walk would demand downloading models the
/// active profile never uses. `query.rs` is exempt: it name-matches a single
/// resolved entry, so the whole-registry set is filtered down to one anyway.
pub(crate) fn missing_models(paths: &Paths) -> Vec<&'static ModelEntry> {
    MODEL_REGISTRY
        .iter()
        .filter(|e| !model_manifest_ok(paths, e))
        .collect()
}

/// B2: the missing-model set scoped to the ACTIVE profile's `[embedder,
/// reranker]` only (the summariser is handled by its own US4 download path).
/// `conn` resolves the active profile from `meta`. Used by the `plugin enable`
/// download prompt so a workspace only ever pulls the models its profile uses.
pub(crate) fn missing_models_for_profile(
    paths: &Paths,
    conn: &rusqlite::Connection,
) -> Result<Vec<&'static ModelEntry>, TomeError> {
    let embedder = crate::index::meta::active_embedder(conn)?;
    let reranker = crate::index::meta::active_reranker(conn)?;
    Ok([embedder, reranker]
        .into_iter()
        .filter(|e| !model_manifest_ok(paths, e))
        .collect())
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
    use crate::embedding::profile::{Profile, embedder_for, reranker_for};
    let e = embedder_for(Profile::DEFAULT);
    let r = reranker_for(Profile::DEFAULT);
    let s = crate::summarise::registry::summariser_entry();
    let seed = |m: &crate::embedding::registry::ModelEntry| crate::index::MetaSeed {
        name: m.name.to_owned(),
        version: m.version.to_owned(),
    };
    (seed(e), seed(r), seed(s))
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
    human_relative_at(then_rfc3339, time::OffsetDateTime::now_utc())
}

/// [`human_relative`] with the reference "now" supplied by the caller so the
/// bucketing is deterministically testable (production calls pass
/// `OffsetDateTime::now_utc()` via `human_relative`). A `then` in the future
/// relative to `now` collapses to "just now" rather than emitting a negative
/// duration.
pub(crate) fn human_relative_at(then_rfc3339: &str, now: time::OffsetDateTime) -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    let Ok(then) = OffsetDateTime::parse(then_rfc3339, &Rfc3339) else {
        return "—".to_owned();
    };
    let delta = now - then;
    let secs = delta.whole_seconds();
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

/// Best-effort "last upstream change" timestamp for a plugin, computed at
/// DISPLAY time (never stored → no schema/migration cost) by asking the
/// catalog's content-addressed clone for the committer date of the most
/// recent commit that touched the plugin's subtree.
///
/// `clone_dir` is the catalog checkout root (`paths.cache_dir_for(url)`);
/// `rel_source` is the plugin's manifest-declared `source` sub-path relative
/// to that root (the same value `plugin list` joins to build `plugin_dir`).
/// The `git log` runs against `clone_dir` scoped to `rel_source` so the
/// timestamp reflects that plugin's history, not the whole catalog's.
///
/// Returns `None` — degrade to the `indexed_at` fallback — when the clone
/// isn't present / isn't a git repo, the subtree has no history (a shallow
/// clone whose single commit didn't touch it, or an unpublished local
/// catalog), or `git` fails for any reason. Failures never propagate: this
/// mirrors the read-only, no-lock contract of `list`/`show` and must not
/// change the command's exit code. All captured `git` output is scrubbed for
/// credentials inside `catalog::git` before it could reach any surface.
pub(crate) fn last_upstream_change_at_display(
    clone_dir: &std::path::Path,
    catalog: &str,
    rel_source: &str,
) -> Option<time::OffsetDateTime> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    // A missing `.git` means this is not a real clone (e.g. a fixture or a
    // local-path catalog) — skip the shell-out entirely rather than let `git`
    // walk up to some ancestor repository.
    if !clone_dir.join(".git").exists() {
        return None;
    }
    let git = crate::catalog::git::Git::new(catalog);
    // `Ok(None)` = no history for the subtree; `Err` = git blew up. Both
    // degrade to `None` here (best-effort display field).
    let iso = git.last_commit_iso(clone_dir, rel_source).ok().flatten()?;
    OffsetDateTime::parse(&iso, &Rfc3339).ok()
}

/// [`last_upstream_change_at_display`] for callers that hold the plugin
/// identity + a read handle rather than a pre-split `(clone_dir, source)` pair
/// (e.g. `plugin show`, whose `resolve_plugin_dir` returns the JOINED
/// `clone_dir.join(source)`). Recovers the `(clone_dir, source)` split the same
/// way `resolve_plugin_dir` does — enrolment URL → `cache_dir_for(url)` for the
/// clone root, catalog manifest → the plugin's `source` sub-path — then routes
/// through the SAME `.git`-guarded [`last_upstream_change_at_display`] `list`
/// uses.
///
/// Routing through the shared guarded helper (rather than running `git log`
/// from inside the joined plugin dir) is load-bearing: `git log` WALKS UP to
/// find an enclosing `.git`, so a plugin dir that is not itself inside a real
/// clone but sits under an unrelated ancestor repository (e.g. a `$HOME`
/// dotfiles repo) would otherwise report that ANCESTOR's HEAD timestamp — a
/// silently-wrong value. The `clone_dir.join(".git").exists()` guard in the
/// shared helper makes `show` and `list` resolve the same fact by the same
/// mechanism (both return `None` → `—`).
///
/// Same best-effort contract: any resolution/git failure degrades to `None`;
/// nothing propagates (the read-only, no-lock display path must not change the
/// command's exit code).
pub(crate) fn last_upstream_change_for_id(
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace_name: &str,
    id: &crate::plugin::PluginId,
) -> Option<time::OffsetDateTime> {
    // Enrolment URL → content-addressed clone root (mirrors resolve_plugin_dir).
    let enrolment = crate::index::workspace_catalogs::find(conn, workspace_name, &id.catalog)
        .ok()
        .flatten()?;
    let clone_dir = paths.cache_dir_for(&enrolment.url);

    // Catalog manifest → the plugin's declared `source` sub-path; fall back to
    // the plugin name (the same fallback resolve_plugin_dir applies when the
    // manifest is absent/unreadable).
    let rel_source = crate::catalog::manifest::read_catalog_manifest(&clone_dir)
        .and_then(|m| {
            m.plugins
                .iter()
                .find(|p| p.name == id.plugin)
                .map(|p| p.source.clone())
        })
        .unwrap_or_else(|| id.plugin.clone());

    last_upstream_change_at_display(&clone_dir, &id.catalog, &rel_source)
}

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
                profile: None,
            },
        )?;
    }
    crate::index::open_read_only(&db_path)
}

/// Best-effort read of a plugin's `plugin_version` from the index, for the
/// catalog-attributed telemetry events (Phase 10 / US4, FR-056). Opens the
/// central index READ-ONLY with NO advisory lock (NFR-009) and returns the
/// first matching `skills` row's `plugin_version` (every row for one plugin
/// carries the same manifest version).
///
/// Infallible + best-effort, matching the silent telemetry path: any failure
/// (missing DB, no rows, query error) yields an empty string rather than
/// propagating — the attributed event still fires with a blank version rather
/// than crashing or altering the user's exit code. The artefact version is a
/// PUBLISHED manifest value (the FR-059 carve-out), never a user secret.
pub(crate) fn attributed_plugin_version(
    paths: &Paths,
    scope: &crate::workspace::Scope,
    id: &crate::plugin::PluginId,
) -> String {
    let Ok(conn) = crate::index::open_read_only(&paths.index_db) else {
        return String::new();
    };
    crate::index::skills::list_for_plugin(&conn, scope.name().as_str(), &id.catalog, &id.plugin)
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .map(|row| row.plugin_version)
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    /// Fixed reference "now" so the relative-time bucketing is deterministic.
    /// Parsed via the same `Rfc3339` path production uses (the `time` crate's
    /// `macros` feature is not enabled, so `datetime!` is unavailable).
    fn now() -> OffsetDateTime {
        OffsetDateTime::parse("2026-01-15T12:00:00Z", &Rfc3339).expect("fixed reference now")
    }

    #[test]
    fn human_relative_buckets_by_elapsed_time() {
        let n = now();
        // < 60s → "just now"
        assert_eq!(
            human_relative_at("2026-01-15T11:59:30Z", n),
            "just now",
            "30s ago",
        );
        assert_eq!(
            human_relative_at("2026-01-15T12:00:00Z", n),
            "just now",
            "exactly now",
        );
        // < 60m → "Xm ago"
        assert_eq!(
            human_relative_at("2026-01-15T11:57:00Z", n),
            "3m ago",
            "3 minutes ago",
        );
        // < 24h → "Xh ago"
        assert_eq!(
            human_relative_at("2026-01-15T07:00:00Z", n),
            "5h ago",
            "5 hours ago",
        );
        // ≥ 24h → "Xd ago"
        assert_eq!(
            human_relative_at("2026-01-12T12:00:00Z", n),
            "3d ago",
            "3 days ago",
        );
    }

    #[test]
    fn human_relative_future_timestamp_is_just_now() {
        // A `then` after `now` (clock skew) collapses to "just now", never a
        // negative duration.
        assert_eq!(human_relative_at("2026-01-15T12:05:00Z", now()), "just now");
    }

    #[test]
    fn human_relative_unparsable_is_dash() {
        assert_eq!(human_relative_at("not-a-timestamp", now()), "—");
    }

    /// `registry_seeds()` must yield the DEFAULT (Medium) profile's embedder
    /// and reranker names, not the first embedder/reranker in registry order.
    #[test]
    fn registry_seeds_yields_medium_profile_models() {
        let (embedder_seed, reranker_seed, _summariser_seed) = registry_seeds();
        assert_eq!(
            embedder_seed.name, "bge-base-en-v1.5",
            "DEFAULT profile embedder must be bge-base-en-v1.5"
        );
        assert_eq!(
            reranker_seed.name, "bge-reranker-large",
            "DEFAULT profile reranker must be bge-reranker-large"
        );
    }
}
