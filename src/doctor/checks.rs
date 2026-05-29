//! Per-subsystem check functions used by `tome doctor`'s assembly path.
//! Each function is pure compute over `(paths, scope, …)`; they share
//! the read-only-DB convention with `tome status` (`PRAGMA
//! integrity_check`, no advisory lock).
//!
//! Models / index / drift are delegated to `commands::status`'s
//! already-`pub` helpers so the two surfaces stay consistent — doctor's
//! checks must report the same health values status would for the
//! overlapping subsystems.
//!
//! Phase 4 checks:
//! - `check_catalogs` enumerates the resolved scope's catalogs and
//!   classifies each on-disk clone.
//! - `harness_detect::probe` (sibling module) handles the harness list.
//!
//! Phase 5 / US5.b checks (read-only per FR-124 — none of these
//! lazy-create plugin-data / workspace-data dirs):
//! - `build_prompts_report` enumerates user-invocable entries via
//!   the production `PromptRegistry::build_for_workspace`.
//! - `detect_orphan_data_dirs` walks the central + per-workspace
//!   plugin-data trees and compares against `workspace_skills`.
//! - `count_entries_by_kind` aggregates by `kind` + flags entries
//!   whose source mtime exceeds the stored `indexed_at`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::catalog::manifest::CatalogManifest;
use crate::commands::plugin::registry_seeds;
use crate::doctor::report::{
    CatalogCacheHealth, CatalogCacheState, EntryCountsByKind, OrphanDataDirReport, PromptsReport,
};
use crate::error::TomeError;
use crate::index::{self, OpenOptions, workspace_catalogs};
use crate::mcp::prompts::PromptRegistry;
use crate::paths::Paths;
use crate::workspace::{Scope, WorkspaceName};

/// Enumerate every catalog in the resolved scope's enrolments (via
/// `workspace_catalogs`) and classify the on-disk clone:
///
/// - Missing → cache directory not on disk.
/// - NotARepo → directory exists but lacks `.git/`.
/// - ManifestInvalid → directory + `.git/` present but `tome-catalog.toml`
///   is missing or unparsable.
/// - Ok → everything parses.
///
/// `tome catalog show <name>` is the corresponding read-only inspect
/// surface; doctor's check is intentionally lighter (existence + parse
/// only, no validation of plugin sources).
///
/// Returns an empty `Vec` when the central DB is absent or the scope's
/// workspace has no enrolments.
pub fn check_catalogs(paths: &Paths, scope: &Scope) -> Result<Vec<CatalogCacheHealth>, TomeError> {
    let workspace_name = scope.name().as_str();

    let enrolments = if paths.index_db.is_file() {
        let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: embedder_seed,
                reranker: reranker_seed,
                summariser: summariser_seed,
            },
        )?;
        workspace_catalogs::list_for_workspace(&conn, workspace_name).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut out = Vec::with_capacity(enrolments.len());

    // Step 1: classify every catalog the resolved workspace enrols.
    let mut referenced_paths: HashSet<PathBuf> = HashSet::new();
    for e in &enrolments {
        let cache_path = paths.cache_dir_for(&e.url);
        referenced_paths.insert(cache_path.clone());
        let state = classify_clone(&cache_path);
        out.push(CatalogCacheHealth {
            name: e.catalog_name.clone(),
            url: e.url.clone(),
            cache_path,
            state,
        });
    }

    // Step 2: enumerate on-disk clones at `paths.catalogs_dir` and
    // surface any directory NOT referenced by the resolved config as
    // `Orphan`. Per `catalog-extensions-p3.md` §"Doctor reporting"
    // bullet 4: cache exists but no config references it. The URL is
    // unknown at the doctor level (we'd need to parse the manifest to
    // recover the original source URL); leaving it empty keeps the
    // JSON wire shape simple — the user only needs the cache path
    // to act on it.
    if paths.catalogs_dir.is_dir() {
        let entries = match std::fs::read_dir(&paths.catalogs_dir) {
            Ok(it) => it,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %paths.catalogs_dir.display(),
                    "doctor: read_dir(catalogs_dir) failed during orphan walk; skipping",
                );
                return Ok(out);
            }
        };
        for de in entries.flatten() {
            let p = de.path();
            if !p.is_dir() {
                continue;
            }
            if referenced_paths.contains(&p) {
                continue;
            }
            // Only orphans we can confidently classify (a directory
            // with `.git/` + parsable manifest is a real abandoned
            // catalog clone). A half-broken directory shows up as
            // `Missing` / `NotARepo` / `ManifestInvalid` on the
            // referenced-catalog path; unreferenced half-broken dirs
            // are unactionable noise and we skip them.
            let manifest = p.join("tome-catalog.toml");
            if !p.join(".git").is_dir() || !manifest.is_file() {
                continue;
            }
            // Unknown URL (we don't re-parse just to recover the
            // source); the user has the path which is what they need
            // to remove it.
            out.push(CatalogCacheHealth {
                name: p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                url: String::new(),
                cache_path: p,
                state: crate::doctor::report::CatalogCacheState::Orphan,
            });
        }
    }

    Ok(out)
}

