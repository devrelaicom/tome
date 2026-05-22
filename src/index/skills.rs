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

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::TomeError;

/// One row in the `skills` table after a successful read.
#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub id: i64,
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub description: String,
    pub plugin_version: String,
    pub path: String,
    pub content_hash: String,
    pub enabled: bool,
    pub indexed_at: String,
}

/// Inputs to [`enable_plugin_atomic`]. The text Tome embeds is composed as
/// `name + "\n\n" + description` (research §R8); the caller supplies the
/// raw fields plus the on-disk path.
#[derive(Debug, Clone)]
pub struct PendingSkill {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub description: String,
    pub plugin_version: String,
    pub path: String,
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

/// The text composition Tome hashes and embeds. By construction two skills
/// with the same `(name, description)` produce the same hash, which is the
/// condition under which FR-006 / FR-032 perform a no-op refresh.
pub fn content_hash(name: &str, description: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\n\n");
    hasher.update(description.as_bytes());
    hex::encode(hasher.finalize())
}

/// Embedding text for a `(name, description)` pair. Same composition as
/// [`content_hash`] so the embedder sees exactly the bytes whose digest we
/// stored.
pub fn embedding_text(name: &str, description: &str) -> String {
    format!("{name}\n\n{description}")
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

/// Look up a single skill by identity, with enablement evaluated against
/// `workspace_name`. Returns `Ok(None)` when absent.
pub fn find(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    name: &str,
) -> Result<Option<SkillRecord>, TomeError> {
    let join = workspace_join(4);
    let sql = format!(
        "SELECT s.id, s.catalog, s.plugin, s.name, s.description, s.plugin_version, s.path,
                s.content_hash, {ENABLED_EXPR}, s.indexed_at
         FROM skills AS s
         {join}
         WHERE s.catalog = ?1 AND s.plugin = ?2 AND s.name = ?3"
    );
    conn.query_row(
        &sql,
        params![catalog, plugin, name, workspace_name],
        |row| {
            Ok(SkillRecord {
                id: row.get(0)?,
                catalog: row.get(1)?,
                plugin: row.get(2)?,
                name: row.get(3)?,
                description: row.get(4)?,
                plugin_version: row.get(5)?,
                path: row.get(6)?,
                content_hash: row.get(7)?,
                enabled: row.get::<_, i64>(8)? != 0,
                indexed_at: row.get(9)?,
            })
        },
    )
    .optional()
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("find skill: {e}")))
}

/// List every skill row for one plugin, ordered by name. Enablement is
/// evaluated against `workspace_name`.
pub fn list_for_plugin(
    conn: &Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<Vec<SkillRecord>, TomeError> {
    let join = workspace_join(3);
    let sql = format!(
        "SELECT s.id, s.catalog, s.plugin, s.name, s.description, s.plugin_version, s.path,
                s.content_hash, {ENABLED_EXPR}, s.indexed_at
         FROM skills AS s
         {join}
         WHERE s.catalog = ?1 AND s.plugin = ?2
         ORDER BY s.name"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare list: {e}")))?;
    let rows = stmt
        .query_map(params![catalog, plugin, workspace_name], |row| {
            Ok(SkillRecord {
                id: row.get(0)?,
                catalog: row.get(1)?,
                plugin: row.get(2)?,
                name: row.get(3)?,
                description: row.get(4)?,
                plugin_version: row.get(5)?,
                path: row.get(6)?,
                content_hash: row.get(7)?,
                enabled: row.get::<_, i64>(8)? != 0,
                indexed_at: row.get(9)?,
            })
        })
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

