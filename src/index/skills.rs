//! CRUD over the `skills` table plus the atomic enable orchestrator.
//!
//! Per FR-004, enabling a plugin is one indivisible step: either every skill
//! is embedded and inserted, or nothing changes on disk. The
//! [`enable_plugin_atomic`] helper wraps embed-and-insert in a single
//! SQLite transaction so a SIGINT or embedder failure rolls back cleanly.
//!
//! Per FR-006 / FR-032, an enable / refresh of a skill whose
//! `(name, description)` text composition has not changed is a no-op embed:
//! we keep the existing vector and only flip `enabled = 1`. The diff is
//! detected via [`content_hash`].
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

/// Look up a single skill by identity. Returns `Ok(None)` when absent.
pub fn find(
    conn: &Connection,
    catalog: &str,
    plugin: &str,
    name: &str,
) -> Result<Option<SkillRecord>, TomeError> {
    conn.query_row(
        "SELECT id, catalog, plugin, name, description, plugin_version, path,
                content_hash, enabled, indexed_at
         FROM skills WHERE catalog = ?1 AND plugin = ?2 AND name = ?3",
        params![catalog, plugin, name],
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

/// List every skill row for one plugin, ordered by name.
pub fn list_for_plugin(
    conn: &Connection,
    catalog: &str,
    plugin: &str,
) -> Result<Vec<SkillRecord>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, catalog, plugin, name, description, plugin_version, path,
                    content_hash, enabled, indexed_at
             FROM skills WHERE catalog = ?1 AND plugin = ?2
             ORDER BY name",
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare list: {e}")))?;
    let rows = stmt
        .query_map(params![catalog, plugin], |row| {
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

/// Flip every row for `(catalog, plugin)` to `enabled = 0`. Embeddings are
/// retained so a subsequent re-enable is cheap (FR-005, FR-006).
pub fn mark_all_disabled_for_plugin(
    conn: &Connection,
    catalog: &str,
    plugin: &str,
) -> Result<u32, TomeError> {
    let affected = conn
        .execute(
            "UPDATE skills SET enabled = 0 WHERE catalog = ?1 AND plugin = ?2",
            params![catalog, plugin],
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
fn upsert_skill(
    tx: &rusqlite::Transaction<'_>,
    pending: &PendingSkill,
    hash: &str,
    embedding: &[f32],
    now: &str,
) -> Result<i64, TomeError> {
    let enabled: i64 = 1;
    tx.execute(
        "INSERT INTO skills
            (catalog, plugin, name, description, plugin_version, path,
             content_hash, enabled, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(catalog, plugin, name) DO UPDATE SET
            description    = excluded.description,
            plugin_version = excluded.plugin_version,
            path           = excluded.path,
            content_hash   = excluded.content_hash,
            enabled        = excluded.enabled,
            indexed_at     = excluded.indexed_at",
        params![
            pending.catalog,
            pending.plugin,
            pending.name,
            pending.description,
            pending.plugin_version,
            pending.path,
            hash,
            enabled,
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

    let bytes = embedding_to_bytes(embedding);
    tx.execute(
        "INSERT OR REPLACE INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
        params![id, bytes],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("upsert embedding: {e}")))?;

    Ok(id)
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

        match existing {
            Some((id, stored_hash)) if stored_hash == hash => {
                tx.execute(
                    "UPDATE skills SET enabled = 1, plugin_version = ?2, path = ?3,
                                       indexed_at = ?4
                     WHERE id = ?1",
                    params![id, skill.plugin_version, skill.path, now],
                )
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!(
                        "flip enabled for {}/{}: {e}",
                        skill.plugin, skill.name
                    ))
                })?;
            }
            _ => {
                let embedding = embed(&embedding_text(&skill.name, &skill.description))?;
                upsert_skill(&tx, skill, &hash, &embedding, &now)?;
                newly_embedded = newly_embedded.saturating_add(1);
            }
        }
    }

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit enable tx: {e}")))?;

    let total_skills = u32::try_from(pending.len()).unwrap_or(u32::MAX);
    Ok(EnableSummary {
        total_skills,
        newly_embedded,
    })
}
