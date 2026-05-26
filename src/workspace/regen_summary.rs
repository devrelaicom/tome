//! `tome workspace regen-summary [<name>]` — regenerate cached
//! summaries.
//!
//! Phase 4 / US2.a-2. Contract reference:
//! [`contracts/workspace-commands.md` §`tome workspace regen-summary`]
//! and [`contracts/summariser.md`].
//!
//! ## Algorithm
//!
//! 1. Validate the workspace exists (exit 13 otherwise).
//! 2. Acquire the central advisory lock.
//! 3. Load enabled plugins for the workspace via the `workspace_skills`
//!    × `skills` join, grouped by `(catalog, plugin)`.
//! 4. Construct a [`PluginSummariesInput`]; call
//!    [`Summariser::summarise`].
//! 5. On `SummariserFailure`, bubble (exit 24). Prior cached summary
//!    (if any) is left in place — we have not written yet.
//! 6. Emit a `tracing::warn!` if the short summary exceeds 800 chars
//!    or the long summary exceeds 2500 chars (FR-425). The value is
//!    still cached.
//! 7. Update the workspace's `settings.toml` `[summaries]` section
//!    atomically via `toml_edit::DocumentMut` so other sections
//!    (`[[catalogs]]`, `harnesses`) are preserved.
//! 8. Rewrite `<root>/workspaces/<name>/RULES.md` body from `long`.
//! 9. Release the lock.
//! 10. Sync the new central RULES.md to every bound project's marker
//!     RULES.md via
//!     [`crate::workspace::sync::sync_workspace_rules_to_bound_projects`].
//!
//! ## Forward-progress / failure-modes
//!
//! `regen-summary` is the explicit summarisation command. Failure here
//! is the result, not a side-effect — the prior cached summary stays
//! in place (FR-385's "forward-progress" carve-out does NOT apply
//! because the underlying skill-state hasn't changed).

use std::path::Path;
use std::str::FromStr;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::store;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock};
use crate::paths::Paths;
use crate::summarise::{
    LONG_MAX_CHARS, PluginSummariesInput, PluginSummaryItem, SHORT_MAX_CHARS, SkillSummaryItem,
    Summariser,
};
use crate::workspace::WorkspaceName;
use crate::workspace::sync::sync_workspace_rules_to_bound_projects;

// Length-window caps live in [`crate::summarise`] (US4.d-1
// consolidation — there used to be a duplicate pair here that drifted
// 100 chars on the long bound, firing the warn at a different boundary
// than the inference loop). Re-imported above; no module-local
// re-export so consumers don't accidentally re-fork on the next major
// edit.

/// Outcome of [`regen`]. Serialised by the CLI's `--json` mode.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RegenSummaryOutcome {
    /// The workspace whose summary was regenerated.
    pub workspace: WorkspaceName,
    /// Character count of the new short summary.
    pub short_chars: usize,
    /// Character count of the new long summary.
    pub long_chars: usize,
    /// Number of bound projects whose marker RULES.md was synced.
    pub bound_projects_synced: u32,
}

/// Regenerate the cached short + long summaries for `name`. See
/// module-level docs for the full algorithm.
pub fn regen(
    name: &WorkspaceName,
    summariser: &dyn Summariser,
    paths: &Paths,
) -> Result<RegenSummaryOutcome, TomeError> {
    if let Some(parent) = paths.index_lock.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let lock = acquire_lock(&paths.index_lock)?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    // Workspace membership.
    // Polish R-M7: route through the consolidated helper.
    let workspace_id: i64 = crate::index::workspaces::resolve_id_required(&conn, name)?;

    let input = load_summariser_input(&conn, workspace_id)?;

    // Summarise. Failure here exits 24 with prior cache untouched.
    // Note: the advisory lock + DB conn are held across the summariser
    // call. This is deliberate — the regen path is single-action and
    // ordering is important for the post-summarise `last_used_at` bump.
    // Performance trade-off documented in `us2-disposition.md` (R-M5
    // deferred).
    let output = summariser.summarise(&input)?;

    // Length-window warning per FR-425.
    let short_chars = output.short.chars().count();
    let long_chars = output.long.chars().count();
    if short_chars > SHORT_MAX_CHARS {
        tracing::warn!(
            workspace = name.as_str(),
            short_chars,
            limit = SHORT_MAX_CHARS,
            "summariser output exceeds recommended length window (short)",
        );
    }
    if long_chars > LONG_MAX_CHARS {
        tracing::warn!(
            workspace = name.as_str(),
            long_chars,
            limit = LONG_MAX_CHARS,
            "summariser output exceeds recommended length window (long)",
        );
    }

    // Write the settings.toml `[summaries]` section preserving any
    // other sections (`[[catalogs]]`, `harnesses`).
    let now = OffsetDateTime::now_utc();
    let generated_at = now
        .format(&Rfc3339)
        .map_err(|e| TomeError::Io(std::io::Error::other(format!("rfc3339 format: {e}"))))?;

    let settings_path = paths.workspace_settings_file(name);
    let updated_settings = update_settings_summaries(
        &settings_path,
        name.as_str(),
        &output.short,
        &output.long,
        &generated_at,
    )?;
    store::write_atomic(&settings_path, updated_settings.as_bytes())?;

    // Rewrite RULES.md from the long summary.
    let rules_path = paths.workspace_rules_file(name);
    store::write_atomic(&rules_path, output.long.as_bytes())?;

    // FR-411: bump `last_used_at` on the workspaces row after a
    // successful summariser invocation. The advisory lock is still held;
    // no other writer can be mutating the row.
    let now_unix = now.unix_timestamp();
    conn.execute(
        "UPDATE workspaces SET last_used_at = ?1 WHERE name = ?2",
        rusqlite::params![now_unix, name.as_str()],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "regen-summary: bump last_used_at for `{}`: {e}",
            name.as_str(),
        ))
    })?;

    // Drop the DB handle BEFORE releasing the lock so any WAL checkpoint
    // completes inside the lock window.
    drop(conn);
    // Release the lock BEFORE syncing bound projects — the sync helper
    // does not need the central-DB write lock.
    drop(lock);

    let bound_projects_synced = sync_workspace_rules_to_bound_projects(name, paths)?;

    Ok(RegenSummaryOutcome {
        workspace: name.clone(),
        short_chars,
        long_chars,
        bound_projects_synced,
    })
}