/// Phase 4 / F2a: the Phase 3 opt-in `workspaces.txt` registry is gone.
/// Workspace bindings now live in the central database's
/// `workspace_projects` table (F11). The function is retained as a
/// `present: false, tracked: 0` stub so the doctor JSON envelope shape
/// stays unchanged until F11 promotes a richer per-binding report.
pub fn check_workspace_registry(_paths: &Paths) -> crate::doctor::report::WorkspaceRegistryStatus {
    crate::doctor::report::WorkspaceRegistryStatus {
        present: false,
        tracked: 0,
    }
}

/// Classify a single clone path. Pure FS reads — no network, no git
/// shell-out.
fn classify_clone(path: &Path) -> CatalogCacheState {
    if !path.exists() {
        return CatalogCacheState::Missing;
    }
    if !path.is_dir() {
        // A file at the cache path is degenerate but not impossible
        // (manual filesystem editing). Treat as Missing — the rebuild
        // path is the same.
        return CatalogCacheState::Missing;
    }
    let git_dir = path.join(".git");
    if !git_dir.exists() {
        return CatalogCacheState::NotARepo;
    }
    let manifest_path = path.join("tome-catalog.toml");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return CatalogCacheState::ManifestInvalid;
    };
    // We deliberately use lenient parsing — doctor only reports whether
    // the manifest is readable, not whether every plugin entry is
    // resolvable. `tome catalog show` is the surface for the deeper
    // validation.
    if CatalogManifest::parse_and_validate(&manifest_path, path, &bytes).is_err() {
        return CatalogCacheState::ManifestInvalid;
    }
    CatalogCacheState::Ok
}

// -------------------------------------------------------------------------
// Phase 5 / US5.b — read-only doctor surfaces.
// -------------------------------------------------------------------------

/// Build the prompts-surface report for the resolved workspace via the
/// production [`PromptRegistry::build_for_workspace`] — the SAME path
/// the MCP server runs at startup. Doctor surfaces every collision and
/// every derived prompt name so authors can confirm what their entries
/// will look like over the wire.
///
/// Failures during registry build (e.g. missing entry body files) are
/// not fatal — `PromptRegistry::build_for_workspace` already collapses
/// per-entry parse failures to warn-and-skip. A registry-wide DB error
/// surfaces here as an `Err`.
///
/// Returns `None` (mapped to absent JSON field by the doctor assembler)
/// when the workspace has zero user-invocable entries AND zero
/// collisions — same convention as the Phase 4 surfaces.
pub fn build_prompts_report(
    workspace: &WorkspaceName,
    paths: &Paths,
    conn: &rusqlite::Connection,
) -> Result<PromptsReport, TomeError> {
    let registry = PromptRegistry::build_for_workspace(workspace, paths, conn, false)?;
    let prompts = registry.descriptors();
    let collisions = registry.collisions.clone();
    Ok(PromptsReport {
        prompts,
        collisions,
    })
}

