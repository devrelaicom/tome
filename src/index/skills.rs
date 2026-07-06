//! CRUD over the `skills` table plus the atomic enable orchestrator.
//!
//! Per FR-004, enabling a plugin is one indivisible step: either every skill
//! is embedded and inserted, or nothing changes on disk. The
//! [`enable_plugin_atomic`] helper wraps embed-and-insert in a single
//! SQLite transaction so a SIGINT or embedder failure rolls back cleanly.
//!
//! Per FR-006 / FR-032, an enable / refresh of a skill whose
//! `(name, description)` text composition has not changed is a no-op embed:
//! we keep the existing vector and only ensure the matching
//! `workspace_skills` row exists. The diff is detected via [`content_hash`].
//!
//! Phase 4 / F9: the Phase 2/3 `skills.enabled` column is dropped.
//! Enablement is now expressed by a row in `workspace_skills` joining the
//! skill to a workspace. Phase 4 / F11a lifts the workspace identity to a
//! runtime parameter (`workspace_name: &str`) sourced from the resolved
//! scope — every SQL site below threads that value through the
//! `workspace_skills` join. The `skills` table itself is shared across
//! workspaces; only the junction is per-workspace (FR-380, FR-381,
//! FR-382, FR-383).
//!
//! Spec: data-model.md §5 (`SkillRecord`) and §9 (`ContentHash`),
//! research §R8 (embedding-text composition).

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::manifest::read_catalog_manifest;
use crate::error::TomeError;
use crate::index::workspace_catalogs;
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;

/// One row in the `skills` table after a successful read.
#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub id: i64,
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// Phase 5: kind discriminator (`skill` | `command`).
    pub kind: EntryKind,
    pub description: String,
    pub plugin_version: String,
    pub path: String,
    pub content_hash: String,
    /// Phase 5: indexed/embedded `when_to_use` guidance.
    pub when_to_use: Option<String>,
    /// Phase 5: resolved `searchable` flag (controls `search_skills`
    /// visibility).
    pub searchable: bool,
    /// Phase 5: resolved `user_invocable` flag (controls
    /// `prompts/list` visibility).
    pub user_invocable: bool,
    pub enabled: bool,
    pub indexed_at: String,
}

/// Inputs to [`enable_plugin_atomic`]. Phase 5 widens the original
/// `(name, description)` text composition with `when_to_use` and adds the
/// kind discriminator + resolved boolean flags. The caller supplies the
/// raw fields plus the on-disk path.
#[derive(Debug, Clone)]
pub struct PendingSkill {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// Phase 5: `skill` or `command`. The discriminator is directory-rooted
    /// (`<plugin>/skills/*` → Skill, `<plugin>/commands/*` → Command) and
    /// recorded in the `skills.kind` column.
    pub kind: EntryKind,
    pub description: String,
    pub plugin_version: String,
    pub path: String,
    /// Phase 5: `when_to_use` frontmatter — contributes to embedding text
    /// when present; `NULL` in DB when absent.
    pub when_to_use: Option<String>,
    /// Phase 5: resolved `searchable` flag (see
    /// `contracts/frontmatter-p5.md` § Resolved defaults).
    pub searchable: bool,
    /// Phase 5: resolved `user_invocable` flag.
    pub user_invocable: bool,
}

/// Outcome summary of an atomic enable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnableSummary {
    pub total_skills: u32,
    pub newly_embedded: u32,
}

/// Outcome summary of an atomic reindex. Mirrors the contract's
/// Added / Modified / Removed / Unchanged breakdown so the catalog-update
/// summary table and the `tome reindex` JSON record can both consume it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReindexSummary {
    /// On-disk now, not in the index before — embedded and inserted.
    pub added: u32,
    /// On-disk and in the index, content_hash changed (or `force = true`)
    /// — re-embedded and the row updated in place.
    pub modified: u32,
    /// In the index but no longer on-disk — row + embedding dropped.
    pub removed: u32,
    /// On-disk and in the index, content_hash unchanged, `force = false`
    /// — no embedder call.
    pub unchanged: u32,
}

/// The text composition Tome hashes and embeds.
///
/// Phase 5 (`contracts/entry-schema-p5.md` § Embedding text composition):
/// the composition widens to include `when_to_use` when present:
///
/// ```text
/// {name}
///
/// {description}
///
/// When to use: {when_to_use}
/// ```
///
/// The "When to use:" line + preceding blank line appear only when
/// `when_to_use` is non-empty. Two entries with the same composition
/// (name + description + when_to_use) produce the same hash, the
/// condition under which FR-006 / FR-032 perform a no-op refresh.
///
/// Pre-Phase-5 callers that omit `when_to_use` (pass `None`) produce the
/// historical `name + "\n\n" + description` shape — so existing rows
/// migrated forward via the v2→v3 schema migration keep their hashes
/// stable until they're reindexed with a frontmatter that now declares
/// `when_to_use`.
pub fn content_hash(name: &str, description: &str, when_to_use: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    let text = embedding_text(name, description, when_to_use);
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Embedding text per [`content_hash`]. Same composition function so the
/// embedder sees exactly the bytes whose digest we stored.
pub fn embedding_text(name: &str, description: &str, when_to_use: Option<&str>) -> String {
    match when_to_use {
        Some(wtu) if !wtu.is_empty() => {
            format!("{name}\n\n{description}\n\nWhen to use: {wtu}")
        }
        _ => format!("{name}\n\n{description}"),
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

/// SQL fragment computing the boolean `enabled` value as a LEFT JOIN to
/// `workspace_skills` keyed on the resolved workspace. Used in every
/// `SELECT` that previously read `skills.enabled` directly.
const ENABLED_EXPR: &str = "CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END";

/// SQL fragment joining `skills s` to `workspace_skills ws` against the
/// workspace whose name is bound at the supplied 1-based parameter index.
/// The LEFT JOIN means rows present in `skills` but not enabled in the
/// requested workspace still appear, with `ws.skill_id IS NULL` — same row
/// count as the Phase 2/3 `skills` projection.
fn workspace_join(workspace_param_index: usize) -> String {
    format!(
        "LEFT JOIN workspace_skills AS ws \
                ON ws.skill_id = s.id \
               AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?{workspace_param_index})"
    )
}

/// Standard `SELECT` projection used by both [`find`] and
/// [`list_for_plugin`]. Encodes the post-Phase-5 column shape including
/// the kind discriminator, `when_to_use`, and the resolved boolean
/// flags. The trailing column is always the LEFT-JOIN `enabled`
/// expression so `row.get(N)` indices match across callers.
const SELECT_COLS: &str = "s.id, s.catalog, s.plugin, s.name, s.kind, s.description, \
                           s.plugin_version, s.path, s.content_hash, s.when_to_use, \
                           s.searchable, s.user_invocable, s.indexed_at";

fn row_to_skill_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRecord> {
    let kind_text: String = row.get(4)?;
    let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(msg)),
        )
    })?;
    Ok(SkillRecord {
        id: row.get(0)?,
        catalog: row.get(1)?,
        plugin: row.get(2)?,
        name: row.get(3)?,
        kind,
        description: row.get(5)?,
        plugin_version: row.get(6)?,
        path: row.get(7)?,
        content_hash: row.get(8)?,
        when_to_use: row.get::<_, Option<String>>(9)?,
        searchable: row.get::<_, i64>(10)? != 0,
        user_invocable: row.get::<_, i64>(11)? != 0,
        indexed_at: row.get(12)?,
        // The trailing `enabled` column is appended by each caller's
        // SQL — see `find` / `list_for_plugin` below.
        enabled: row.get::<_, i64>(13)? != 0,
    })
}