/// Collect the enabled plugins + their skills for the workspace, in
/// stable `(catalog, plugin, name)` order. Skill descriptions are read
/// from the `skills.description` column.
fn load_summariser_input(
    conn: &rusqlite::Connection,
    workspace_id: i64,
) -> Result<PluginSummariesInput, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.catalog, s.plugin, s.name, COALESCE(s.description, '')
             FROM workspace_skills AS ws
             JOIN skills           AS s ON s.id = ws.skill_id
             WHERE ws.workspace_id = ?1
             ORDER BY s.catalog, s.plugin, s.name",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "regen-summary: prepare load_summariser_input: {e}"
            ))
        })?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id], |row| {
            let catalog: String = row.get(0)?;
            let plugin: String = row.get(1)?;
            let name: String = row.get(2)?;
            let description: String = row.get(3)?;
            Ok((catalog, plugin, name, description))
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "regen-summary: query load_summariser_input: {e}"
            ))
        })?;

    // Group by (catalog, plugin) preserving the ORDER BY above.
    let mut plugins: Vec<PluginSummaryItem> = Vec::new();
    for row in rows {
        let (catalog, plugin, skill_name, skill_description) = row.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "regen-summary: read row for load_summariser_input: {e}"
            ))
        })?;
        match plugins.last_mut() {
            Some(last) if last.catalog == catalog && last.plugin == plugin => {
                last.skills.push(SkillSummaryItem {
                    name: skill_name,
                    description: skill_description,
                });
            }
            _ => {
                plugins.push(PluginSummaryItem {
                    catalog,
                    plugin,
                    // US4.a (production LlamaSummariser) will surface
                    // plugin.json's description here. The stub doesn't
                    // consume it; leaving empty is a documented
                    // simplification.
                    description: String::new(),
                    skills: vec![SkillSummaryItem {
                        name: skill_name,
                        description: skill_description,
                    }],
                });
            }
        }
    }

    Ok(PluginSummariesInput { plugins })
}

/// Read `settings.toml` (or fabricate the minimal `name = "<workspace>"`
/// scaffold if absent), update the `[summaries]` section, and return
/// the new serialised body. Uses `toml_edit::DocumentMut` so
/// `[[catalogs]]`, `harnesses`, comments, and key order survive the
/// rewrite untouched.
fn update_settings_summaries(
    settings_path: &Path,
    workspace_name: &str,
    short: &str,
    long: &str,
    generated_at: &str,
) -> Result<String, TomeError> {
    let body =
        match crate::util::bounded_read_to_string(settings_path, crate::util::TOME_CONFIG_MAX) {
            Ok(b) => b,
            Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                // Fall back to a minimal scaffold matching workspace::init's
                // render_settings_toml output. Init lands settings.toml
                // before regen-summary can run, so this branch is mostly
                // defensive.
                format!("name = \"{workspace_name}\"\n")
            }
            Err(e) => return Err(e),
        };

    let mut doc: toml_edit::DocumentMut =
        body.parse()
            .map_err(|e: toml_edit::TomlError| TomeError::WorkspaceMalformed {
                path: settings_path.to_path_buf(),
                reason: format!("workspace settings.toml is unparsable: {e}"),
            })?;

    let summaries_item = doc
        .as_table_mut()
        .entry("summaries")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
    let summaries_table =
        summaries_item
            .as_table_mut()
            .ok_or_else(|| TomeError::WorkspaceMalformed {
                path: settings_path.to_path_buf(),
                reason: "`[summaries]` is present but not a table".to_owned(),
            })?;

    summaries_table["short"] = toml_edit::value(short);
    summaries_table["long"] = toml_edit::value(long);
    // Emit `generated_at` as an unquoted TOML datetime literal (per
    // contracts/workspace-commands.md's example). Falls back to a basic
    // string when the RFC 3339 input isn't parseable by `toml_edit`'s
    // datetime grammar — should never happen with `OffsetDateTime`'s
    // canonical formatter, but the fallback keeps the write infallible.
    summaries_table["generated_at"] = match toml_edit::Datetime::from_str(generated_at) {
        Ok(dt) => toml_edit::value(dt),
        Err(_) => toml_edit::value(generated_at),
    };

    Ok(doc.to_string())
}
