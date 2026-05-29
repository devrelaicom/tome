//! Enable / disable orchestrator for a single plugin.
//!
//! This module composes the plugin-metadata parsers (`plugin::manifest`,
//! `plugin::frontmatter`), the index layer (`index::open`, `index::acquire_lock`,
//! `index::enable_plugin_atomic`, `index::mark_all_disabled_for_plugin`), and
//! the embedding model presence check (`embedding::registry`,
//! `embedding::download`) into the contract described in
//! `specs/002-phase-2-plugins-index/contracts/plugin-commands.md` (lines 9–97).
//!
//! No CLI / IO / prompt code lives here — slice 1b wires `tome plugin
//! {enable,disable}` on top of this surface. The TTY-versus-non-TTY decision
//! for "is it OK to download a missing model" is reduced to the
//! [`LifecycleDeps::allow_model_download`] boolean so this module remains
//! testable without a terminal.
//!
//! Spec: FR-004, FR-005, FR-006, FR-013a/b/c, FR-024, FR-025, FR-053;
//! contracts/plugin-commands.md §1–§2.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::catalog::git::was_cancelled;
use crate::config::Config;
use crate::embedding::download::download_model;
use crate::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelManifest};
use crate::embedding::{Embedder, ModelKind};
use crate::error::{PluginState, TomeError};
use crate::index::skills::{EnableSummary, PendingSkill};
use crate::index::{
    self, MetaSeed, OpenOptions, ReindexSummary, acquire_lock, delete_by_plugin,
    enable_plugin_atomic, mark_all_disabled_for_plugin, reindex_plugin_atomic,
};
use crate::paths::Paths;
use crate::plugin::components::{list_agent_files, list_command_files};
use crate::plugin::frontmatter::{
    FrontmatterError, ParsedSkill, parse_skill_frontmatter, validate_argument_names,
};
use crate::plugin::identity::{EntryKind, PluginId};
use crate::plugin::manifest::{manifest_path_for, parse_plugin_manifest};

/// Result of a successful enable.
#[derive(Debug, Clone)]
pub struct EnableOutcome {
    pub plugin: PluginId,
    pub summary: EnableSummary,
    pub duration: Duration,
    /// Human-readable warnings collected during the walk. Each entry is a
    /// stable diagnostic the CLI layer surfaces on stderr — FR-011 / FR-012
    /// fallback notices and FR-013c skipped-skill notices.
    pub warnings: Vec<String>,
}

/// Result of a successful disable.
#[derive(Debug, Clone)]
pub struct DisableOutcome {
    pub plugin: PluginId,
    pub skills_retained: u32,
    pub duration: Duration,
}

/// Result of a successful reindex of one plugin.
#[derive(Debug, Clone)]
pub struct ReindexOutcome {
    pub plugin: PluginId,
    pub summary: ReindexSummary,
    pub duration: Duration,
    /// Same shape as `EnableOutcome::warnings` — FR-011 / FR-012 / FR-013c
    /// notices accumulated while walking the on-disk skills.
    pub warnings: Vec<String>,
}

/// Inputs to [`enable`]. Kept as a single struct so the CLI wrapper that
/// constructs `embedder`, `embedder_seed`, `reranker_seed`, and the TTY
/// decision can pass them through unchanged.
pub struct LifecycleDeps<'a> {
    pub paths: &'a Paths,
    /// The active workspace scope. Phase 4 / F11a routes every
    /// lifecycle write through the `workspace_skills` junction keyed on
    /// `scope.name()`; the central index database stays shared across
    /// every workspace, the per-workspace dimension is the junction row.
    pub scope: &'a crate::workspace::Scope,
    pub config: &'a Config,
    pub embedder: &'a dyn Embedder,
    pub embedder_seed: MetaSeed,
    pub reranker_seed: MetaSeed,
    /// Phase 4 / F9 added a third runtime identity row to the index
    /// `meta` table. Lifecycle callers thread the configured summariser
    /// identity through alongside the embedder + reranker seeds; the
    /// value is written into `meta` on first bootstrap and consulted by
    /// drift detection thereafter.
    pub summariser_seed: MetaSeed,
    /// `true` when the CLI has confirmed (via TTY prompt) that Tome may
    /// download missing models. `false` is the non-TTY refusal contract —
    /// the function returns `ModelMissing` (exit 30) per plugin-commands.md
    /// step 4.
    pub allow_model_download: bool,
}

impl LifecycleDeps<'_> {
    /// The resolved workspace name as bound to SQL. Always valid — the
    /// inner [`crate::workspace::WorkspaceName`] is constructed by the
    /// resolver only after [`crate::workspace::WorkspaceName::parse`] or
    /// the membership check on the central DB has succeeded.
    pub fn workspace_name(&self) -> &str {
        self.scope.name().as_str()
    }
}

// -------------------------------------------------------------------------
// Public API
// -------------------------------------------------------------------------