/// Look up a single skill by identity, with enablement evaluated against
/// `workspace_name`. Returns `Ok(None)` when absent.
///
/// Phase 5: the identity tuple is `(catalog, plugin, kind, name)` —
/// callers must specify the `kind` they want. The legacy callers that
/// only know `(catalog, plugin, name)` semantically want skills (the
/// pre-Phase-5 default); they should pass `EntryKind::Skill`.
pub fn find(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    kind: EntryKind,
    name: &str,
) -> Result<Option<SkillRecord>, TomeError> {
    let join = workspace_join(5);
    let sql = format!(
        "SELECT {SELECT_COLS}, {ENABLED_EXPR}
         FROM skills AS s
         {join}
         WHERE s.catalog = ?1 AND s.plugin = ?2 AND s.kind = ?3 AND s.name = ?4"
    );
    conn.query_row(
        &sql,
        params![catalog, plugin, kind.as_str(), name, workspace_name],
        row_to_skill_record,
    )
    .optional()
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("find skill: {e}")))
}

/// List every entry row (both kinds) for one plugin, ordered by
/// `(kind, name)`. Enablement is evaluated against `workspace_name`.
///
/// Phase 5: both `skill` and `command` rows are returned. Callers that
/// only want skills filter on `record.kind == EntryKind::Skill`.
pub fn list_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<Vec<SkillRecord>, TomeError> {
    let join = workspace_join(3);
    let sql = format!(
        "SELECT {SELECT_COLS}, {ENABLED_EXPR}
         FROM skills AS s
         {join}
         WHERE s.catalog = ?1 AND s.plugin = ?2
         ORDER BY s.kind, s.name"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare list: {e}")))?;
    let rows = stmt
        .query_map(
            params![catalog, plugin, workspace_name],
            row_to_skill_record,
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query list: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect list: {e}")))
}

/// Distinct enabled plugin names for one catalog, sorted alphabetically.
/// Used by `tome catalog update` to drive the per-catalog reindex pass and
/// by `tome reindex <catalog>` to scope the explicit form. Phase 4 / F11a:
/// "enabled" means a `workspace_skills` row exists against the resolved
/// workspace `workspace_name`.
pub fn enabled_plugins_for_catalog(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
) -> Result<Vec<String>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.plugin
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.catalog = ?1 AND w.name = ?2
             ORDER BY s.plugin",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare enabled plugins: {e}"))
        })?;
    let rows = stmt
        .query_map(params![catalog, workspace_name], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query enabled plugins: {e}"))
        })?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect enabled plugins: {e}")))
}

/// The set of agent `<name>` values held by **≥ 2 distinct agent-kind rows
/// enabled in `workspace_name`** — the cross-plugin agent name-clash set
/// (FR-072). This is the single source of truth for agent name-collision
/// detection: later US1 work (native-translation displayed-name prefixing,
/// MCP-persona naming) consults it so a clashing agent name is disambiguated
/// identically everywhere. Computed **once per sync** (FR-072), not per
/// entry.
///
/// "≥ 2 rows" is keyed on the identity `(catalog, plugin)` pair behind each
/// agent row, so two plugins each shipping `agents/reviewer.md` clash, but a
/// single plugin's lone `reviewer` agent does not. Only rows enrolled in the
/// resolved workspace via `workspace_skills` count — a name held solely by
/// disabled agents is not a live clash.
///
/// Returns a `BTreeSet` so the caller gets deterministic ordering for
/// display / logging without a follow-up sort.
///
// Landed in the agent-indexing slice as the SSOT for clash detection; its
// first non-test consumer is the harness sync's native-translation
// reconciliation (`crate::harness::sync`), which computes the clash flag
// once per sync and threads it per agent into `translate_agent`.
pub(crate) fn agent_name_clash_set(
    conn: &Connection,
    workspace_name: &str,
) -> Result<std::collections::BTreeSet<String>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.name
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.kind = 'agent' AND w.name = ?1
             GROUP BY s.name
             HAVING COUNT(DISTINCT s.catalog || '/' || s.plugin) >= 2
             ORDER BY s.name",
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare clash set: {e}")))?;
    let rows = stmt
        .query_map(params![workspace_name], |row| row.get::<_, String>(0))
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query clash set: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect clash set: {e}")))
}

/// One enabled agent row, projected for the harness sync's native-agent
/// reconciliation (Phase 6 / US1). Carries the provenance identity plus
/// the catalog-relative source body path so the caller can resolve the
/// on-disk `.md` via [`resolve_entry_body_path`] and parse it into a
/// `CanonicalAgent`.
#[derive(Debug, Clone)]
pub struct EnabledAgent {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// The `skills.path` column — catalog-relative body path
    /// (`agents/<name>.md`).
    pub path: String,
    /// The `skills.plugin_version` column (`plugin.json` `version`).
    /// Phase 6 / US4 (C4-1): threaded onto the persona `PromptEntry` so
    /// `${TOME_PLUGIN_VERSION}` resolves in a persona body — the
    /// command/skill registry query already selects this, so the persona
    /// path mirrors it rather than substituting an empty string.
    pub plugin_version: String,
    /// The `skills.indexed_at` column (RFC 3339). Phase 6 / US4 (R4-1):
    /// threaded onto the persona `EntryIdentity` so a `<name>-persona`
    /// colliding with a command/skill tie-breaks by `indexed_at ASC` like
    /// every other entry (FR-062) instead of unconditionally winning the
    /// base name. Only the reserved `drop-persona` keeps an empty
    /// `indexed_at` (its documented first-sort reservation).
    pub indexed_at: String,
}

/// Every `agent`-kind row enabled in `workspace_name`, ordered by
/// `(catalog, plugin, name)` for deterministic emission (Phase 6 / US1).
///
/// "Enabled" means a `workspace_skills` row joins the agent to the
/// workspace — the same enrolment junction the clash-set query consults.
/// The harness sync uses this to enumerate which agents to translate and
/// emit; the ordering keeps the per-file `added`/`updated`/`removed`
/// outcome stable across runs.
pub fn enabled_agents_for_workspace(
    conn: &Connection,
    workspace_name: &str,
) -> Result<Vec<EnabledAgent>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.catalog, s.plugin, s.name, s.path, s.plugin_version, s.indexed_at
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.kind = 'agent' AND w.name = ?1
             ORDER BY s.catalog, s.plugin, s.name",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare enabled agents: {e}"))
        })?;
    let rows = stmt
        .query_map(params![workspace_name], |row| {
            Ok(EnabledAgent {
                catalog: row.get(0)?,
                plugin: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                plugin_version: row.get(4)?,
                indexed_at: row.get(5)?,
            })
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query enabled agents: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect enabled agents: {e}")))
}

/// DELETE every `workspace_skills` row for `(workspace_name, plugin)`. The
/// underlying `skills` rows + embeddings are retained so a subsequent
/// re-enable is cheap (FR-005, FR-006 + FR-383 retention rule) and so
/// other workspaces that still reference the same skill keep working.
/// Phase 4 / F11a redefines "disable" as removing the workspace
/// enrolment; shared skills outlive any single workspace.
pub fn mark_all_disabled_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<u32, TomeError> {
    let affected = conn
        .execute(
            "DELETE FROM workspace_skills
             WHERE workspace_id = (SELECT id FROM workspaces WHERE name = ?1)
               AND skill_id IN (
                  SELECT id FROM skills WHERE catalog = ?2 AND plugin = ?3
               )",
            params![workspace_name, catalog, plugin],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("mark_all_disabled_for_plugin: {e}"))
        })?;
    u32::try_from(affected).map_err(|_| {
        TomeError::IndexIntegrityCheckFailure(format!("affected rows ({affected}) overflows u32"))
    })
}

