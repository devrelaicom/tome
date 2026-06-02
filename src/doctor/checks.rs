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
use crate::doctor::report::{
    CatalogCacheHealth, CatalogCacheState, EntryCountsByKind, OrphanDataDirReport, PromptsReport,
};
use crate::error::TomeError;
use crate::index::{self, workspace_catalogs};
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
        // FR-002 / F-DOCTOR-RW read-only contract: `tome doctor` (no `--fix`)
        // must NOT migrate an unlocked DB nor take the advisory lock during a
        // health check. `index::open` does both (runs `apply_pending` + would
        // be wrapped by callers under the lock for writes), so it cannot be
        // used here. `open_read_only` skips bootstrap + migration + the
        // lock, and refuses a future schema with `SchemaTooNew` (exit 52)
        // rather than the migrating `index::open`'s `SchemaVersionTooNew`
        // (exit 73). Swallow EITHER open/schema error into an empty
        // enrolment list so the read-only check degrades rather than aborts
        // on a stale OR a future schema — mirroring `mod.rs`'s `check_index`
        // (`unwrap_or_else`) and the Phase 5/6 `open_read_only` match arms.
        // `--fix`'s lock-held `repair_schema` is unaffected.
        match index::open_read_only(&paths.index_db) {
            Ok(conn) => {
                workspace_catalogs::list_for_workspace(&conn, workspace_name).unwrap_or_default()
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "doctor: open_read_only(index) failed during catalog check; degrading to empty enrolment list",
                );
                Vec::new()
            }
        }
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
    // `tome-catalog.toml` is third-party; cap the read at PLUGIN_MANIFEST_MAX
    // (FR-006, F-PLUGIN-MANIFEST-DOS). An over-cap file is `Err`, folding
    // into the same `ManifestInvalid` an unreadable manifest already yields
    // — doctor reports it as a problem, never silently OK after an
    // unbounded read.
    let Ok(bytes) = crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX)
    else {
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

// -------------------------------------------------------------------------
// Phase 6 / US5 — read-only doctor surfaces (hooks / guardrails / agents /
// privilege-escalation / personas). Per FR-124 every function below only
// reads: `fs::read` / `read_dir` / index queries. None create a directory
// nor invoke the substitution layer. Contract:
// `contracts/doctor-extensions-p6.md`.
// -------------------------------------------------------------------------

use crate::doctor::report::{
    AgentHarnessEntry, AgentsReport, CatalogPlugin, DroppedFieldEntry, GuardrailsFileEntry,
    GuardrailsReport, HookEventEntry, HookPluginEntry, HooksReport, PersonaEntry, PersonaReport,
    PrivilegeAgentEntry, PrivilegeEscalationReport, PrivilegePluginEntry,
};

/// Build the Phase 6 hooks surface (Claude Code only). For each enabled
/// plugin shipping a `hooks/hooks.json`, re-derive its rewritten entries and
/// compare them against the project's `.claude/settings.local.json` by deep
/// structural equality — the SAME ownership test the sync merge uses
/// (NFR-003). An entry found in the file is `contributed`; a re-derived
/// entry with no structural match is `missing` (drift from a user edit).
///
/// Read-only: `fs::read` of the source + the settings file only. The
/// `settings.local.json` is parsed but never written.
pub fn build_hooks_report(
    paths: &Paths,
    project_root: &Path,
    workspace: &WorkspaceName,
    conn: &rusqlite::Connection,
) -> Result<HooksReport, TomeError> {
    let settings_path = project_root.join(".claude").join("settings.local.json");
    // Parse the existing settings hooks object once (read-only). A missing
    // file means everything re-derived counts as `missing` (the merge would
    // create it on next sync).
    let existing_hooks = read_settings_hooks(&settings_path);

    let enabled = crate::index::skills::enabled_plugins_for_workspace(conn, workspace.as_str())?;
    let mut plugins: Vec<HookPluginEntry> = Vec::new();
    for (catalog, plugin) in &enabled {
        let plugin_root = match crate::index::skills::plugin_root_dir(
            conn,
            paths,
            workspace.as_str(),
            catalog,
            plugin,
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let plugin_data = paths.plugin_data_dir_for(catalog, plugin);
        let rewritten =
            match crate::harness::hooks::read_rewritten_entries(&plugin_root, &plugin_data) {
                Ok(Some(h)) if !h.is_empty() => h,
                // No hooks.json (or empty) → plugin contributes nothing. A
                // parse failure is surfaced by the sync path (exit 43); the
                // read-only doctor surface skips it rather than aborting.
                _ => continue,
            };

        let mut contributed: Vec<HookEventEntry> = Vec::new();
        let mut missing: Vec<HookEventEntry> = Vec::new();
        for (event, entries) in &rewritten.events {
            let present_in_file = existing_hooks
                .as_ref()
                .and_then(|h| h.get(event))
                .and_then(serde_json::Value::as_array);
            let mut found = 0usize;
            let mut gone = 0usize;
            for entry in entries {
                let matched = present_in_file
                    .map(|arr| arr.iter().any(|existing| existing == entry))
                    .unwrap_or(false);
                if matched {
                    found += 1;
                } else {
                    gone += 1;
                }
            }
            if found > 0 {
                contributed.push(HookEventEntry {
                    event: event.clone(),
                    count: found,
                });
            }
            if gone > 0 {
                missing.push(HookEventEntry {
                    event: event.clone(),
                    count: gone,
                });
            }
        }

        if !contributed.is_empty() || !missing.is_empty() {
            plugins.push(HookPluginEntry {
                catalog: catalog.clone(),
                plugin: plugin.clone(),
                contributed,
                missing,
            });
        }
    }

    Ok(HooksReport { plugins })
}

/// Read the `hooks` object out of a `settings.local.json`, returning `None`
/// when the file is absent / unparsable / lacks a `hooks` object. Read-only.
fn read_settings_hooks(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let body = crate::util::bounded_read_to_string(path, crate::util::HARNESS_MCP_MAX).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    value
        .as_object()
        .and_then(|o| o.get("hooks"))
        .and_then(serde_json::Value::as_object)
        .cloned()
}

/// Build the Phase 6 guardrails surface. For each effective harness's
/// guardrails target, parse the existing marker regions on disk
/// (`present`), classify any region whose plugin is no longer enabled as
/// `orphaned`, and record the steady-state set of plugins suppressed for
/// the Claude Code target because they ship real JSON hooks (`suppressed`,
/// FR-013). `suppressed` is derived from the enabled set (plugins shipping
/// BOTH `hooks/GUARDRAILS.md` AND `hooks/hooks.json`) independent of on-disk
/// region presence — the region is intentionally absent because the real
/// hooks supersede the prose fallback.
///
/// Read-only: `fs::read` of each target only; marker parsing never writes.
/// Shared targets (e.g. `AGENTS.md` across two harnesses) are reported once.
pub fn build_guardrails_report(
    paths: &Paths,
    project_root: &Path,
    workspace: &WorkspaceName,
    conn: &rusqlite::Connection,
) -> Result<GuardrailsReport, TomeError> {
    use std::collections::BTreeSet;

    // C5-2 / R5-2: `suppressed` is a STEADY-STATE audit, not a drift
    // artifact. For the Claude Code (`suppress_if_hooks_present`) target,
    // it lists enabled plugins that ship BOTH a `hooks/GUARDRAILS.md` prose
    // body AND a `hooks/hooks.json`: the real JSON hooks supersede the prose
    // fallback, so Tome intentionally does NOT render the plugin's region
    // into `CLAUDE.md` (FR-013). That region is therefore expected to be
    // ABSENT on disk — so `suppressed` must be derived from the enabled set
    // independent of on-disk region presence, not intersected with the
    // (necessarily empty) `present_keys`.
    let enabled = crate::index::skills::enabled_plugins_for_workspace(conn, workspace.as_str())?;
    let mut enabled_keys: BTreeSet<String> = BTreeSet::new();
    // Enabled plugins shipping BOTH GUARDRAILS.md + hooks.json → suppressed
    // for the Claude Code target. Preserve `(catalog, plugin)` enumeration
    // order so the emitted `suppressed` list is stable.
    let mut suppressed_candidates: Vec<CatalogPlugin> = Vec::new();
    for (catalog, plugin) in &enabled {
        enabled_keys.insert(crate::harness::guardrails::region_key(catalog, plugin));
        if let Ok(plugin_root) =
            crate::index::skills::plugin_root_dir(conn, paths, workspace.as_str(), catalog, plugin)
        {
            let hooks_dir = plugin_root.join("hooks");
            if hooks_dir.join("GUARDRAILS.md").exists() && hooks_dir.join("hooks.json").exists() {
                suppressed_candidates.push(CatalogPlugin {
                    catalog: catalog.clone(),
                    plugin: plugin.clone(),
                });
            }
        }
    }

    let mut files: Vec<GuardrailsFileEntry> = Vec::new();
    let mut seen_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    crate::harness::with_effective_modules(|mods| {
        for m in mods {
            let target = m.guardrails_target(project_root);
            let (file, suppress_flag) = match &target.placement {
                crate::harness::GuardrailsPlacement::InFileRegion { file }
                | crate::harness::GuardrailsPlacement::StandaloneSibling { file } => {
                    (file.clone(), target.suppress_if_hooks_present)
                }
            };
            if !seen_paths.insert(file.clone()) {
                continue;
            }

            // `present` / `orphaned` are the ON-DISK regions: regions Tome
            // wrote that are still parseable. `suppressed` is the
            // steady-state audit above, gated to the suppress target only
            // (Claude Code). A file with neither on-disk regions nor any
            // suppressed plugin contributes nothing.
            let present_keys = parse_guardrails_region_keys(&file);
            let suppressed: Vec<CatalogPlugin> = if suppress_flag {
                suppressed_candidates.clone()
            } else {
                Vec::new()
            };
            if present_keys.is_empty() && suppressed.is_empty() {
                continue;
            }

            let mut present: Vec<CatalogPlugin> = Vec::new();
            let mut orphaned: Vec<CatalogPlugin> = Vec::new();
            for key in &present_keys {
                let Some(cp) = split_region_key(key) else {
                    continue;
                };
                present.push(cp.clone());
                if !enabled_keys.contains(key) {
                    orphaned.push(cp);
                }
            }

            files.push(GuardrailsFileEntry {
                path: file,
                present,
                orphaned,
                suppressed,
            });
        }
    });

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(GuardrailsReport { files })
}

/// Parse the present guardrails region keys (`<catalog>:<plugin>`) from a
/// target file. Returns an empty vec when the file is absent / unparsable.
/// Read-only — bounded read + marker scan.
fn parse_guardrails_region_keys(path: &Path) -> Vec<String> {
    let body = match crate::util::bounded_read_to_string(path, crate::util::HARNESS_RULES_MAX) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    crate::harness::guardrails::present_region_keys(&body)
}

/// Split a `<catalog>:<plugin>` provenance key. The catalog half has no
/// colon (matching the guardrails marker grammar); everything after the
/// first colon is the plugin.
fn split_region_key(key: &str) -> Option<CatalogPlugin> {
    let (catalog, plugin) = key.split_once(':')?;
    if catalog.is_empty() || plugin.is_empty() {
        return None;
    }
    Some(CatalogPlugin {
        catalog: catalog.to_owned(),
        plugin: plugin.to_owned(),
    })
}

/// Build the Phase 6 agents surface. For each native-supporting harness,
/// enumerate the `<plugin>__*` files Tome owns in its `agent_dir`
/// (`present`), classify owned files whose plugin is no longer enabled as
/// `orphaned`, and re-translate each enabled agent to record dropped fields.
///
/// Read-only: `read_dir` of each agent dir + re-translation in memory (no
/// file is written). The clash set + canonical parses mirror the sync path.
pub fn build_agents_report(
    paths: &Paths,
    project_root: &Path,
    workspace: &WorkspaceName,
    conn: &rusqlite::Connection,
) -> Result<AgentsReport, TomeError> {
    use std::collections::HashSet;

    let clash_set = crate::index::skills::agent_name_clash_set(conn, workspace.as_str())?;
    let enabled = crate::index::skills::enabled_agents_for_workspace(conn, workspace.as_str())?;
    let enabled_plugins: HashSet<String> = enabled.iter().map(|a| a.plugin.clone()).collect();

    // Parse each enabled agent once into a CanonicalAgent + clash flag. A
    // parse failure (post-enable source corruption) is skipped here — the
    // sync path surfaces it as exit 45; the read-only doctor report omits
    // the unparsable agent rather than aborting.
    let mut prepared: Vec<(crate::harness::agents::CanonicalAgent, bool)> = Vec::new();
    for row in &enabled {
        let abs = match crate::index::skills::resolve_entry_body_path(
            conn,
            paths,
            workspace.as_str(),
            &row.catalog,
            &row.plugin,
            &row.path,
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let Ok(contents) =
            crate::util::bounded_read_to_string(&abs, crate::util::HARNESS_RULES_MAX)
        else {
            continue;
        };
        let stem = abs
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&row.name);
        let Ok(canonical) = crate::harness::agents::CanonicalAgent::parse(
            &row.catalog,
            &row.plugin,
            stem,
            &contents,
        ) else {
            continue;
        };
        let clashes = clash_set.contains(&canonical.name);
        prepared.push((canonical, clashes));
    }

    let mut harnesses: Vec<AgentHarnessEntry> = Vec::new();
    crate::harness::with_effective_modules(|mods| {
        for m in mods {
            if !m.supports_native_agents() {
                continue;
            }
            let Some(dir) = m.agent_dir(project_root) else {
                continue;
            };

            // Owned `<plugin>__*` files on disk.
            let owned = list_owned_agent_files(&dir);
            let mut present: Vec<String> = Vec::new();
            let mut orphaned: Vec<String> = Vec::new();
            for filename in &owned {
                present.push(filename.clone());
                if let Some(plugin) = crate::harness::agents::plugin_of_owned_file_pub(filename)
                    && !enabled_plugins.contains(plugin)
                {
                    orphaned.push(filename.clone());
                }
            }

            // Dropped fields from re-translation (informational).
            let mut dropped_fields: Vec<DroppedFieldEntry> = Vec::new();
            for (canonical, clashes) in &prepared {
                if let Ok(translated) = m.translate_agent(canonical, *clashes)
                    && !translated.dropped_fields.is_empty()
                {
                    let stem = translated
                        .filename
                        .rsplit_once('.')
                        .map(|(s, _)| s.to_owned())
                        .unwrap_or_else(|| translated.filename.clone());
                    dropped_fields.push(DroppedFieldEntry {
                        agent: stem,
                        fields: translated.dropped_fields.clone(),
                    });
                }
            }

            present.sort();
            orphaned.sort();
            dropped_fields.sort_by(|a, b| a.agent.cmp(&b.agent));
            harnesses.push(AgentHarnessEntry {
                harness: m.name().to_owned(),
                present,
                orphaned,
                dropped_fields,
            });
        }
    });

    harnesses.sort_by(|a, b| a.harness.cmp(&b.harness));
    Ok(AgentsReport { harnesses })
}

/// Enumerate the Tome-owned `<plugin>__*` agent filenames in `dir`. Skips
/// symlinks and non-files (S-M6 defence-in-depth). Read-only.
fn list_owned_agent_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for de in entries.flatten() {
        let p = de.path();
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if crate::harness::agents::plugin_of_owned_file_pub(name).is_some() {
            out.push(name.to_owned());
        }
    }
    out
}

/// Build the Phase 6 privilege-escalation surface (FR-051). Re-parse every
/// enabled agent's SOURCE `.md` and group those carrying any of `hooks` /
/// `mcpServers` / `permissionMode` by plugin. Read REGARDLESS of
/// `strip_plugin_agent_privileges` — the audit reads the source agent, never
/// the (possibly-stripped) emission clone, so the escalation surface stays
/// auditable.
///
/// Read-only: `fs::read` of each agent source only.
pub fn build_privilege_escalation_report(
    paths: &Paths,
    workspace: &WorkspaceName,
    conn: &rusqlite::Connection,
) -> Result<PrivilegeEscalationReport, TomeError> {
    let enabled = crate::index::skills::enabled_agents_for_workspace(conn, workspace.as_str())?;

    // Preserve enumeration order (catalog, plugin, name) while grouping by
    // (catalog, plugin). A `Vec` keyed scan keeps the wire order stable.
    let mut plugins: Vec<PrivilegePluginEntry> = Vec::new();
    for row in &enabled {
        let abs = match crate::index::skills::resolve_entry_body_path(
            conn,
            paths,
            workspace.as_str(),
            &row.catalog,
            &row.plugin,
            &row.path,
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let Ok(contents) =
            crate::util::bounded_read_to_string(&abs, crate::util::HARNESS_RULES_MAX)
        else {
            continue;
        };
        let stem = abs
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&row.name);
        let Ok(canonical) = crate::harness::agents::CanonicalAgent::parse(
            &row.catalog,
            &row.plugin,
            stem,
            &contents,
        ) else {
            continue;
        };

        let mut fields: Vec<String> = Vec::new();
        if canonical.hooks.is_some() {
            fields.push("hooks".to_owned());
        }
        if canonical.mcp_servers.is_some() {
            fields.push("mcpServers".to_owned());
        }
        if canonical.permission_mode.is_some() {
            fields.push("permissionMode".to_owned());
        }
        if fields.is_empty() {
            continue;
        }

        let agent = PrivilegeAgentEntry {
            name: canonical.name.clone(),
            fields,
        };
        match plugins
            .iter_mut()
            .find(|p| p.catalog == row.catalog && p.plugin == row.plugin)
        {
            Some(existing) => existing.agents.push(agent),
            None => plugins.push(PrivilegePluginEntry {
                catalog: row.catalog.clone(),
                plugin: row.plugin.clone(),
                agents: vec![agent],
            }),
        }
    }

    Ok(PrivilegeEscalationReport { plugins })
}

/// Build the Phase 6 personas surface. Enumerate one persona per enabled
/// agent with its resolved `<name>-persona` slug (or
/// `<plugin>-<name>-persona` for a clashing agent, FR-061), reusing the US4
/// clash set + name derivation. Read-only: persona names are derived from
/// frontmatter + entry rows WITHOUT invoking substitution or creating any
/// directory.
///
/// Only called when `expose_agents_as_personas` resolves true at the doctor
/// scope; the assembler maps a false flag to `None` on `DoctorReport`.
pub fn build_persona_report(
    workspace: &WorkspaceName,
    conn: &rusqlite::Connection,
) -> Result<PersonaReport, TomeError> {
    let clash_set = crate::index::skills::agent_name_clash_set(conn, workspace.as_str())?;
    let enabled = crate::index::skills::enabled_agents_for_workspace(conn, workspace.as_str())?;

    let mut personas: Vec<PersonaEntry> = Vec::new();
    for row in &enabled {
        let clash_prefixed = clash_set.contains(&row.name);
        // FR-061 name derivation, mirroring `prompts::collect_persona_identities`:
        // `<plugin>-<name>` base for a clash, `<name>` otherwise, then the
        // `-persona` suffix via the shared `derive_suffixed_name`.
        let base = if clash_prefixed {
            format!("{}-{}", row.plugin, row.name)
        } else {
            row.name.clone()
        };
        let resolved_persona_name = crate::mcp::prompt_name::derive_suffixed_name(&base, "persona");
        personas.push(PersonaEntry {
            catalog: row.catalog.clone(),
            plugin: row.plugin.clone(),
            agent_name: row.name.clone(),
            resolved_persona_name,
            clash_prefixed,
        });
    }

    Ok(PersonaReport {
        personas,
        drop_persona: crate::mcp::prompts::DROP_PERSONA_NAME.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // `seed_enrolment` below bootstraps a DB via the migrating `index::open`
    // (a legitimate test seeder, NOT the read-only contract surface), so it
    // needs the seeds + `OpenOptions` the production path no longer imports.
    use crate::commands::plugin::registry_seeds;
    use crate::index::{OpenOptions, workspace_catalogs};
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