/// Detect orphan plugin-data + workspace-data directories per
/// `contracts/doctor-extensions-p5.md` § Detection algorithm.
///
/// Algorithm:
/// 1. Walk `<root>/plugin-data/<catalog>/<plugin>/` and record every
///    `(catalog, plugin)` pair on disk.
/// 2. Read `SELECT DISTINCT catalog, plugin FROM skills s JOIN
///    workspace_skills ws ON ws.skill_id = s.id` to build the
///    enabled-anywhere set.
/// 3. On-disk pairs NOT in the enabled set → `plugin_data` orphans.
/// 4. Walk `<root>/workspaces/<ws>/plugin-data/<catalog>/<plugin>/` for
///    every workspace dir on disk. For each, look up the per-workspace
///    `workspace_skills` enrolment. Not enrolled → `workspace_data`
///    orphan.
///
/// FR-124 invariant: this function only reads — no `create_dir_all`,
/// no writes. Missing top-level dirs (no plugin-data tree ever written)
/// produce empty vectors.
pub fn detect_orphan_data_dirs(
    paths: &Paths,
    conn: &rusqlite::Connection,
) -> Result<OrphanDataDirReport, TomeError> {
    // Set of (catalog, plugin) pairs enabled in ANY workspace.
    let mut any_enrolment_pairs: HashSet<(String, String)> = HashSet::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT s.catalog, s.plugin
                 FROM skills AS s
                 JOIN workspace_skills AS ws ON ws.skill_id = s.id",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "prepare orphan plugin-data enrolment query: {e}"
                ))
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "query orphan plugin-data enrolment: {e}"
                ))
            })?;
        for r in rows {
            let pair = r.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "collect orphan plugin-data enrolment row: {e}"
                ))
            })?;
            any_enrolment_pairs.insert(pair);
        }
    }

    // Map of workspace_name -> set of (catalog, plugin) pairs enrolled.
    let mut per_workspace_pairs: HashMap<String, HashSet<(String, String)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT w.name, s.catalog, s.plugin
                 FROM workspaces AS w
                 JOIN workspace_skills AS ws ON ws.workspace_id = w.id
                 JOIN skills        AS s  ON s.id = ws.skill_id",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "prepare orphan workspace-data enrolment query: {e}"
                ))
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "query orphan workspace-data enrolment: {e}"
                ))
            })?;
        for r in rows {
            let (ws, cat, plug) = r.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "collect orphan workspace-data enrolment row: {e}"
                ))
            })?;
            per_workspace_pairs
                .entry(ws)
                .or_default()
                .insert((cat, plug));
        }
    }

    let plugin_data = walk_plugin_data_for_orphans(&paths.plugin_data_root(), |pair| {
        !any_enrolment_pairs.contains(pair)
    });

    let workspace_data =
        walk_workspace_plugin_data_for_orphans(&paths.workspaces_dir, &per_workspace_pairs);

    Ok(OrphanDataDirReport {
        plugin_data,
        workspace_data,
    })
}

/// Walk `<root>/plugin-data/<catalog>/<plugin>/` and call `is_orphan`
/// for each `(catalog, plugin)` pair to filter. Returned paths are
/// absolute (joined under `root`). Skips symlinks and non-directories
/// at every level (S-M6 defence-in-depth, mirrored from
/// `orphan_cleanup`).
fn walk_plugin_data_for_orphans<F>(plugin_data_root: &Path, mut is_orphan: F) -> Vec<PathBuf>
where
    F: FnMut(&(String, String)) -> bool,
{
    let mut out: Vec<PathBuf> = Vec::new();
    let catalogs = match std::fs::read_dir(plugin_data_root) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %plugin_data_root.display(),
                "doctor: read_dir(plugin_data) failed during orphan walk; skipping",
            );
            return out;
        }
    };
    for c_entry in catalogs.flatten() {
        let c_path = c_entry.path();
        // Symlink defence + dir-only.
        let Ok(meta) = std::fs::symlink_metadata(&c_path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        let Some(catalog_name) = c_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(plugins) = std::fs::read_dir(&c_path) else {
            continue;
        };
        for p_entry in plugins.flatten() {
            let p_path = p_entry.path();
            let Ok(p_meta) = std::fs::symlink_metadata(&p_path) else {
                continue;
            };
            if p_meta.file_type().is_symlink() || !p_meta.is_dir() {
                continue;
            }
            let Some(plugin_name) = p_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let pair = (catalog_name.to_owned(), plugin_name.to_owned());
            if is_orphan(&pair) {
                out.push(p_path);
            }
        }
    }
    out.sort();
    out
}