/// Drop every row for `(catalog, plugin)` from `skills` and the matching
/// virtual-table rows from `skill_embeddings`. Used by catalog removal and
/// upstream-deletion cascades (FR-035).
pub fn delete_by_plugin(conn: &Connection, catalog: &str, plugin: &str) -> Result<u32, TomeError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin delete tx: {e}")))?;

    tx.execute(
        "DELETE FROM skill_embeddings WHERE skill_id IN
            (SELECT id FROM skills WHERE catalog = ?1 AND plugin = ?2)",
        params![catalog, plugin],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete embeddings: {e}")))?;
    let removed = tx
        .execute(
            "DELETE FROM skills WHERE catalog = ?1 AND plugin = ?2",
            params![catalog, plugin],
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete skills: {e}")))?;

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit delete tx: {e}")))?;
    u32::try_from(removed).map_err(|_| {
        TomeError::IndexIntegrityCheckFailure(format!("removed rows ({removed}) overflows u32"))
    })
}

/// Resolve an entry row's `path` column to an absolute on-disk body path.
///
/// The `skills.path` column stores the entry body path **relative** to
/// the plugin's catalog-side root directory (e.g. `skills/foo/SKILL.md`
/// or `commands/bar.md`). The absolute path is recovered by:
///
/// 1. Looking up the catalog's on-disk cache directory via
///    [`workspace_catalogs::resolve_catalog_path`] (returns the URL-hashed
///    cache dir under `<root>/catalogs/`).
/// 2. Reading `<cache_dir>/tome-catalog.toml` to find the plugin's
///    `source` declaration; falling back to `<cache_dir>/<plugin>` for
///    manifest-less catalogs. Mirrors
///    [`crate::plugin::lifecycle::resolve_plugin_dir`].
/// 3. Joining the plugin dir with the stored relative path.
///
/// US1.d reviewer pass (BLOCKER S-H1): the stored path MUST be a
/// normalised relative path. Absolute paths and `..` components are
/// rejected at the boundary — a forged or upgrade-corrupted DB row
/// otherwise lets the resolved absolute path escape the plugin
/// directory. The legacy "absolute paths short-circuit the manifest
/// walk" pre-US1.b carve-out is gone; Phase 5 writers always store
/// catalog-relative paths and an absolute string in the column now
/// surfaces as `TomeError::Io(InvalidInput)` (exit 7).
///
/// US1.b initially shipped this resolver inline in `mcp::prompts`; US1.c
/// promotes it because the same resolution now serves two callers:
/// `prompts/get` and the previously-broken `get_skill` MCP tool, which
/// was calling `PathBuf::from(&row.path)` against a relative path string
/// (latent bug surfaced during US1.b implementation review).
///
/// # Errors
///
/// - [`TomeError::CatalogNotFound`] when the workspace has no enrolment
///   for the row's catalog (e.g. the catalog was disabled mid-session).
/// - [`TomeError::EntryNotFound`] when the catalog enrolment exists but
///   the on-disk plugin directory has gone missing (catalog cache
///   evicted; manifest references a plugin that no longer exists on
///   disk). The mismatch is preserved so callers can map to a per-
///   surface error envelope (e.g. `prompt_not_found` for the MCP prompts
///   surface, `unknown_plugin` for `get_skill`).
pub fn resolve_entry_body_path(
    conn: &Connection,
    paths: &Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    stored_path: &str,
) -> Result<PathBuf, TomeError> {
    let stored = PathBuf::from(stored_path);
    validate_db_stored_path(&stored)?;

    let plugin_dir = plugin_root_dir(conn, paths, workspace_name, catalog, plugin)?;
    if !plugin_dir.is_dir() {
        return Err(TomeError::EntryNotFound {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
            name: stored_path.to_owned(),
            kind: "entry".to_owned(),
        });
    }
    Ok(plugin_dir.join(&stored))
}

/// Resolve the absolute on-disk root of an installed plugin —
/// `<catalog-cache>/<source>` (manifest declaration) or
/// `<catalog-cache>/<plugin>` (manifest-less / flat fallback).
///
/// This is the `${CLAUDE_PLUGIN_ROOT}` target value the Phase 6 hooks
/// rewrite resolves against (Phase 5's `${TOME_PLUGIN_DIR}` value). It is
/// the shared prefix [`resolve_entry_body_path`] joins the catalog-relative
/// body path onto; promoted to its own helper at the second consumer
/// (single-source-of-truth promotion) so the hooks writer and the entry-body
/// resolver agree on what "plugin root" means.
///
/// Unlike [`resolve_entry_body_path`] this does NOT assert the directory
/// exists — the caller decides whether an absent plugin root is an error
/// (the hooks pass treats a plugin with no `hooks/hooks.json` as a benign
/// no-op).
pub fn plugin_root_dir(
    conn: &Connection,
    paths: &Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<PathBuf, TomeError> {
    let catalog_path =
        workspace_catalogs::resolve_catalog_path(conn, paths, workspace_name, catalog)?;
    let plugin_dir = match read_catalog_manifest(&catalog_path) {
        Some(manifest) => manifest
            .plugins
            .iter()
            .find(|p| p.name == plugin)
            .map(|decl| catalog_path.join(&decl.source))
            .unwrap_or_else(|| catalog_path.join(plugin)),
        None => catalog_path.join(plugin),
    };
    Ok(plugin_dir)
}

/// Every `(catalog, plugin)` pair with at least one entry enabled in
/// `workspace_name`, ordered by `(catalog, plugin)` for deterministic
/// reconciliation (Phase 6 / US2).
///
/// "Enabled" means a `workspace_skills` row joins one of the plugin's
/// entries to the workspace — the same enrolment junction the agent and
/// clash-set queries consult. The hooks sync uses this to enumerate which
/// plugins' `hooks/hooks.json` to read; the ordering keeps the per-event
/// merge outcome stable across runs.
pub fn enabled_plugins_for_workspace(
    conn: &Connection,
    workspace_name: &str,
) -> Result<Vec<(String, String)>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.catalog, s.plugin
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE w.name = ?1
             ORDER BY s.catalog, s.plugin",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare enabled plugins ws: {e}"))
        })?;
    let rows = stmt
        .query_map(params![workspace_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query enabled plugins ws: {e}"))
        })?;
    rows.collect::<Result<_, _>>().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("collect enabled plugins ws: {e}"))
    })
}