/// Enable a plugin: walk its skills, embed-and-insert under one SQLite
/// transaction, and surface fallback / skipped-skill warnings.
///
/// The full contract is captured by `plugin-commands.md` §1. Atomic guarantee
/// (FR-004): on any failure after the lock is acquired, the on-disk index is
/// indistinguishable from its pre-call state.
pub fn enable(id: &PluginId, deps: &LifecycleDeps<'_>) -> Result<EnableOutcome, TomeError> {
    let started = Instant::now();
    let plugin_dir = resolve_plugin_dir(id, deps.config)?;

    // Step 2 — manifest parse. We don't *use* the parsed fields below (the
    // `plugin_version` we record per-skill is sourced from this manifest's
    // `version` field), but reading it early gives us the right exit code
    // (22) before we touch the index.
    let manifest_path = manifest_path_for(&plugin_dir);
    let manifest = parse_plugin_manifest(&manifest_path)?;
    let plugin_version = manifest
        .version
        .clone()
        .unwrap_or_else(|| "0.0.0".to_string());

    // Step 3 — already-enabled check. We open the DB read-only-ish (the
    // bootstrap is idempotent on re-open) and look for any enabled row.
    // Doing this before the lock is acquired keeps the contention surface
    // small: a quick check and bail.
    if any_skill_enabled(deps, id)? {
        return Err(TomeError::PluginAlreadyInState {
            plugin: id.to_string(),
            state: PluginState::Enabled,
        });
    }

    // Step 4 — model presence (T074).
    ensure_models_present(deps)?;

    // Step 5 — advisory lock. Held until step 10.
    let lock = acquire_lock(&deps.paths.index_lock.clone())?;

    // Run the rest under the lock; release explicitly on success, drop on
    // failure (Drop releases best-effort, matching the lock module's docs).
    let result = enable_locked(id, &plugin_dir, &plugin_version, deps);

    match result {
        Ok((summary, warnings)) => {
            lock.release()?;
            Ok(EnableOutcome {
                plugin: id.clone(),
                summary,
                duration: started.elapsed(),
                warnings,
            })
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

/// Disable a plugin: flip every `(catalog, plugin)` row's `enabled` column
/// to 0. Embeddings are retained for cheap re-enable (FR-005, FR-006).
///
/// The CLI layer is responsible for the confirmation prompt (and `--force`).
/// This function performs no prompting; it only mutates state.
pub fn disable(
    id: &PluginId,
    paths: &Paths,
    scope: &crate::workspace::Scope,
    config: &Config,
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
    summariser_seed: MetaSeed,
) -> Result<DisableOutcome, TomeError> {
    let started = Instant::now();
    // We still resolve the plugin directory to reject typos before touching
    // the index — same exit-code surface as enable.
    let _plugin_dir = resolve_plugin_dir(id, config)?;

    let lock = acquire_lock(&paths.index_lock.clone())?;
    let outcome = disable_locked(
        id,
        paths,
        scope,
        embedder_seed,
        reranker_seed,
        summariser_seed,
    );

    match outcome {
        Ok(skills_retained) => {
            lock.release()?;
            Ok(DisableOutcome {
                plugin: id.clone(),
                skills_retained,
                duration: started.elapsed(),
            })
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

/// Reindex one plugin: walk its on-disk skills, diff against the index,
/// re-embed only the modified ones (unless `force` is set), delete rows
/// for skills no longer on disk.
///
/// Drives the contracts at `contracts/reindex.md` and the per-plugin
/// branch of `contracts/catalog-extensions.md` §"tome catalog update". Both
/// consumers run inside the advisory lock; this function acquires it.
///
/// Pre-condition: the plugin is enabled. The caller is responsible for
/// filtering to enabled plugins before invoking — neither `catalog update`
/// nor `tome reindex` reindexes disabled rows.
pub fn reindex_plugin(
    id: &PluginId,
    deps: &LifecycleDeps<'_>,
    force: bool,
) -> Result<ReindexOutcome, TomeError> {
    let started = Instant::now();
    let plugin_dir = resolve_plugin_dir(id, deps.config)?;

    let manifest_path = manifest_path_for(&plugin_dir);
    let manifest = parse_plugin_manifest(&manifest_path)?;
    let plugin_version = manifest
        .version
        .clone()
        .unwrap_or_else(|| "0.0.0".to_string());

    let lock = acquire_lock(&deps.paths.index_lock.clone())?;
    let result = reindex_locked(id, &plugin_dir, &plugin_version, deps, force);

    match result {
        Ok((summary, warnings)) => {
            lock.release()?;
            info!(
                plugin = %id,
                added = summary.added,
                modified = summary.modified,
                removed = summary.removed,
                unchanged = summary.unchanged,
                force,
                "plugin reindex completed",
            );
            Ok(ReindexOutcome {
                plugin: id.clone(),
                summary,
                duration: started.elapsed(),
                warnings,
            })
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

/// Cascade-disable every plugin in `plugins` under one advisory-lock
/// acquisition. Phase 4 / F11a: removes each plugin's `workspace_skills`
/// enrolment rows for the resolved workspace `workspace_name` rather
/// than dropping the underlying `skills` + `skill_embeddings` rows
/// outright. The skill rows are retained so other workspaces enrolling
/// the same plugin (post-F11b multi-workspace catalog enrolment) still
/// see their data — this honours FR-383 across the workspace dimension.
/// Returns one `(plugin_name, rows_dropped)` pair per input plugin,
/// preserving input order — empty plugins still get a `0` entry so the
/// caller can join against its own list of plugins to emit per-plugin
/// telemetry.
///
/// Used by `tome catalog remove --force`. The single-lock-per-cascade
/// semantics match `contracts/catalog-extensions.md` §"tome catalog remove".
///
/// Unlike `auto_disable_orphan`, this function does NOT require an
/// `Embedder` — the cascade is pure deletion. The seeds are still required
/// because `index::open` validates them against the on-disk `meta` rows.
pub fn cascade_disable_for_catalog(
    paths: &Paths,
    workspace_name: &str,
    catalog: &str,
    plugins: &[String],
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
    summariser_seed: MetaSeed,
) -> Result<Vec<(String, u32)>, TomeError> {
    if plugins.is_empty() {
        return Ok(Vec::new());
    }
    let lock = acquire_lock(&paths.index_lock.clone())?;
    let result = (|| -> Result<Vec<(String, u32)>, TomeError> {
        let conn = index::open(
            &paths.index_db.clone(),
            &OpenOptions {
                embedder: embedder_seed,
                reranker: reranker_seed,
                summariser: summariser_seed,
            },
        )?;
        let mut breakdown: Vec<(String, u32)> = Vec::with_capacity(plugins.len());
        for plugin in plugins {
            let dropped = mark_all_disabled_for_plugin(&conn, workspace_name, catalog, plugin)?;
            breakdown.push((plugin.clone(), dropped));
        }
        Ok(breakdown)
    })();

    match result {
        Ok(breakdown) => {
            lock.release()?;
            let total: u32 = breakdown.iter().map(|(_, n)| *n).sum();
            info!(
                catalog,
                plugins_disabled = plugins.len(),
                rows_dropped = total,
                "catalog cascade disable completed",
            );
            Ok(breakdown)
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

/// De-index every row for a plugin whose `plugin.json` is gone post-refresh
/// (FR-033). Returns the number of dropped `skills` rows. The caller is
/// responsible for emitting the loud-warning stderr line.
///
/// Used by `tome catalog update`. NOT used by `tome reindex` — reindex on a
/// missing plugin reports `PluginNotFound` (exit 20), not a silent cascade.
pub fn auto_disable_orphan(id: &PluginId, deps: &LifecycleDeps<'_>) -> Result<u32, TomeError> {
    let lock = acquire_lock(&deps.paths.index_lock.clone())?;
    let result = auto_disable_locked(id, deps);

    match result {
        Ok(dropped) => {
            lock.release()?;
            warn!(
                plugin = %id,
                dropped,
                "plugin auto-disabled (manifest missing or unparsable)",
            );
            Ok(dropped)
        }
        Err(e) => {
            drop(lock);
            Err(e)
        }
    }
}

// -------------------------------------------------------------------------
// Private helpers
// -------------------------------------------------------------------------

/// Resolve `<catalog>/<plugin>` against the registry and on-disk cache.
///
/// Authoritative source is the catalog's `tome-catalog.toml`:
/// `entry.path.join(&plugins[].source)` for the entry whose `name` matches
/// `id.plugin`. The lookup is intentionally manifest-first so that catalogs
/// declaring nested layouts (e.g. `source = "./plugins/foo"`) work uniformly
/// across `enable`, `show`, and `list` (see also FR-008 and `query.md`).
///
/// When `tome-catalog.toml` is absent or unparsable the resolver falls back
/// to the flat layout `entry.path.join(&id.plugin)` — this preserves
/// back-compat for library callers that construct catalog roots without a
/// manifest (the `lifecycle.rs` in-module tests, hand-rolled fixtures, and
/// the "I cloned a plugin into a bare directory" recovery path).
pub fn resolve_plugin_dir(id: &PluginId, config: &Config) -> Result<PathBuf, TomeError> {
    let entry = config
        .catalogs
        .get(&id.catalog)
        .ok_or_else(|| TomeError::CatalogNotFound(id.catalog.clone()))?;

    let plugin_dir = match crate::catalog::manifest::read_catalog_manifest(&entry.path) {
        Some(manifest) => {
            let decl = manifest
                .plugins
                .iter()
                .find(|p| p.name == id.plugin)
                .ok_or_else(|| TomeError::PluginNotFound(id.to_string()))?;
            entry.path.join(&decl.source)
        }
        None => entry.path.join(&id.plugin),
    };

    if !plugin_dir.is_dir() {
        return Err(TomeError::PluginNotFound(id.to_string()));
    }
    Ok(plugin_dir)
}

/// Returns `true` when the index already contains at least one
/// `(catalog, plugin)` row enrolled in the resolved workspace via
/// `workspace_skills` (Phase 4 / F11a redefinition of "enabled").
fn any_skill_enabled(deps: &LifecycleDeps<'_>, id: &PluginId) -> Result<bool, TomeError> {
    let conn = index::open(
        &deps.paths.index_db.clone(),
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
            summariser: deps.summariser_seed.clone(),
        },
    )?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.catalog = ?1 AND s.plugin = ?2 AND w.name = ?3",
            rusqlite::params![id.catalog, id.plugin, deps.workspace_name()],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("check enabled skills: {e}")))?;
    Ok(count > 0)
}

/// Steps 6–9 of the enable contract — held under the advisory lock.
fn enable_locked(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    deps: &LifecycleDeps<'_>,
) -> Result<(EnableSummary, Vec<String>), TomeError> {
    if was_cancelled() {
        return Err(TomeError::Interrupted);
    }

    let mut conn = index::open(
        &deps.paths.index_db.clone(),
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
            summariser: deps.summariser_seed.clone(),
        },
    )?;

    let mut warnings: Vec<String> = Vec::new();
    let pending = collect_pending_skills(id, plugin_dir, plugin_version, &mut warnings)?;

    let embedder = deps.embedder;
    let workspace_name = deps.workspace_name();
    let summary = enable_plugin_atomic(&mut conn, workspace_name, &pending, |text| {
        // Cancellation is observed inside the embed loop too (handover
        // gotcha #3): each embed call peeks the SIGINT flag. The closure
        // returns `Err(TomeError::Interrupted)` which `enable_plugin_atomic`
        // propagates and the surrounding transaction rolls back.
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        embedder.embed(text)
    })?;

    info!(
        plugin = %id,
        total = summary.total_skills,
        newly = summary.newly_embedded,
        skipped = warnings.len(),
        "plugin enable completed",
    );

    Ok((summary, warnings))
}

/// Disable-locked branch — runs under the advisory lock.
fn disable_locked(
    id: &PluginId,
    paths: &Paths,
    scope: &crate::workspace::Scope,
    embedder_seed: MetaSeed,
    reranker_seed: MetaSeed,
    summariser_seed: MetaSeed,
) -> Result<u32, TomeError> {
    let conn = index::open(
        &paths.index_db.clone(),
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    let workspace_name = scope.name().as_str();

    // The contract requires "already-disabled" detection. Two cases
    // collapse into one PluginAlreadyInState: zero rows for the plugin
    // OR every row already disenrolled from the resolved workspace.
    let (total, enabled_count): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM skills AS s
             LEFT JOIN workspace_skills AS ws
                    ON ws.skill_id = s.id
                   AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?3)
             WHERE s.catalog = ?1 AND s.plugin = ?2",
            rusqlite::params![id.catalog, id.plugin, workspace_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count skills: {e}")))?;
    if total == 0 || enabled_count == 0 {
        return Err(TomeError::PluginAlreadyInState {
            plugin: id.to_string(),
            state: PluginState::Disabled,
        });
    }

    let affected = mark_all_disabled_for_plugin(&conn, workspace_name, &id.catalog, &id.plugin)?;
    Ok(affected)
}

/// Held under the advisory lock. Runs the on-disk walk, then dispatches
/// to `reindex_plugin_atomic`. The embedder closure peeks the SIGINT flag
/// per skill so a cancellation aborts the transaction.
fn reindex_locked(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    deps: &LifecycleDeps<'_>,
    force: bool,
) -> Result<(ReindexSummary, Vec<String>), TomeError> {
    if was_cancelled() {
        return Err(TomeError::Interrupted);
    }

    let mut conn = index::open(
        &deps.paths.index_db.clone(),
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
            summariser: deps.summariser_seed.clone(),
        },
    )?;

    let mut warnings: Vec<String> = Vec::new();
    let pending = collect_pending_skills(id, plugin_dir, plugin_version, &mut warnings)?;

    let embedder = deps.embedder;
    let workspace_name = deps.workspace_name();
    let summary = reindex_plugin_atomic(
        &mut conn,
        workspace_name,
        &id.catalog,
        &id.plugin,
        &pending,
        force,
        |text| {
            if was_cancelled() {
                return Err(TomeError::Interrupted);
            }
            embedder.embed(text)
        },
    )?;

    Ok((summary, warnings))
}

/// Held under the advisory lock. Drops every `(catalog, plugin)` row + its
/// embeddings. Idempotent — zero rows is fine.
fn auto_disable_locked(id: &PluginId, deps: &LifecycleDeps<'_>) -> Result<u32, TomeError> {
    let conn = index::open(
        &deps.paths.index_db.clone(),
        &OpenOptions {
            embedder: deps.embedder_seed.clone(),
            reranker: deps.reranker_seed.clone(),
            summariser: deps.summariser_seed.clone(),
        },
    )?;
    delete_by_plugin(&conn, &id.catalog, &id.plugin)
}

/// Walk every entry under `<plugin_dir>` — `skills/*/SKILL.md`,
/// `commands/*.md`, and (Phase 6 / US1) `agents/*.md` — and produce a
/// unified [`PendingSkill`] list keyed on the kind discriminator. Errors
/// per-file are funnelled consistently across the three surfaces:
///
/// * Delimiter failure → `SkillFrontmatterParseError` for skills/commands;
///   `AgentTranslationFailed` (exit 45) for agents (NFR-010 — a malformed
///   *recognised* agent structure fails loudly per entry-schema-p6.md §
///   "Indexing pipeline" step 1).
/// * YAML body failure → warning + skip (FR-013c) for skills/commands;
///   `AgentTranslationFailed` (exit 45) for agents.
/// * Illegal `arguments` name → `InvalidArgumentFrontmatter` (exit 29).
/// * IO failure → bubble as `TomeError::Io`.
fn collect_pending_skills(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<PendingSkill>, TomeError> {
    let mut pending: Vec<PendingSkill> = Vec::new();
    collect_skill_entries(id, plugin_dir, plugin_version, warnings, &mut pending)?;
    collect_command_entries(id, plugin_dir, plugin_version, warnings, &mut pending)?;
    collect_agent_entries(id, plugin_dir, plugin_version, warnings, &mut pending)?;
    Ok(pending)
}

/// Walk `<plugin_dir>/skills/*/SKILL.md` and append one [`PendingSkill`]
/// (kind = Skill) per parseable entry.
fn collect_skill_entries(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    warnings: &mut Vec<String>,
    pending: &mut Vec<PendingSkill>,
) -> Result<(), TomeError> {
    let skills_root = plugin_dir.join("skills");
    if !skills_root.is_dir() {
        debug!(
            plugin = %id,
            skills_dir = %skills_root.display(),
            "no skills directory; enabling zero rows",
        );
        return Ok(());
    }

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&skills_root) {
        Ok(it) => it
            .filter_map(|res| res.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => return Err(TomeError::Io(e)),
    };
    // Deterministic ordering across platforms / filesystems.
    entries.sort();

    for skill_dir in entries {
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let skill_file = skill_dir.join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }

        let dir_name = skill_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        match parse_one_entry(
            id,
            plugin_dir,
            plugin_version,
            &skill_file,
            &dir_name,
            EntryKind::Skill,
            warnings,
        )? {
            Some(p) => pending.push(p),
            None => continue,
        }
    }

    Ok(())
}

/// Walk `<plugin_dir>/commands/*.md` (flat, non-recursive) and append one
/// [`PendingSkill`] (kind = Command) per parseable entry.
fn collect_command_entries(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    warnings: &mut Vec<String>,
    pending: &mut Vec<PendingSkill>,
) -> Result<(), TomeError> {
    let commands = list_command_files(plugin_dir);
    if commands.is_empty() {
        debug!(
            plugin = %id,
            "no commands directory or no command files; enabling zero command rows",
        );
        return Ok(());
    }

    for cmd in commands {
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        match parse_one_entry(
            id,
            plugin_dir,
            plugin_version,
            &cmd.path,
            &cmd.name,
            EntryKind::Command,
            warnings,
        )? {
            Some(p) => pending.push(p),
            None => continue,
        }
    }

    Ok(())
}

/// Walk `<plugin_dir>/agents/*.md` (flat, non-recursive) and append one
/// [`PendingSkill`] (kind = Agent) per file. Phase 6 / US1.
///
/// Unlike skills/commands, a malformed agent file fails LOUDLY: per
/// `entry-schema-p6.md` § "Indexing pipeline" step 1 + NFR-010, an agent
/// whose frontmatter delimiters are absent or whose YAML body is invalid
/// surfaces as [`TomeError::AgentTranslationFailed`] (exit 45) rather than
/// the skill-style "abort the whole plugin" (delimiter) / "warn and skip"
/// (YAML body) split. Agents are not searchable and not user-invocable, so
/// argument-name validation does not apply. The clash-set (FR-072) is a
/// separate per-sync computation, not done here.
fn collect_agent_entries(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    warnings: &mut Vec<String>,
    pending: &mut Vec<PendingSkill>,
) -> Result<(), TomeError> {
    let agents = list_agent_files(plugin_dir);
    if agents.is_empty() {
        debug!(
            plugin = %id,
            "no agents directory or no agent files; enabling zero agent rows",
        );
        return Ok(());
    }

    for agent in agents {
        if was_cancelled() {
            return Err(TomeError::Interrupted);
        }
        let parsed: ParsedSkill =
            parse_skill_frontmatter(&agent.path).map_err(|err| match err {
                // Both delimiter and YAML-body failures are "malformed
                // recognised structure" for an agent — fail loudly with the
                // agent-specific exit 45 (entry-schema-p6.md, NFR-010).
                FrontmatterError::MissingDelimiters { .. }
                | FrontmatterError::InvalidYaml { .. } => TomeError::AgentTranslationFailed {
                    agent: agent.path.display().to_string(),
                },
                FrontmatterError::Io { source, .. } => TomeError::Io(source),
            })?;

        // `name` = frontmatter `name` else filename stem.
        let (name, name_fallback) = parsed.resolved_name(&agent.name);

        // S-1: the resolved `name` becomes the `<name>` half of the emitted
        // `<plugin>__<name>.<ext>` filename, which sync joins onto each
        // harness's agent dir. An attacker plugin shipping `name:
        // ../../../../tmp/evil` would escape that dir, so reject any `name`
        // that is not a single safe path segment BEFORE storing the row.
        // This is the index-time gate; `reconcile_agents` re-asserts a
        // defence-in-depth `target.parent()` check at write time.
        if !crate::harness::agents::is_safe_agent_name(&name) {
            return Err(TomeError::AgentTranslationFailed {
                agent: agent.path.display().to_string(),
            });
        }

        if name_fallback {
            warnings.push(format!(
                "name fallback applied for {}: using filename `{}`",
                agent.path.display(),
                name,
            ));
        }

        // `description` = frontmatter `description` else the first
        // non-empty body line (trimmed), per entry-schema-p6.md step 3.
        let description = resolve_agent_description(&parsed);

        let rel_path = agent
            .path
            .strip_prefix(plugin_dir)
            .unwrap_or(&agent.path)
            .to_string_lossy()
            .into_owned();

        pending.push(PendingSkill {
            catalog: id.catalog.clone(),
            plugin: id.plugin.clone(),
            name,
            kind: EntryKind::Agent,
            description,
            plugin_version: plugin_version.to_owned(),
            path: rel_path,
            // Agents do not contribute embedding text (when_to_use=NULL),
            // are never searchable, and never user-invocable
            // (entry-schema-p6.md). `resolved_user_invocable` already
            // hard-returns false for the Agent kind, but agents are not
            // embedded at all so these are pinned literals.
            when_to_use: None,
            searchable: false,
            user_invocable: false,
        });
    }

    Ok(())
}

/// Resolve an agent's `description`: trimmed frontmatter `description` if
/// non-empty, else the first non-empty line of the body, trimmed; else an
/// empty string (the `skills.description` column is NOT NULL, so agents
/// with neither a frontmatter description nor any body text store `""`).
fn resolve_agent_description(parsed: &ParsedSkill) -> String {
    if let Some(desc) = parsed
        .frontmatter
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return desc.to_owned();
    }
    parsed
        .body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// Parse a single entry file (skill or command) into a [`PendingSkill`].
/// Returns `Ok(None)` when the file's YAML body is invalid (FR-013c — the
/// warning has already been appended) so the caller skips the entry
/// without aborting the whole plugin.
fn parse_one_entry(
    id: &PluginId,
    plugin_dir: &Path,
    plugin_version: &str,
    file: &Path,
    fallback_name: &str,
    kind: EntryKind,
    warnings: &mut Vec<String>,
) -> Result<Option<PendingSkill>, TomeError> {
    let parsed: ParsedSkill = match parse_skill_frontmatter(file) {
        Ok(p) => p,
        Err(FrontmatterError::MissingDelimiters { file, message }) => {
            return Err(TomeError::SkillFrontmatterParseError { file, message });
        }
        Err(FrontmatterError::InvalidYaml { file: bad, message }) => {
            let warning = format!(
                "skipped {}: frontmatter YAML invalid: {}",
                bad.display(),
                message,
            );
            warn!(file = %bad.display(), reason = %message, "skipping entry: invalid YAML body");
            warnings.push(warning);
            return Ok(None);
        }
        Err(FrontmatterError::Io { file: _, source }) => return Err(TomeError::Io(source)),
    };

    // Argument-name validation (FR-013c sibling — Phase 5). Illegal
    // names are a parse-class failure with exit 29.
    if let Err(reason) = validate_argument_names(&parsed.frontmatter.arguments) {
        return Err(TomeError::InvalidArgumentFrontmatter {
            file: file.to_path_buf(),
            reason,
        });
    }

    let (name, name_fallback) = parsed.resolved_name(fallback_name);
    let (description, desc_fallback) = parsed.resolved_description();
    if name_fallback {
        warnings.push(format!(
            "name fallback applied for {}: using directory name `{}`",
            file.display(),
            name,
        ));
    }
    if desc_fallback {
        warnings.push(format!(
            "description fallback applied for {}: using leading body text",
            file.display(),
        ));
    }

    let rel_path = file
        .strip_prefix(plugin_dir)
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned();

    let when_to_use = parsed
        .frontmatter
        .when_to_use
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let searchable = parsed.frontmatter.resolved_searchable();
    let user_invocable = parsed.frontmatter.resolved_user_invocable(kind);

    Ok(Some(PendingSkill {
        catalog: id.catalog.clone(),
        plugin: id.plugin.clone(),
        name,
        kind,
        description,
        plugin_version: plugin_version.to_owned(),
        path: rel_path,
        when_to_use,
        searchable,
        user_invocable,
    }))
}

/// Step 4 — confirm the embedder and reranker entries in `MODEL_REGISTRY`
/// each have a readable `manifest.json` on disk. Missing models prompt a
/// download iff `allow_model_download` is set; otherwise we error with
/// `ModelMissing` (exit 30).
fn ensure_models_present(deps: &LifecycleDeps<'_>) -> Result<(), TomeError> {
    for entry in MODEL_REGISTRY {
        // Only enforce embedder and reranker. The summariser is downloaded
        // on the regen-summary path (US4) — gating enable on its presence
        // would force every workspace to pull ~400 MB before the first
        // skill is indexed.
        match entry.kind {
            ModelKind::Embedder | ModelKind::Reranker => {}
            ModelKind::Summariser => continue,
        }
        if model_manifest_ok(deps.paths, entry)? {
            continue;
        }
        if !deps.allow_model_download {
            return Err(TomeError::ModelMissing {
                model: entry.name.to_owned(),
            });
        }
        info!(model = entry.name, "downloading model artefact");
        download_model(entry, &deps.paths.models_dir, None)?;
    }
    Ok(())
}

/// Returns `Ok(true)` iff a parseable `manifest.json` for `entry` exists
/// under `paths.models_dir`. A read or parse failure is treated as "model
/// not installed" — the contract redirects the user to download.
fn model_manifest_ok(paths: &Paths, entry: &ModelEntry) -> Result<bool, TomeError> {
    let manifest_path = paths.model_manifest(entry.name)?;
    if !manifest_path.is_file() {
        return Ok(false);
    }
    let bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(_) => return Ok(false),
    };
    match serde_json::from_slice::<ModelManifest>(&bytes) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// -------------------------------------------------------------------------
// Unit tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CatalogEntry;
    use crate::embedding::stub::StubEmbedder;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    // ---- test scaffolding --------------------------------------------------

    /// Build a `Paths` rooted entirely under `root` so tests never touch
    /// `$HOME` or env vars (foundational retro: gotcha #5).
    fn test_paths(root: &Path) -> Paths {
        Paths::from_root(root.to_path_buf())
    }

    fn stub_seed() -> MetaSeed {
        MetaSeed {
            name: "stub-embedder".into(),
            version: "0".into(),
        }
    }

    fn stub_reranker_seed() -> MetaSeed {
        MetaSeed {
            name: "stub-reranker".into(),
            version: "0".into(),
        }
    }

    fn stub_summariser_seed() -> MetaSeed {
        MetaSeed {
            name: "stub-summariser".into(),
            version: "0".into(),
        }
    }

    /// Fabricate model dirs + manifest.json for every entry in
    /// `MODEL_REGISTRY`. We do NOT touch the network.
    fn fabricate_models(paths: &Paths) {
        for entry in MODEL_REGISTRY {
            let dir = paths.models_dir.join(entry.name);
            fs::create_dir_all(&dir).expect("create model dir");
            let manifest = ModelManifest {
                name: entry.name.to_owned(),
                version: entry.version.to_owned(),
                kind: entry.kind,
                source_url: entry.source_url.to_owned(),
                sha256: entry.sha256.to_owned(),
                size_bytes: entry.size_bytes,
                licence: entry.licence.to_owned(),
                files: entry.files.iter().map(|s| (*s).to_owned()).collect(),
                installed_at: OffsetDateTime::now_utc(),
            };
            let body = serde_json::to_vec_pretty(&manifest).expect("serialise manifest");
            fs::write(dir.join("manifest.json"), body).expect("write manifest");
        }
    }

    /// Build a minimal `Config` with one catalog whose cache lives at
    /// `catalog_root`.
    fn config_with_catalog(catalog_name: &str, catalog_root: &Path) -> Config {
        let mut catalogs = BTreeMap::new();
        catalogs.insert(
            catalog_name.to_owned(),
            CatalogEntry {
                name: catalog_name.to_owned(),
                url: "https://example.invalid/repo".into(),
                ref_: "main".into(),
                path: catalog_root.to_path_buf(),
                last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            },
        );
        Config { catalogs }
    }

    /// Lay out a plugin on disk: `<catalog>/<plugin>/.claude-plugin/plugin.json`
    /// + zero or more skills (each `(dir_name, skill_md_contents)`).
    fn write_plugin(
        catalog_root: &Path,
        plugin_name: &str,
        plugin_version: Option<&str>,
        skills: &[(&str, &str)],
    ) -> PathBuf {
        let plugin_dir = catalog_root.join(plugin_name);
        fs::create_dir_all(plugin_dir.join(".claude-plugin")).expect("plugin dir");
        let version_line = plugin_version
            .map(|v| format!(", \"version\": \"{v}\""))
            .unwrap_or_default();
        let manifest = format!(r#"{{"name": "{plugin_name}"{version_line}}}"#);
        fs::write(
            plugin_dir.join(".claude-plugin").join("plugin.json"),
            manifest,
        )
        .expect("write manifest");

        for (dir_name, contents) in skills {
            let skill_dir = plugin_dir.join("skills").join(dir_name);
            fs::create_dir_all(&skill_dir).expect("skill dir");
            fs::write(skill_dir.join("SKILL.md"), contents).expect("write SKILL.md");
        }

        plugin_dir
    }

    /// Construct a `LifecycleDeps` against the supplied stub. We have to
    /// thread it through carefully because `LifecycleDeps` borrows.
    ///
    /// `TEST_SCOPE` is lazily initialised via `OnceLock` because
    /// `WorkspaceName::global()` heap-allocates and so cannot be
    /// evaluated in `static` initialiser position. Tests stay
    /// scope-agnostic by using the privileged `global` workspace
    /// (matches the historical Phase 1/2 behaviour).
    fn test_scope() -> &'static crate::workspace::Scope {
        static TEST_SCOPE: std::sync::OnceLock<crate::workspace::Scope> =
            std::sync::OnceLock::new();
        TEST_SCOPE
            .get_or_init(|| crate::workspace::Scope(crate::workspace::WorkspaceName::global()))
    }

    fn make_deps<'a>(
        paths: &'a Paths,
        config: &'a Config,
        embedder: &'a StubEmbedder,
        allow_model_download: bool,
    ) -> LifecycleDeps<'a> {
        LifecycleDeps {
            paths,
            scope: test_scope(),
            config,
            embedder,
            embedder_seed: stub_seed(),
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download,
        }
    }

    fn count_rows(paths: &Paths, catalog: &str, plugin: &str) -> (i64, i64) {
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_seed(),
                reranker: stub_reranker_seed(),
                summariser: stub_summariser_seed(),
            },
        )
        .expect("open index");
        let workspace_name = test_scope().name().as_str();
        conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM skills AS s
             LEFT JOIN workspace_skills AS ws
                    ON ws.skill_id = s.id
                   AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?3)
             WHERE s.catalog = ?1 AND s.plugin = ?2",
            rusqlite::params![catalog, plugin, workspace_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("count")
    }

    fn good_skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\nbody text\n")
    }

    // ---- enable: happy path ------------------------------------------------

    #[test]
    fn enable_happy_path_inserts_skills() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first skill")),
                ("beta", &good_skill_md("beta", "second skill")),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable should succeed");
        assert_eq!(outcome.summary.total_skills, 2);
        assert_eq!(outcome.summary.newly_embedded, 2);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);

        let (total, enabled_sum) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 2);
        assert_eq!(enabled_sum, 2);
    }

    // ---- enable: idempotency rejected --------------------------------------

    #[test]
    fn enable_when_already_enabled_returns_error() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        enable(&id, &deps).expect("first enable");
        let err = enable(&id, &deps).expect_err("re-enable rejected");
        match err {
            TomeError::PluginAlreadyInState { state, .. } => {
                assert_eq!(state, PluginState::Enabled);
            }
            other => panic!("expected PluginAlreadyInState, got {other:?}"),
        }
    }

    // ---- enable: unknown catalog / plugin ----------------------------------

    #[test]
    fn enable_unknown_catalog() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let config = Config::default();
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "ghost/plug".parse().unwrap();
        let err = enable(&id, &deps).expect_err("unknown catalog");
        assert!(matches!(err, TomeError::CatalogNotFound(c) if c == "ghost"));
    }

    #[test]
    fn enable_unknown_plugin_directory() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let catalog_root = tmp.path().join("catalog");
        fs::create_dir_all(&catalog_root).unwrap();
        let config = config_with_catalog("acme", &catalog_root);
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/ghost".parse().unwrap();
        let err = enable(&id, &deps).expect_err("unknown plugin dir");
        assert!(matches!(err, TomeError::PluginNotFound(s) if s == "acme/ghost"));
    }

    // ---- enable: delimiter failure aborts the plugin -----------------------

    #[test]
    fn enable_aborts_on_missing_frontmatter_delimiters() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // A SKILL.md with no `---` at all — delimiter failure, the whole
        // enable aborts and nothing is inserted.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("broken", "no frontmatter here at all\n"),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let err = enable(&id, &deps).expect_err("delimiter failure aborts");
        assert!(matches!(err, TomeError::SkillFrontmatterParseError { .. }));

        // Transaction rolled back: zero rows for this plugin.
        let (total, _) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 0);
    }

    // ---- enable: YAML body failure skips one skill -------------------------

    #[test]
    fn enable_skips_skill_with_invalid_yaml_body_but_keeps_going() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // Bad skill: delimiters present, but the YAML body is `:` which is
        // syntactically invalid YAML. Good skill: well-formed.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("good", &good_skill_md("good", "a fine skill")),
                ("bad", "---\n: not valid yaml here\n---\nbody\n"),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable continues past bad skill");
        assert_eq!(outcome.summary.total_skills, 1);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.contains("frontmatter YAML invalid")),
            "expected skip warning, got {:?}",
            outcome.warnings,
        );

        // Only one row inserted; the bad skill's row is absent.
        let (total, _) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 1);
    }

    // ---- enable: fallback warnings -----------------------------------------

    #[test]
    fn enable_emits_fallback_warning_for_missing_name() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        // Empty name → directory-name fallback triggers a warning. The
        // description is present so only one fallback fires.
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[(
                "mydir",
                "---\nname: \"\"\ndescription: a description\n---\nbody\n",
            )],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();

        let outcome = enable(&id, &deps).expect("enable");
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.contains("name fallback applied") && w.contains("mydir")),
            "expected fallback warning, got {:?}",
            outcome.warnings,
        );
    }

    // ---- disable -----------------------------------------------------------

    #[test]
    fn disable_flips_all_rows_to_disabled() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();
        enable(&id, &deps).expect("enable");

        let outcome = disable(
            &id,
            &paths,
            test_scope(),
            &config,
            stub_seed(),
            stub_reranker_seed(),
            stub_summariser_seed(),
        )
        .expect("disable");
        assert_eq!(outcome.skills_retained, 2);

        let (total, enabled_sum) = count_rows(&paths, "acme", "plug");
        assert_eq!(total, 2);
        assert_eq!(enabled_sum, 0);
    }

    #[test]
    fn disable_when_already_disabled_returns_error() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/plug".parse().unwrap();
        enable(&id, &deps).expect("enable");
        disable(
            &id,
            &paths,
            test_scope(),
            &config,
            stub_seed(),
            stub_reranker_seed(),
            stub_summariser_seed(),
        )
        .expect("disable");

        let err = disable(
            &id,
            &paths,
            test_scope(),
            &config,
            stub_seed(),
            stub_reranker_seed(),
            stub_summariser_seed(),
        )
        .expect_err("second disable rejected");
        match err {
            TomeError::PluginAlreadyInState { state, .. } => {
                assert_eq!(state, PluginState::Disabled);
            }
            other => panic!("expected PluginAlreadyInState, got {other:?}"),
        }
    }

    // ---- model presence ----------------------------------------------------

    #[test]
    fn enable_returns_model_missing_when_models_absent_and_download_disallowed() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        // Note: we deliberately do NOT fabricate_models(&paths).

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );

        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false); // <-- false
        let id: PluginId = "acme/plug".parse().unwrap();

        let err = enable(&id, &deps).expect_err("model-missing");
        assert!(matches!(err, TomeError::ModelMissing { .. }));
    }

    // Cancellation: covered end-to-end by the slice-3 atomicity test (T084).
    // Unit-testing it here would require flipping `catalog::git::CANCELLED`
    // and remembering to flip it back across every other test, which is
    // racy under cargo's parallel runner. Skipped intentionally.

    // ---- reindex: helpers --------------------------------------------------

    /// Per-skill snapshot used by the reindex tests to assert on `name`,
    /// `content_hash`, `enabled`, and presence of an embedding row.
    fn snapshot_skills(paths: &Paths, catalog: &str, plugin: &str) -> Vec<(String, String, bool)> {
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_seed(),
                reranker: stub_reranker_seed(),
                summariser: stub_summariser_seed(),
            },
        )
        .expect("open index");
        let workspace_name = test_scope().name().as_str();
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.content_hash,
                        CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END AS enabled
                 FROM skills AS s
                 LEFT JOIN workspace_skills AS ws
                        ON ws.skill_id = s.id
                       AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?3)
                 WHERE s.catalog = ?1 AND s.plugin = ?2
                 ORDER BY s.name",
            )
            .expect("prepare snapshot");
        let rows = stmt
            .query_map(rusqlite::params![catalog, plugin, workspace_name], |row| {
                let name: String = row.get(0)?;
                let hash: String = row.get(1)?;
                let enabled: i64 = row.get(2)?;
                Ok((name, hash, enabled != 0))
            })
            .expect("query snapshot");
        rows.collect::<Result<Vec<_>, _>>().expect("collect")
    }

    /// Replace one SKILL.md on disk so a subsequent reindex sees a different
    /// description (and therefore a different content_hash).
    fn rewrite_skill(plugin_dir: &Path, skill_dir_name: &str, new_contents: &str) {
        let path = plugin_dir
            .join("skills")
            .join(skill_dir_name)
            .join("SKILL.md");
        fs::write(path, new_contents).expect("rewrite SKILL.md");
    }

    /// Delete one skill from disk so a subsequent reindex classifies it as
    /// Removed.
    fn remove_skill(plugin_dir: &Path, skill_dir_name: &str) {
        fs::remove_dir_all(plugin_dir.join("skills").join(skill_dir_name))
            .expect("remove skill dir");
    }

    /// Add one new skill on disk so a subsequent reindex classifies it as
    /// Added.
    fn add_skill(plugin_dir: &Path, skill_dir_name: &str, contents: &str) {
        let skill_dir = plugin_dir.join("skills").join(skill_dir_name);
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(skill_dir.join("SKILL.md"), contents).expect("write SKILL.md");
    }

    fn enable_and_count_embed_calls(
        id: &PluginId,
        paths: &Paths,
        config: &Config,
    ) -> (StubEmbedder, usize) {
        let embedder = StubEmbedder::new();
        let deps = make_deps(paths, config, &embedder, false);
        enable(id, &deps).expect("initial enable");
        let baseline = embedder.call_count();
        (embedder, baseline)
    }

    // ---- reindex: unchanged scope --------------------------------------------

    #[test]
    fn reindex_when_nothing_changed_skips_embedder() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let (embedder, baseline) = enable_and_count_embed_calls(&id, &paths, &config);
        assert_eq!(baseline, 2, "initial enable embeds both skills");

        let deps = make_deps(&paths, &config, &embedder, false);
        let outcome = reindex_plugin(&id, &deps, false).expect("reindex unchanged");
        assert_eq!(outcome.summary.added, 0);
        assert_eq!(outcome.summary.modified, 0);
        assert_eq!(outcome.summary.removed, 0);
        assert_eq!(outcome.summary.unchanged, 2);
        assert_eq!(
            embedder.call_count(),
            baseline,
            "no embed call should fire when nothing changed",
        );
    }

    // ---- reindex: modified ---------------------------------------------------

    #[test]
    fn reindex_re_embeds_only_the_modified_skill() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        let plugin_dir = write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let (embedder, baseline) = enable_and_count_embed_calls(&id, &paths, &config);

        // Mutate beta's description; alpha untouched.
        rewrite_skill(
            &plugin_dir,
            "beta",
            &good_skill_md("beta", "updated second"),
        );

        let deps = make_deps(&paths, &config, &embedder, false);
        let outcome = reindex_plugin(&id, &deps, false).expect("reindex modified");
        assert_eq!(outcome.summary.modified, 1);
        assert_eq!(outcome.summary.unchanged, 1);
        assert_eq!(outcome.summary.added, 0);
        assert_eq!(outcome.summary.removed, 0);
        assert_eq!(
            embedder.call_count() - baseline,
            1,
            "exactly one embed call for the modified skill",
        );

        let snap = snapshot_skills(&paths, "acme", "plug");
        let beta = snap.iter().find(|(n, _, _)| n == "beta").unwrap();
        assert_eq!(
            beta.1,
            crate::index::skills::content_hash("beta", "updated second", None)
        );
    }

    // ---- reindex: added ------------------------------------------------------

    #[test]
    fn reindex_inserts_new_skill_and_embeds_only_it() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        let plugin_dir = write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[("alpha", &good_skill_md("alpha", "first"))],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let (embedder, baseline) = enable_and_count_embed_calls(&id, &paths, &config);

        add_skill(&plugin_dir, "gamma", &good_skill_md("gamma", "third"));

        let deps = make_deps(&paths, &config, &embedder, false);
        let outcome = reindex_plugin(&id, &deps, false).expect("reindex added");
        assert_eq!(outcome.summary.added, 1);
        assert_eq!(outcome.summary.unchanged, 1);
        assert_eq!(embedder.call_count() - baseline, 1);

        let snap = snapshot_skills(&paths, "acme", "plug");
        assert!(snap.iter().any(|(n, _, _)| n == "gamma"));
    }

    // ---- reindex: removed ----------------------------------------------------

    #[test]
    fn reindex_deletes_row_for_removed_skill_no_embed_call() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        let plugin_dir = write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let (embedder, baseline) = enable_and_count_embed_calls(&id, &paths, &config);

        remove_skill(&plugin_dir, "beta");

        let deps = make_deps(&paths, &config, &embedder, false);
        let outcome = reindex_plugin(&id, &deps, false).expect("reindex removed");
        assert_eq!(outcome.summary.removed, 1);
        assert_eq!(outcome.summary.unchanged, 1);
        assert_eq!(outcome.summary.modified, 0);
        assert_eq!(outcome.summary.added, 0);
        assert_eq!(embedder.call_count(), baseline, "no embed on removal");

        let snap = snapshot_skills(&paths, "acme", "plug");
        assert_eq!(snap.len(), 1, "beta row dropped");
        assert!(snap.iter().all(|(n, _, _)| n != "beta"));
    }

    // ---- reindex: --force ----------------------------------------------------

    #[test]
    fn reindex_force_re_embeds_unchanged_skills_too() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let (embedder, baseline) = enable_and_count_embed_calls(&id, &paths, &config);

        let deps = make_deps(&paths, &config, &embedder, false);
        let outcome = reindex_plugin(&id, &deps, true).expect("force reindex");
        // Force classifies everything-with-existing-row as modified.
        assert_eq!(outcome.summary.modified, 2);
        assert_eq!(outcome.summary.unchanged, 0);
        assert_eq!(
            embedder.call_count() - baseline,
            2,
            "force re-embeds every existing skill",
        );
    }

    // ---- reindex: errors -----------------------------------------------------

    #[test]
    fn reindex_unknown_catalog_returns_catalog_not_found() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let config = Config::default();
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "ghost/plug".parse().unwrap();
        let err = reindex_plugin(&id, &deps, false).expect_err("unknown catalog");
        assert!(matches!(err, TomeError::CatalogNotFound(c) if c == "ghost"));
    }

    #[test]
    fn reindex_unknown_plugin_returns_plugin_not_found() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fabricate_models(&paths);
        let catalog_root = tmp.path().join("catalog");
        fs::create_dir_all(&catalog_root).unwrap();
        let config = config_with_catalog("acme", &catalog_root);
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        let id: PluginId = "acme/ghost".parse().unwrap();
        let err = reindex_plugin(&id, &deps, false).expect_err("unknown plugin");
        assert!(matches!(err, TomeError::PluginNotFound(s) if s == "acme/ghost"));
    }

    // ---- auto_disable_orphan -------------------------------------------------

    #[test]
    fn auto_disable_orphan_drops_all_rows_for_plugin() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        write_plugin(
            &catalog_root,
            "plug",
            Some("1.0.0"),
            &[
                ("alpha", &good_skill_md("alpha", "first")),
                ("beta", &good_skill_md("beta", "second")),
            ],
        );
        let id: PluginId = "acme/plug".parse().unwrap();
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);
        enable(&id, &deps).expect("initial enable");
        assert_eq!(count_rows(&paths, "acme", "plug"), (2, 2));

        let dropped = auto_disable_orphan(&id, &deps).expect("orphan disable");
        assert_eq!(dropped, 2);
        assert_eq!(count_rows(&paths, "acme", "plug"), (0, 0));
    }

    #[test]
    fn auto_disable_orphan_is_idempotent_when_no_rows() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(tmp.path());
        fs::create_dir_all(&paths.root).unwrap();
        fabricate_models(&paths);

        let catalog_root = tmp.path().join("catalog");
        let config = config_with_catalog("acme", &catalog_root);
        let id: PluginId = "acme/ghost".parse().unwrap();
        let embedder = StubEmbedder::new();
        let deps = make_deps(&paths, &config, &embedder, false);

        let dropped = auto_disable_orphan(&id, &deps).expect("orphan disable on empty");
        assert_eq!(dropped, 0);
    }
}