/// Walk `<root>/workspaces/<ws>/plugin-data/<catalog>/<plugin>/` and
/// emit one orphan for every triple whose `(catalog, plugin)` isn't in
/// the per-workspace enrolment set. A workspace dir with no
/// `plugin-data/` subdir contributes zero orphans.
fn walk_workspace_plugin_data_for_orphans(
    workspaces_dir: &Path,
    per_workspace_pairs: &HashMap<String, HashSet<(String, String)>>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let workspaces = match std::fs::read_dir(workspaces_dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %workspaces_dir.display(),
                "doctor: read_dir(workspaces_dir) failed during orphan walk; skipping",
            );
            return out;
        }
    };
    for w_entry in workspaces.flatten() {
        let w_path = w_entry.path();
        let Ok(w_meta) = std::fs::symlink_metadata(&w_path) else {
            continue;
        };
        if w_meta.file_type().is_symlink() || !w_meta.is_dir() {
            continue;
        }
        let Some(workspace_name) = w_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let plugin_data_root = w_path.join("plugin-data");
        let empty: HashSet<(String, String)> = HashSet::new();
        let enrolled = per_workspace_pairs.get(workspace_name).unwrap_or(&empty);
        let dirs = walk_plugin_data_for_orphans(&plugin_data_root, |pair| !enrolled.contains(pair));
        out.extend(dirs);
    }
    out.sort();
    out
}