/// Refuse absolute paths and `..` components in DB-stored relative
/// paths. The shared S-H1 (US1.d BLOCKER) boundary check; consumed by
/// [`resolve_entry_body_path`] and by
/// `commands/plugin/show.rs::list_entries`'s frontmatter-resolution
/// path. Phase 5 Polish M-4 promoted this from two inline copies to a
/// single SSOT so future safety additions (NUL refusal, UTF-8 component
/// validation, …) land in one place rather than silently lagging at the
/// second site.
pub(crate) fn validate_db_stored_path(stored: &std::path::Path) -> Result<(), TomeError> {
    if stored.is_absolute() {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("stored entry path is absolute: {}", stored.display()),
        )));
    }
    if stored
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "stored path contains parent-directory traversal: {}",
                stored.display(),
            ),
        )));
    }
    Ok(())
}

/// Insert a new entry row + matching embedding, or update an existing row
/// in place. Run inside an already-open transaction by the caller.
///
/// Phase 5: writes the kind discriminator and the new
/// `searchable`/`user_invocable`/`when_to_use` columns. The conflict
/// target is the widened identity tuple `(catalog, plugin, kind, name)`,
/// matching the post-v3 unique index `skills_unique`.
///
/// Phase 6 / US1: `embedding` is `None` for agent rows — agents are never
/// embedded (`entry-schema-p6.md` § "Indexing pipeline" step 6). The prior
/// `skill_embeddings` row (if any) is still deleted so a kind that *was*
/// embeddable and is now an agent — or a re-index that drops embedding —
/// leaves no orphan vector, but no new vector row is written when `None`.
fn upsert_skill(
    tx: &rusqlite::Transaction<'_>,
    pending: &PendingSkill,
    hash: &str,
    embedding: Option<&[f32]>,
    now: &str,
) -> Result<i64, TomeError> {
    tx.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version, path,
             content_hash, when_to_use, searchable, user_invocable, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(catalog, plugin, kind, name) DO UPDATE SET
            description    = excluded.description,
            plugin_version = excluded.plugin_version,
            path           = excluded.path,
            content_hash   = excluded.content_hash,
            when_to_use    = excluded.when_to_use,
            searchable     = excluded.searchable,
            user_invocable = excluded.user_invocable,
            indexed_at     = excluded.indexed_at",
        params![
            pending.catalog,
            pending.plugin,
            pending.name,
            pending.kind.as_str(),
            pending.description,
            pending.plugin_version,
            pending.path,
            hash,
            pending.when_to_use,
            i64::from(pending.searchable),
            i64::from(pending.user_invocable),
            now,
        ],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("upsert skill row: {e}")))?;

    let id: i64 = tx
        .query_row(
            "SELECT id FROM skills
             WHERE catalog = ?1 AND plugin = ?2 AND kind = ?3 AND name = ?4",
            params![
                pending.catalog,
                pending.plugin,
                pending.kind.as_str(),
                pending.name,
            ],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("look up skill id: {e}")))?;

    // The plain `skill_embeddings` BLOB table (schema v6) does not support
    // `INSERT OR REPLACE` without a PRIMARY KEY conflict path, so we
    // DELETE-then-INSERT. The DELETE is a no-op when there's no prior row,
    // so this is correct for both first-time inserts and re-embeds.
    tx.execute(
        "DELETE FROM skill_embeddings WHERE skill_id = ?1",
        params![id],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("drop prior embedding: {e}")))?;
    // Phase 6 / US1: agent rows pass `None` and get no `skill_embeddings`
    // row at all. Searchable kinds (skill/command) always carry a vector.
    if let Some(embedding) = embedding {
        let bytes = embedding_to_bytes(embedding);
        tx.execute(
            "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
            params![id, bytes],
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("insert embedding: {e}")))?;
    }

    Ok(id)
}

/// INSERT-or-UPSERT a `workspace_skills` row joining `skill_id` to the
/// workspace named `workspace_name`. Idempotent: re-running for an
/// already-enrolled skill is a no-op apart from bumping `enabled_at`.
/// The PK `(workspace_id, skill_id)` enforces uniqueness. Phase 4 / F11a:
/// the privileged-`global`-only variant from F9 is gone; this is the
/// general form.
fn upsert_workspace_skill(
    tx: &rusqlite::Transaction<'_>,
    workspace_name: &str,
    skill_id: i64,
    enabled_at_unix: i64,
) -> Result<(), TomeError> {
    tx.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
         VALUES ((SELECT id FROM workspaces WHERE name = ?1), ?2, ?3)
         ON CONFLICT(workspace_id, skill_id) DO UPDATE SET enabled_at = excluded.enabled_at",
        params![workspace_name, skill_id, enabled_at_unix],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "upsert workspace_skills ({workspace_name}, skill_id={skill_id}): {e}"
        ))
    })?;
    Ok(())
}

/// Embed an entry's text unless it is an agent. Phase 6 / US1: agent rows
/// are never embedded (`entry-schema-p6.md` § "Indexing pipeline" step 6),
/// so this returns `Ok(None)` for `EntryKind::Agent` without invoking the
/// embedder; every other kind embeds the standard composition. Shared by
/// the Added and Modified branches of [`reindex_plugin_atomic`].
fn embed_unless_agent<F>(
    pending: &PendingSkill,
    embed: &mut F,
) -> Result<Option<Vec<f32>>, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
{
    if pending.kind == EntryKind::Agent {
        return Ok(None);
    }
    let vector = embed(&embedding_text(
        &pending.name,
        &pending.description,
        pending.when_to_use.as_deref(),
    ))?;
    Ok(Some(vector))
}

fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(v));
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Enable every skill in `pending` under one SQLite transaction (FR-004).
///
/// For each skill:
/// * Compute [`content_hash`] over `(name, description)`.
/// * If a row already exists with the same hash, simply flip `enabled = 1`
///   — no embedder call (FR-006).
/// * Otherwise, invoke `embed` on [`embedding_text`] and upsert both the
///   `skills` row and the `skill_embeddings` row.
///
/// `embed` is a closure rather than a trait so this slice can land without
/// pulling in the embedding crate that arrives in slice 5. A trait wrapper
/// is a thin adapter over this signature.
///
/// `on_entry` fires once per `pending` entry, AFTER it is fully processed
/// (embedded-or-refreshed + enrolled). It exists to drive a determinate
/// progress bar from the caller (#421) without this module knowing about
/// presentation; keep it side-effect-light — it runs inside the enable
/// transaction.
pub fn enable_plugin_atomic<F, P>(
    conn: &mut Connection,
    workspace_name: &str,
    pending: &[PendingSkill],
    mut embed: F,
    mut on_entry: P,
) -> Result<EnableSummary, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
    P: FnMut(),
{
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin enable tx: {e}")))?;

    let now = now_rfc3339();
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let mut newly_embedded: u32 = 0;

    for skill in pending {
        let hash = content_hash(
            &skill.name,
            &skill.description,
            skill.when_to_use.as_deref(),
        );

        // Phase 5: identity includes `kind` — same-name entries across
        // kinds resolve to two distinct rows.
        let existing: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, content_hash FROM skills
                 WHERE catalog = ?1 AND plugin = ?2 AND kind = ?3 AND name = ?4",
                params![skill.catalog, skill.plugin, skill.kind.as_str(), skill.name,],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "lookup existing skill {}/{}: {e}",
                    skill.plugin, skill.name
                ))
            })?;

        let skill_id = match existing {
            Some((id, stored_hash)) if stored_hash == hash => {
                // Cheap re-enable (FR-006): metadata refresh only, no
                // embedder invocation, no embedding rewrite. Phase 5
                // also refreshes the resolved boolean flags +
                // `when_to_use` so frontmatter changes that don't
                // touch the embedding-text composition still propagate.
                tx.execute(
                    "UPDATE skills
                     SET plugin_version = ?2,
                         path = ?3,
                         when_to_use = ?4,
                         searchable = ?5,
                         user_invocable = ?6,
                         indexed_at = ?7
                     WHERE id = ?1",
                    params![
                        id,
                        skill.plugin_version,
                        skill.path,
                        skill.when_to_use,
                        i64::from(skill.searchable),
                        i64::from(skill.user_invocable),
                        now,
                    ],
                )
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!(
                        "refresh metadata for {}/{}: {e}",
                        skill.plugin, skill.name
                    ))
                })?;
                id
            }
            _ => {
                // Phase 6 / US1: agents are never embedded — route through
                // `embed_unless_agent` so the "never embed agents" predicate
                // is single-sourced with the reindex path. `newly_embedded`
                // counts only kinds that actually produced a vector (skills +
                // commands).
                let embedding = embed_unless_agent(skill, &mut embed)?;
                let id = upsert_skill(&tx, skill, &hash, embedding.as_deref(), &now)?;
                if embedding.is_some() {
                    newly_embedded = newly_embedded.saturating_add(1);
                }
                id
            }
        };

        // Enrol the entry in the resolved workspace (Phase 4 / F11a
        // replacement for F9's privileged-`global`-only write). Agents are
        // enrolled too — the junction is plugin-grained and kind-agnostic
        // (entry-schema-p6.md), so enabling enrols skills+commands+agents
        // and disabling removes them all.
        upsert_workspace_skill(&tx, workspace_name, skill_id, now_unix)?;
        // Per-entry progress tick (#421): fires for cheap re-enables too, so
        // a bar sized to `pending.len()` always completes.
        on_entry();
    }

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit enable tx: {e}")))?;

    let total_skills = u32::try_from(pending.len()).unwrap_or(u32::MAX);
    Ok(EnableSummary {
        total_skills,
        newly_embedded,
    })
}

/// Atomically reconcile the index for one plugin against an on-disk snapshot.
///
/// `pending` is the snapshot of skills currently visible under
/// `<plugin_dir>/skills/*/SKILL.md`. Existing rows for `(catalog, plugin)` are
/// classified against this snapshot:
///
/// * **Added** — in `pending` but no row for `(catalog, plugin, name)` exists.
///   Embed and INSERT.
/// * **Modified** — row exists, `content_hash` differs (or `force = true`).
///   Re-embed and UPDATE in place. `enabled` is forced back to 1 — reindexing
///   a disabled-but-stored row brings it back into the active set.
/// * **Unchanged** — row exists, `content_hash` matches, `force = false`.
///   No embedder call; `plugin_version`, `path`, and `indexed_at` are still
///   refreshed so observers see that the reindex visited the row.
/// * **Removed** — row exists for some `name` not in `pending`. DELETE the
///   row and its embedding.
///
/// All four classes commit inside one SQLite transaction so a SIGINT or
/// embedder failure leaves the index unchanged.
///
/// Phase 4 / F11a: the `enabled` bit is no longer carried on the
/// `skills` row. Reindex re-asserts the matching
/// `workspace_skills(workspace_name, id)` row for every
/// Added/Modified/Unchanged skill (a no-op when the enrolment already
/// exists; the PK keeps idempotency).
/// `on_entry` fires once per `pending` entry after it is fully processed —
/// the reindex analogue of `enable_plugin_atomic`'s progress tick (#421).
/// The Removed pass does not tick: a bar sized to `pending.len()` covers
/// exactly the visited-on-disk set, and row deletion is microseconds.
#[allow(clippy::too_many_arguments)] // the #421 progress tick pushed this to 8; the sole caller is lifecycle.rs
pub fn reindex_plugin_atomic<F, P>(
    conn: &mut Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    pending: &[PendingSkill],
    force: bool,
    mut embed: F,
    mut on_entry: P,
) -> Result<ReindexSummary, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
    P: FnMut(),
{
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin reindex tx: {e}")))?;

    let now = now_rfc3339();
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let mut summary = ReindexSummary::default();

    // Snapshot existing rows once per call. Keyed by `(kind, name)` so
    // Phase 5's same-name-different-kind entries don't collide. We'll
    // diff against `pending` below and use the leftover set for the
    // Removed branch.
    let mut existing: std::collections::HashMap<(EntryKind, String), (i64, String)> =
        std::collections::HashMap::new();
    {
        let mut stmt = tx
            .prepare(
                "SELECT id, kind, name, content_hash FROM skills
                 WHERE catalog = ?1 AND plugin = ?2",
            )
            .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare existing: {e}")))?;
        let rows = stmt
            .query_map(params![catalog, plugin], |row| {
                let id: i64 = row.get(0)?;
                let kind_text: String = row.get(1)?;
                let name: String = row.get(2)?;
                let hash: String = row.get(3)?;
                let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(msg)),
                    )
                })?;
                Ok(((kind, name), (id, hash)))
            })
            .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query existing: {e}")))?;
        for row in rows {
            let (key, value) = row.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("collect existing: {e}"))
            })?;
            existing.insert(key, value);
        }
    }

    // Pass 1 — Added / Modified / Unchanged.
    for skill in pending {
        let hash = content_hash(
            &skill.name,
            &skill.description,
            skill.when_to_use.as_deref(),
        );

        let skill_id = match existing.remove(&(skill.kind, skill.name.clone())) {
            Some((id, stored_hash)) if stored_hash == hash && !force => {
                // Unchanged: touch metadata only. Phase 5 refreshes
                // `when_to_use` + resolved flags so frontmatter changes
                // outside the embedding-text composition still propagate.
                tx.execute(
                    "UPDATE skills
                     SET plugin_version = ?2,
                         path = ?3,
                         when_to_use = ?4,
                         searchable = ?5,
                         user_invocable = ?6,
                         indexed_at = ?7
                     WHERE id = ?1",
                    params![
                        id,
                        skill.plugin_version,
                        skill.path,
                        skill.when_to_use,
                        i64::from(skill.searchable),
                        i64::from(skill.user_invocable),
                        now,
                    ],
                )
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!(
                        "touch unchanged skill {}/{}: {e}",
                        skill.plugin, skill.name
                    ))
                })?;
                summary.unchanged = summary.unchanged.saturating_add(1);
                id
            }
            Some(_) => {
                // Modified (or force=true rewriting an unchanged row).
                // Phase 6 / US1: agents skip the embedder (no vector row).
                let embedding = embed_unless_agent(skill, &mut embed)?;
                let id = upsert_skill(&tx, skill, &hash, embedding.as_deref(), &now)?;
                summary.modified = summary.modified.saturating_add(1);
                id
            }
            None => {
                // Added. Phase 6 / US1: agents skip the embedder.
                let embedding = embed_unless_agent(skill, &mut embed)?;
                let id = upsert_skill(&tx, skill, &hash, embedding.as_deref(), &now)?;
                summary.added = summary.added.saturating_add(1);
                id
            }
        };

        // Re-assert the resolved-workspace enrolment for every visited
        // skill (idempotent — PK keyed on (workspace_id, skill_id)).
        upsert_workspace_skill(&tx, workspace_name, skill_id, now_unix)?;
        // Per-entry progress tick (#421): fires for Unchanged rows too, so
        // a bar sized to `pending.len()` always completes.
        on_entry();
    }

    // Pass 2 — Removed: anything still left in `existing` is on-index but
    // not on-disk. Drop the row + its embedding.
    for (_key, (id, _hash)) in existing {
        tx.execute(
            "DELETE FROM skill_embeddings WHERE skill_id = ?1",
            params![id],
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete embedding: {e}")))?;
        tx.execute("DELETE FROM skills WHERE id = ?1", params![id])
            .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete skill: {e}")))?;
        summary.removed = summary.removed.saturating_add(1);
    }

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit reindex tx: {e}")))?;

    Ok(summary)
}