/// Insert a new skill row + matching embedding, or update an existing row
/// in place. Run inside an already-open transaction by the caller.
/// Phase 4 / F9: the `enabled` column is gone; enablement is recorded
/// separately via [`upsert_workspace_skill`].
fn upsert_skill(
    tx: &rusqlite::Transaction<'_>,
    pending: &PendingSkill,
    hash: &str,
    embedding: &[f32],
    now: &str,
) -> Result<i64, TomeError> {
    tx.execute(
        "INSERT INTO skills
            (catalog, plugin, name, description, plugin_version, path,
             content_hash, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(catalog, plugin, name) DO UPDATE SET
            description    = excluded.description,
            plugin_version = excluded.plugin_version,
            path           = excluded.path,
            content_hash   = excluded.content_hash,
            indexed_at     = excluded.indexed_at",
        params![
            pending.catalog,
            pending.plugin,
            pending.name,
            pending.description,
            pending.plugin_version,
            pending.path,
            hash,
            now,
        ],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("upsert skill row: {e}")))?;

    let id: i64 = tx
        .query_row(
            "SELECT id FROM skills WHERE catalog = ?1 AND plugin = ?2 AND name = ?3",
            params![pending.catalog, pending.plugin, pending.name],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("look up skill id: {e}")))?;

    // vec0 virtual tables do not support `INSERT OR REPLACE` or `ON CONFLICT`,
    // so we DELETE-then-INSERT. The DELETE is a no-op when there's no prior
    // row, so this is correct for both first-time inserts and re-embeds.
    tx.execute(
        "DELETE FROM skill_embeddings WHERE skill_id = ?1",
        params![id],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("drop prior embedding: {e}")))?;
    let bytes = embedding_to_bytes(embedding);
    tx.execute(
        "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
        params![id, bytes],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("insert embedding: {e}")))?;

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
pub fn enable_plugin_atomic<F>(
    conn: &mut Connection,
    workspace_name: &str,
    pending: &[PendingSkill],
    mut embed: F,
) -> Result<EnableSummary, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
{
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin enable tx: {e}")))?;

    let now = now_rfc3339();
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let mut newly_embedded: u32 = 0;

    for skill in pending {
        let hash = content_hash(&skill.name, &skill.description);

        let existing: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, content_hash FROM skills
                 WHERE catalog = ?1 AND plugin = ?2 AND name = ?3",
                params![skill.catalog, skill.plugin, skill.name],
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
                // embedder invocation, no embedding rewrite.
                tx.execute(
                    "UPDATE skills SET plugin_version = ?2, path = ?3, indexed_at = ?4
                     WHERE id = ?1",
                    params![id, skill.plugin_version, skill.path, now],
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
                let embedding = embed(&embedding_text(&skill.name, &skill.description))?;
                let id = upsert_skill(&tx, skill, &hash, &embedding, &now)?;
                newly_embedded = newly_embedded.saturating_add(1);
                id
            }
        };

        // Enrol the skill in the resolved workspace (Phase 4 / F11a
        // replacement for F9's privileged-`global`-only write).
        upsert_workspace_skill(&tx, workspace_name, skill_id, now_unix)?;
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
pub fn reindex_plugin_atomic<F>(
    conn: &mut Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    pending: &[PendingSkill],
    force: bool,
    mut embed: F,
) -> Result<ReindexSummary, TomeError>
where
    F: FnMut(&str) -> Result<Vec<f32>, TomeError>,
{
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin reindex tx: {e}")))?;

    let now = now_rfc3339();
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let mut summary = ReindexSummary::default();

    // Snapshot existing rows once per call. We'll diff against `pending`
    // below and use the leftover set for the Removed branch.
    let mut existing: std::collections::HashMap<String, (i64, String)> =
        std::collections::HashMap::new();
    {
        let mut stmt = tx
            .prepare(
                "SELECT id, name, content_hash FROM skills
                 WHERE catalog = ?1 AND plugin = ?2",
            )
            .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare existing: {e}")))?;
        let rows = stmt
            .query_map(params![catalog, plugin], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let hash: String = row.get(2)?;
                Ok((name, (id, hash)))
            })
            .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query existing: {e}")))?;
        for row in rows {
            let (name, value) = row.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("collect existing: {e}"))
            })?;
            existing.insert(name, value);
        }
    }

    // Pass 1 — Added / Modified / Unchanged.
    for skill in pending {
        let hash = content_hash(&skill.name, &skill.description);

        let skill_id = match existing.remove(&skill.name) {
            Some((id, stored_hash)) if stored_hash == hash && !force => {
                // Unchanged: touch metadata only.
                tx.execute(
                    "UPDATE skills SET plugin_version = ?2, path = ?3, indexed_at = ?4
                     WHERE id = ?1",
                    params![id, skill.plugin_version, skill.path, now],
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
                let embedding = embed(&embedding_text(&skill.name, &skill.description))?;
                let id = upsert_skill(&tx, skill, &hash, &embedding, &now)?;
                summary.modified = summary.modified.saturating_add(1);
                id
            }
            None => {
                // Added.
                let embedding = embed(&embedding_text(&skill.name, &skill.description))?;
                let id = upsert_skill(&tx, skill, &hash, &embedding, &now)?;
                summary.added = summary.added.saturating_add(1);
                id
            }
        };

        // Re-assert the resolved-workspace enrolment for every visited
        // skill (idempotent — PK keyed on (workspace_id, skill_id)).
        upsert_workspace_skill(&tx, workspace_name, skill_id, now_unix)?;
    }

    // Pass 2 — Removed: anything still left in `existing` is on-index but
    // not on-disk. Drop the row + its embedding.
    for (_name, (id, _hash)) in existing {
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