/// Per-kind entry counts for the resolved workspace per
/// `contracts/doctor-extensions-p5.md` § `entry_counts`. The
/// `pending_re_embedding` heuristic compares each enabled entry's
/// resolved source-file mtime against its stored `indexed_at`. Bounded
/// to one `fs::metadata` call per enabled entry (microseconds each).
pub fn count_entries_by_kind(
    workspace: &WorkspaceName,
    paths: &Paths,
    conn: &rusqlite::Connection,
) -> Result<EntryCountsByKind, TomeError> {
    // R-M3 (US5.c): wrap both SELECT statements in a single read
    // transaction so the per-kind counts and the pending-re-embedding
    // walk see the same SQLite snapshot. Without this, a concurrent
    // writer between the two statements can produce skills+commands
    // disagreeing with pending_re_embedding's enumerated row set.
    // `open_read_only` produces a connection that can't write, so the
    // transaction is purely a snapshot boundary; rollback is implicit
    // when `tx` drops.
    let tx = conn.unchecked_transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "begin read transaction for entry counts: {e}"
        ))
    })?;

    // Per-kind counts via SQL aggregate. `kind` is the schema-v3 column,
    // widened in schema v4 to also admit `agent` (entry-schema-p6.md).
    let (skills, commands, agents): (u32, u32, u32) = {
        let mut stmt = tx
            .prepare(
                "SELECT s.kind, COUNT(*)
                 FROM skills AS s
                 JOIN workspace_skills AS ws ON ws.skill_id = s.id
                 JOIN workspaces       AS w  ON w.id = ws.workspace_id
                 WHERE w.name = ?1
                 GROUP BY s.kind",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("prepare entry_counts: {e}"))
            })?;
        let rows = stmt
            .query_map(rusqlite::params![workspace.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("query entry_counts: {e}"))
            })?;
        let mut skills = 0u32;
        let mut commands = 0u32;
        let mut agents = 0u32;
        for r in rows {
            let (kind_text, n) = r.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("collect entry_counts row: {e}"))
            })?;
            // R-m2 (US5.c): SQLite COUNT(*) is non-negative; the prior
            // `n.max(0)` defensive clamp is unreachable.
            let n_u32 = u32::try_from(n).unwrap_or(u32::MAX);
            // M-3 (Polish): canonical EntryKind dispatch over stringly-
            // typed match — surfaces schema drift as
            // IndexIntegrityCheckFailure rather than silently
            // undercounting via `_ => {}`. Matches the discipline at
            // src/index/skills.rs:189 / :753, src/index/query.rs:106,
            // src/mcp/prompts.rs:281.
            let kind = kind_text
                .parse::<crate::plugin::identity::EntryKind>()
                .map_err(|msg| {
                    TomeError::IndexIntegrityCheckFailure(format!(
                        "unknown kind `{kind_text}` in entry_counts: {msg}"
                    ))
                })?;
            match kind {
                crate::plugin::identity::EntryKind::Skill => skills = n_u32,
                crate::plugin::identity::EntryKind::Command => commands = n_u32,
                crate::plugin::identity::EntryKind::Agent => agents = n_u32,
            }
        }
        (skills, commands, agents)
    };

    // pending_re_embedding: for each enabled entry, compare source-file
    // mtime against indexed_at. Cap at u32::MAX (heuristic doesn't need
    // perfect precision for arbitrary-large workspaces).
    let mut pending: u32 = 0;
    {
        let mut stmt = tx
            .prepare(
                "SELECT s.catalog, s.plugin, s.path, s.indexed_at
                 FROM skills AS s
                 JOIN workspace_skills AS ws ON ws.skill_id = s.id
                 JOIN workspaces       AS w  ON w.id = ws.workspace_id
                 WHERE w.name = ?1",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("prepare pending_re_embedding: {e}"))
            })?;
        let rows = stmt
            .query_map(rusqlite::params![workspace.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("query pending_re_embedding: {e}"))
            })?;
        for r in rows {
            let (catalog, plugin, path, indexed_at) = r.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "collect pending_re_embedding row: {e}"
                ))
            })?;
            // Resolve absolute path on disk. Failures (catalog/plugin
            // not on disk, traversal refused) → skip; doctor doesn't
            // flag them here as pending — orphan-detection / other
            // checks cover that surface.
            let Ok(abs) = crate::index::skills::resolve_entry_body_path(
                conn,
                paths,
                workspace.as_str(),
                &catalog,
                &plugin,
                &path,
            ) else {
                continue;
            };
            let Ok(meta) = std::fs::metadata(&abs) else {
                continue;
            };
            let Ok(mtime) = meta.modified() else {
                continue;
            };
            // Parse `indexed_at` (RFC3339 → SystemTime). On parse
            // failure leave the entry uncounted — DB corruption surface
            // belongs to integrity_check, not this heuristic.
            let Ok(indexed_dt) = time::OffsetDateTime::parse(
                &indexed_at,
                &time::format_description::well_known::Rfc3339,
            ) else {
                continue;
            };
            let indexed_st = SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(indexed_dt.unix_timestamp().max(0) as u64);
            if mtime > indexed_st {
                pending = pending.saturating_add(1);
            }
        }
    }

    Ok(EntryCountsByKind {
        skills,
        commands,
        agents,
        pending_re_embedding: pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::workspace_catalogs;
    use crate::workspace::Scope;
    use tempfile::TempDir;

    fn fixture_paths(tmp: &Path) -> Paths {
        Paths::from_root(tmp.to_path_buf())
    }

    /// Seed one enrolment into the central DB for the privileged
    /// `global` workspace. Bootstrap happens via `index::open`; the
    /// per-test cache_path is the URL-hashed dir under `catalogs/` by
    /// construction of `paths.cache_dir_for(url)`.
    fn seed_enrolment(paths: &Paths, catalog: &str, url: &str) {
        let (e, r, s) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: e,
                reranker: r,
                summariser: s,
            },
        )
        .unwrap();
        workspace_catalogs::insert(&conn, "global", catalog, url, "main").unwrap();
    }

    #[test]
    fn check_catalogs_returns_empty_when_no_config() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn check_catalogs_reports_missing_for_absent_clone() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/missing";
        seed_enrolment(&paths, "lost", url);

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, CatalogCacheState::Missing);
        assert_eq!(out[0].name, "lost");
    }

    #[test]
    fn check_catalogs_reports_not_a_repo_for_dir_without_git() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/nogit";
        seed_enrolment(&paths, "nogit", url);
        std::fs::create_dir_all(paths.cache_dir_for(url)).unwrap();

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::NotARepo);
    }

    #[test]
    fn check_catalogs_reports_manifest_invalid_when_manifest_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/nomanifest";
        seed_enrolment(&paths, "nomanifest", url);
        std::fs::create_dir_all(paths.cache_dir_for(url).join(".git")).unwrap();

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::ManifestInvalid);
    }
}