/// One tierable entry (skill or command) enabled in a workspace, projected for
/// the routing-directive builder and `tome tier list`. Agents are excluded —
/// they are delivered as native translated files, not via the MCP retrieval
/// tools, so they carry no tier.
#[derive(Debug, Clone)]
pub struct TieredEntry {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    pub description: String,
    pub when_to_use: Option<String>,
    /// The `workspace_skills.tier` column (1 | 2 | 3).
    pub tier: u8,
}

/// Every `skill`/`command` row enabled in `workspace_name`, with its routing
/// tier, ordered by `(tier, catalog, plugin, name)` so the generated directive
/// is byte-stable across runs. Agents (`kind = 'agent'`) are excluded.
pub fn tiered_entries_for_workspace(
    conn: &Connection,
    workspace_name: &str,
) -> Result<Vec<TieredEntry>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.catalog, s.plugin, s.name, s.kind, s.description, s.when_to_use, ws.tier
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE w.name = ?1 AND s.kind IN ('skill', 'command')
             ORDER BY ws.tier, s.catalog, s.plugin, s.name",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare tiered entries: {e}"))
        })?;
    let rows = stmt
        .query_map(params![workspace_name], |row| {
            let kind_text: String = row.get(3)?;
            let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(msg)),
                )
            })?;
            Ok(TieredEntry {
                catalog: row.get(0)?,
                plugin: row.get(1)?,
                name: row.get(2)?,
                kind,
                description: row.get(4)?,
                when_to_use: row.get::<_, Option<String>>(5)?,
                tier: row.get::<_, i64>(6)? as u8,
            })
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query tiered entries: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect tiered entries: {e}")))
}

/// Set the routing `tier` for one enabled `(catalog, plugin, kind, name)` entry
/// in `workspace_name`. Returns `EntryNotFound` (exit 27) when no enrolled row
/// matches — addressing an entry that is not enabled in the workspace, or does
/// not exist, is the same user-facing error.
pub fn set_tier_for_entry(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    kind: &EntryKind,
    name: &str,
    tier: u8,
) -> Result<(), TomeError> {
    let affected = conn
        .execute(
            "UPDATE workspace_skills
             SET tier = ?1
             WHERE workspace_id = (SELECT id FROM workspaces WHERE name = ?2)
               AND skill_id = (SELECT s.id FROM skills s
                               WHERE s.catalog = ?3 AND s.plugin = ?4
                                 AND s.kind = ?5 AND s.name = ?6)",
            params![
                tier as i64,
                workspace_name,
                catalog,
                plugin,
                kind.as_str(),
                name
            ],
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("set tier: {e}")))?;
    if affected == 0 {
        return Err(TomeError::EntryNotFound {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
            name: name.to_owned(),
            kind: kind.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Bulk-set the tier for every enabled `skill`/`command` entry of one plugin in
/// `workspace_name` (the `tome plugin enable --tier` path). Agents are left
/// untouched. Returns the number of rows updated (0 when the plugin has no
/// enrolled tierable entries — a benign no-op the caller may ignore).
pub fn set_tier_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    tier: u8,
) -> Result<u32, TomeError> {
    let affected = conn
        .execute(
            "UPDATE workspace_skills
             SET tier = ?1
             WHERE workspace_id = (SELECT id FROM workspaces WHERE name = ?2)
               AND skill_id IN (SELECT s.id FROM skills s
                                WHERE s.catalog = ?3 AND s.plugin = ?4
                                  AND s.kind IN ('skill', 'command'))",
            params![tier as i64, workspace_name, catalog, plugin],
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("set tier for plugin: {e}")))?;
    u32::try_from(affected).map_err(|_| {
        TomeError::IndexIntegrityCheckFailure(format!("affected rows ({affected}) overflows u32"))
    })
}

/// Reset the routing tier of EVERY enabled `skill`/`command` entry in
/// `workspace_name` to the default (3) — the `tome tier clear --all` path.
/// Agents carry no tier and are untouched by the `kind IN ('skill','command')`
/// scope (mirroring [`tiered_entries_for_workspace`]).
///
/// Returns the affected entries (post-reset, so their `tier` is already the
/// default) in the byte-stable `(tier, catalog, plugin, name)` order the
/// emitter iterates — the caller emits one record per row. A single UPDATE keeps
/// the whole reset atomic under the advisory write lock; the follow-up SELECT
/// re-reads the same enrolled set so the emitted rows exactly reflect what was
/// changed. An empty workspace (no enabled tierable entries) yields an empty
/// vec; the caller emits nothing and exits 0 (a benign idempotent no-op — there
/// is no "nothing to reset" message).
pub fn reset_all_tiers_for_workspace(
    conn: &Connection,
    workspace_name: &str,
) -> Result<Vec<TieredEntry>, TomeError> {
    conn.execute(
        "UPDATE workspace_skills
         SET tier = 3
         WHERE workspace_id = (SELECT id FROM workspaces WHERE name = ?1)
           AND skill_id IN (SELECT s.id FROM skills s
                            WHERE s.kind IN ('skill', 'command'))",
        params![workspace_name],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("reset all tiers: {e}")))?;
    // Return the reset set for emit; the shared projection keeps the ordering
    // byte-stable across `set`/`clear`/`list`.
    tiered_entries_for_workspace(conn, workspace_name)
}

/// The distinct routing tiers held by entries of one `(catalog, plugin)`
/// enabled in `workspace_name`. "Enabled" means a `workspace_skills` row joins
/// the entry to the workspace — the same enrolment junction the tier and
/// aggregate queries consult. Every enabled entry carries a `workspace_skills.tier`
/// (default 3), so an enabled skill / command / agent contributes its tier here.
///
/// Used by `tome plugin list --tier <n>` to keep only plugins that have at
/// least one enabled entry at the requested tier (membership test on the
/// returned set). Returns an empty set when the plugin has no enabled entries.
pub fn enabled_tiers_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<std::collections::BTreeSet<u8>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT ws.tier
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.catalog = ?1 AND s.plugin = ?2 AND w.name = ?3",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare enabled tiers: {e}"))
        })?;
    let rows = stmt
        .query_map(params![catalog, plugin, workspace_name], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query enabled tiers: {e}")))?;
    let mut out = std::collections::BTreeSet::new();
    for r in rows {
        let tier = r.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("collect enabled tiers: {e}"))
        })?;
        // Tiers are constrained to 1..=3 at the write boundary
        // (`--tier`/`tier set` validation); clamp defensively so a corrupt
        // DB value can't panic the read-only list path.
        out.insert(u8::try_from(tier).unwrap_or(0));
    }
    Ok(out)
}

/// The routing `tier` of each entry of one `(catalog, plugin)` enabled in
/// `workspace_name`, keyed by `(kind, name)`. Only entries enrolled via
/// `workspace_skills` appear — a stored-but-disabled entry has no tier and is
/// absent. Used by `tome plugin show --details` to annotate each per-entry line
/// with its tier; the `(kind, name)` key matches how `plugin show` splits the
/// `SkillRecord` list into skills / commands / agents.
pub fn entry_tiers_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<std::collections::HashMap<(EntryKind, String), u8>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.kind, s.name, ws.tier
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE s.catalog = ?1 AND s.plugin = ?2 AND w.name = ?3",
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare entry tiers: {e}")))?;
    let rows = stmt
        .query_map(params![catalog, plugin, workspace_name], |row| {
            let kind_text: String = row.get(0)?;
            let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(msg)),
                )
            })?;
            let name: String = row.get(1)?;
            let tier: i64 = row.get(2)?;
            Ok(((kind, name), u8::try_from(tier).unwrap_or(0)))
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query entry tiers: {e}")))?;
    let mut out = std::collections::HashMap::new();
    for r in rows {
        let (key, tier) = r.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("collect entry tiers: {e}"))
        })?;
        out.insert(key, tier);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{MetaSeed, OpenOptions, open};
    use tempfile::TempDir;

    fn seed() -> MetaSeed {
        MetaSeed {
            name: "stub".into(),
            version: "0".into(),
        }
    }

    /// Open a bootstrapped on-disk DB (the vec0 extension is registered by
    /// `index::open`, which an in-memory raw `Connection` would lack).
    fn open_db(dir: &TempDir) -> Connection {
        open(
            &dir.path().join("index.db"),
            &OpenOptions {
                embedder: seed(),
                reranker: seed(),
                summariser: seed(),
                profile: None,
            },
        )
        .expect("open index")
    }

    /// Insert an agent row for `(catalog, plugin, name)` and enrol it in the
    /// `global` workspace. No embedding row — mirrors the indexing-pipeline
    /// invariant.
    fn insert_enabled_agent(conn: &Connection, catalog: &str, plugin: &str, name: &str) {
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES (?1, ?2, ?3, 'agent', 'd', '0.0.0', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
            params![catalog, plugin, name, format!("agents/{name}.md")],
        )
        .expect("insert agent");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent' AND name=?3",
                params![catalog, plugin, name],
                |r| r.get(0),
            )
            .expect("agent id");
        let ws_id: i64 = conn
            .query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
                r.get(0)
            })
            .expect("global ws id");
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            params![ws_id, skill_id],
        )
        .expect("enrol agent");
    }

    #[test]
    fn clash_set_reports_names_held_by_two_plugins() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        // `reviewer` shipped by two distinct plugins → a clash.
        insert_enabled_agent(&conn, "cat", "plugin-a", "reviewer");
        insert_enabled_agent(&conn, "cat", "plugin-b", "reviewer");
        // `lonely` shipped by only one plugin → not a clash.
        insert_enabled_agent(&conn, "cat", "plugin-a", "lonely");

        let clashes = agent_name_clash_set(&conn, "global").expect("clash set");
        assert!(clashes.contains("reviewer"), "reviewer must clash");
        assert!(
            !clashes.contains("lonely"),
            "single-plugin name must not clash",
        );
        assert_eq!(clashes.len(), 1);
    }

    #[test]
    fn clash_set_ignores_same_plugin_and_disabled_agents() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        // Two `dup` agents under the SAME plugin: GROUP BY counts distinct
        // (catalog, plugin) pairs, so a single plugin can't self-clash.
        insert_enabled_agent(&conn, "cat", "plugin-a", "dup");
        // Insert a second `dup` row for the same plugin would violate the
        // (catalog, plugin, kind, name) unique index — instead use a second
        // catalog to prove cross-catalog clashes are caught.
        insert_enabled_agent(&conn, "other", "plugin-a", "dup");

        let clashes = agent_name_clash_set(&conn, "global").expect("clash set");
        assert!(
            clashes.contains("dup"),
            "same plugin name across two catalogs clashes",
        );

        // A name held only by a non-enrolled (disabled) agent is not a live
        // clash: insert two rows but enrol neither.
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES ('cat','p1','ghost','agent','d','0.0.0','agents/ghost.md','h',0,0,NULL,'1970-01-01T00:00:00Z'),
                    ('cat','p2','ghost','agent','d','0.0.0','agents/ghost.md','h',0,0,NULL,'1970-01-01T00:00:00Z')",
            [],
        )
        .expect("insert disabled agents");
        let clashes = agent_name_clash_set(&conn, "global").expect("clash set");
        assert!(
            !clashes.contains("ghost"),
            "disabled (non-enrolled) agents do not contribute to the clash set",
        );
    }

    // ---- per-entry progress ticks (#421) --------------------------------

    /// A minimal skill-kind [`PendingSkill`] for the tick-count tests.
    fn pending_skill(name: &str) -> PendingSkill {
        PendingSkill {
            catalog: "cat".into(),
            plugin: "plug".into(),
            name: name.into(),
            kind: EntryKind::Skill,
            description: "desc".into(),
            plugin_version: "0.0.0".into(),
            path: format!("skills/{name}/SKILL.md"),
            when_to_use: None,
            searchable: true,
            user_invocable: true,
        }
    }

    /// `enable_plugin_atomic` fires `on_entry` exactly once per pending
    /// entry — including cheap re-enables that skip the embedder — so a
    /// progress bar sized to `pending.len()` always completes (#421).
    #[test]
    fn enable_ticks_on_entry_once_per_pending_entry() {
        let dir = TempDir::new().unwrap();
        let mut conn = open_db(&dir);
        let pending: Vec<PendingSkill> = ["a", "b", "c"].iter().map(|n| pending_skill(n)).collect();

        let embeds = std::cell::Cell::new(0u32);
        let ticks = std::cell::Cell::new(0u32);
        enable_plugin_atomic(
            &mut conn,
            "global",
            &pending,
            |_| {
                embeds.set(embeds.get() + 1);
                Ok(vec![0.1f32; 384])
            },
            || ticks.set(ticks.get() + 1),
        )
        .expect("enable");
        assert_eq!(embeds.get(), 3, "fresh enable embeds every entry");
        assert_eq!(ticks.get(), 3, "one tick per entry");

        // Cheap re-enable: no embedder calls, but the ticks still cover the
        // full pending set — the bar must not stall at 0/N on a no-op run.
        embeds.set(0);
        ticks.set(0);
        enable_plugin_atomic(
            &mut conn,
            "global",
            &pending,
            |_| {
                embeds.set(embeds.get() + 1);
                Ok(vec![0.1f32; 384])
            },
            || ticks.set(ticks.get() + 1),
        )
        .expect("re-enable");
        assert_eq!(embeds.get(), 0, "unchanged entries skip the embedder");
        assert_eq!(ticks.get(), 3, "ticks still cover every entry");
    }

    /// `reindex_plugin_atomic` ticks once per visited entry, embedder-skips
    /// included (Unchanged rows), mirroring the enable path (#421).
    #[test]
    fn reindex_ticks_on_entry_once_per_pending_entry() {
        let dir = TempDir::new().unwrap();
        let mut conn = open_db(&dir);
        let pending: Vec<PendingSkill> = ["a", "b", "c"].iter().map(|n| pending_skill(n)).collect();
        enable_plugin_atomic(
            &mut conn,
            "global",
            &pending,
            |_| Ok(vec![0.1f32; 384]),
            || {},
        )
        .expect("seed enable");

        let embeds = std::cell::Cell::new(0u32);
        let ticks = std::cell::Cell::new(0u32);
        let summary = reindex_plugin_atomic(
            &mut conn,
            "global",
            "cat",
            "plug",
            &pending,
            false,
            |_| {
                embeds.set(embeds.get() + 1);
                Ok(vec![0.1f32; 384])
            },
            || ticks.set(ticks.get() + 1),
        )
        .expect("reindex");
        assert_eq!(summary.unchanged, 3);
        assert_eq!(embeds.get(), 0, "unchanged rows skip the embedder");
        assert_eq!(ticks.get(), 3, "ticks still cover every visited entry");
    }

    /// Insert a skill or command row and enrol it in `global` with the default
    /// tier (3). Returns the `workspace_skills.skill_id` so callers can set a
    /// specific tier after insertion.
    fn insert_enabled_entry(
        conn: &Connection,
        catalog: &str,
        plugin: &str,
        kind: EntryKind,
        name: &str,
    ) {
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES (?1, ?2, ?3, ?4, 'desc', '0.0.0', ?5, 'h', 1, 1, NULL, '1970-01-01T00:00:00Z')",
            params![
                catalog,
                plugin,
                name,
                kind.as_str(),
                format!("skills/{name}.md"),
            ],
        )
        .expect("insert entry");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind=?3 AND name=?4",
                params![catalog, plugin, kind.as_str(), name],
                |r| r.get(0),
            )
            .expect("skill id");
        let ws_id: i64 = conn
            .query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
                r.get(0)
            })
            .expect("global ws id");
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            params![ws_id, skill_id],
        )
        .expect("enrol entry");
    }

    #[test]
    fn tiered_entries_excludes_agents_and_carries_tier() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Skill, "my-skill");
        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Command, "my-cmd");
        insert_enabled_agent(&conn, "cat", "plug", "my-agent");

        // Set skill → tier 1, command → tier 2.
        set_tier_for_entry(
            &conn,
            "global",
            "cat",
            "plug",
            &EntryKind::Skill,
            "my-skill",
            1,
        )
        .expect("set skill tier");
        set_tier_for_entry(
            &conn,
            "global",
            "cat",
            "plug",
            &EntryKind::Command,
            "my-cmd",
            2,
        )
        .expect("set command tier");

        let entries = tiered_entries_for_workspace(&conn, "global").expect("tiered entries");
        assert_eq!(entries.len(), 2, "agent must be excluded");

        let skill = entries
            .iter()
            .find(|e| e.name == "my-skill")
            .expect("skill");
        assert_eq!(skill.tier, 1, "skill tier must be 1");
        assert_eq!(skill.kind, EntryKind::Skill);

        let cmd = entries.iter().find(|e| e.name == "my-cmd").expect("cmd");
        assert_eq!(cmd.tier, 2, "command tier must be 2");
        assert_eq!(cmd.kind, EntryKind::Command);
    }

    #[test]
    fn set_tier_on_unenabled_entry_is_entry_not_found() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        // Nothing enrolled — any set_tier call must return EntryNotFound.
        let err = set_tier_for_entry(
            &conn,
            "global",
            "cat",
            "plug",
            &EntryKind::Skill,
            "ghost",
            1,
        )
        .expect_err("must fail for unenrolled entry");
        assert!(
            matches!(err, TomeError::EntryNotFound { .. }),
            "expected EntryNotFound, got: {err:?}",
        );
    }

    #[test]
    fn enabled_tiers_for_plugin_reports_distinct_tiers() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        // One plugin with a skill at tier 1 and a command left at the default
        // tier (3). An agent in the same plugin also contributes its tier.
        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Skill, "alpha");
        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Command, "beta");
        insert_enabled_agent(&conn, "cat", "plug", "bot");
        set_tier_for_entry(
            &conn,
            "global",
            "cat",
            "plug",
            &EntryKind::Skill,
            "alpha",
            1,
        )
        .expect("set skill tier");

        let tiers = enabled_tiers_for_plugin(&conn, "global", "cat", "plug").expect("tiers");
        assert!(
            tiers.contains(&1),
            "skill at tier 1 must be present: {tiers:?}"
        );
        assert!(
            tiers.contains(&3),
            "command + agent default to tier 3: {tiers:?}",
        );
        assert!(!tiers.contains(&2), "no entry is at tier 2: {tiers:?}");

        // A plugin with no enabled entries yields an empty set.
        let none = enabled_tiers_for_plugin(&conn, "global", "cat", "ghost").expect("empty");
        assert!(none.is_empty(), "unenrolled plugin has no tiers: {none:?}");
    }

    #[test]
    fn entry_tiers_for_plugin_keys_by_kind_and_name() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Skill, "alpha");
        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Command, "beta");
        insert_enabled_agent(&conn, "cat", "plug", "bot");
        set_tier_for_entry(
            &conn,
            "global",
            "cat",
            "plug",
            &EntryKind::Skill,
            "alpha",
            2,
        )
        .expect("set skill tier");

        let tiers = entry_tiers_for_plugin(&conn, "global", "cat", "plug").expect("entry tiers");
        assert_eq!(
            tiers.get(&(EntryKind::Skill, "alpha".to_owned())),
            Some(&2),
            "skill tier must reflect the explicit set: {tiers:?}",
        );
        // Command + agent default to tier 3.
        assert_eq!(
            tiers.get(&(EntryKind::Command, "beta".to_owned())),
            Some(&3),
        );
        assert_eq!(tiers.get(&(EntryKind::Agent, "bot".to_owned())), Some(&3));
        // A non-enrolled entry is absent (not tier 0).
        assert!(!tiers.contains_key(&(EntryKind::Skill, "ghost".to_owned())));
    }

    #[test]
    fn set_tier_for_plugin_bulk() {
        let dir = TempDir::new().unwrap();
        let conn = open_db(&dir);

        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Skill, "alpha");
        insert_enabled_entry(&conn, "cat", "plug", EntryKind::Skill, "beta");
        // An agent in the same plugin — must be left untouched (not counted).
        insert_enabled_agent(&conn, "cat", "plug", "bot");

        let updated =
            set_tier_for_plugin(&conn, "global", "cat", "plug", 1).expect("bulk set tier");
        assert_eq!(updated, 2, "only skill/command rows should be updated");

        let entries = tiered_entries_for_workspace(&conn, "global").expect("tiered entries");
        assert!(
            entries.iter().all(|e| e.tier == 1),
            "all tierable entries must have tier 1",
        );
    }
}
