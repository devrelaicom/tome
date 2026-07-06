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
    UnrepresentedAgentEntry, UnrepresentedAgentsReport, UnrepresentedHookEntry,
    UnrepresentedHooksReport,
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
            // Compare at full nanosecond precision on BOTH sides. `indexed_at`
            // is stored at RFC3339 sub-second precision (`index::skills`) and
            // `mtime` is nanosecond; truncating indexed_at to whole seconds
            // (the previous `from_secs(unix_timestamp())`) false-positived
            // every entry whose source mtime landed in the SAME wall-clock
            // second as indexing — i.e. every freshly-enabled plugin on a fast
            // filesystem (surfaced as a deterministic ubuntu/ext4 CI failure of
            // `pending_re_embedding_zero_when_no_files_touched`, green on the
            // slower macOS runner only by timing luck).
            let indexed_st = SystemTime::UNIX_EPOCH
                + std::time::Duration::from_nanos(indexed_dt.unix_timestamp_nanos().max(0) as u64);
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

    let model_registry = crate::model_registry::ModelRegistry::load(paths);

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
                if let Ok(translated) = m.translate_agent(canonical, *clashes, &model_registry)
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

// ---------------------------------------------------------------------------
// Phase 12 / US4 — provider report (FR-018) + corrupt-index check (FR-017).
// ---------------------------------------------------------------------------

use crate::config::Config;
use crate::doctor::report::ProviderReport;
use crate::provider::config::{Capability, derive_env_var_name};

/// Build the read-only provider report (FR-018): one [`ProviderReport`] per
/// configured remote provider that a MODEL CAPABILITY references. A provider
/// defined in `[providers]` but referenced by no capability is omitted — the
/// report surfaces the providers Tome would actually use.
///
/// `credential_resolvable` is the SAME precedence the real path uses (env
/// `TOME_<NAME>_API_KEY` → inline `api_key`), but it NEVER exposes the value.
/// `reachable` is left `None` here; [`verify_provider_reachability`] fills it
/// under `--verify`.
///
/// Read-only: this only reads config + the process env. It never writes, never
/// opens the index, never makes a network call.
pub fn build_provider_report(cfg: &Config) -> Vec<ProviderReport> {
    use std::collections::BTreeMap;

    // Map provider-name → the set of capabilities referencing it. A capability
    // references a provider only via a *valid* `provider` field that names a
    // defined entry; a dangling reference is a resolve-time error surfaced
    // elsewhere (exit 93), not a provider row here.
    let mut by_provider: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    let mut record = |provider: Option<&str>, capability: &'static str| {
        if let Some(name) = provider
            && cfg.providers.contains_key(name)
        {
            by_provider
                .entry(name.to_string())
                .or_default()
                .push(capability);
        }
    };
    record(cfg.summariser.provider.as_deref(), "summariser");
    record(cfg.embedding.provider.as_deref(), "embedding");
    record(cfg.reranker.provider.as_deref(), "reranker");

    by_provider
        .into_iter()
        .map(|(name, mut capabilities)| {
            capabilities.sort_unstable();
            capabilities.dedup();
            // SAFETY of the unwrap: `record` only inserts names that
            // `contains_key`, so the entry is always present.
            let entry = cfg.providers.get(&name).expect("provider present");
            let credential_resolvable = credential_resolves(&name, entry.api_key.is_some());
            ProviderReport {
                name,
                kind: entry.kind.as_str().to_string(),
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                credential_resolvable,
                reachable: None,
            }
        })
        .collect()
}

/// Whether a credential resolves for a provider WITHOUT exposing it: the
/// derived env var `TOME_<NAME>_API_KEY` (set & non-empty) OR an inline
/// `api_key`. Mirrors `provider::config::resolve_credential`'s precedence so
/// doctor and the real path never disagree on "is a credential present".
fn credential_resolves(name: &str, has_inline: bool) -> bool {
    let env_var = derive_env_var_name(name);
    if let Ok(value) = std::env::var(&env_var)
        && !value.is_empty()
    {
        return true;
    }
    has_inline
}

/// One "provider configured but no credential resolved" finding (issue #291),
/// scoped to a single capability so its health severity is exact. Built from the
/// same [`ProviderReport`] rows the report renders, so the two never diverge.
///
/// `env_var` is the EXACT expected `TOME_<NAME>_API_KEY` derived via the shared
/// [`derive_env_var_name`] (never hardcoded). The credential VALUE is never
/// carried — this struct only records its *absence*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredentialFinding {
    /// The `[providers.<name>]` registry name.
    pub provider: String,
    /// The capability the provider serves (`embedding` / `summariser` /
    /// `reranker`) — drives the health severity.
    pub capability: String,
    /// The exact expected env var, e.g. `TOME_MY_PROV_API_KEY`.
    pub env_var: String,
    /// `true` when this capability is health-critical (embedding — search stops
    /// working → Unhealthy); `false` for the optional capabilities (summariser /
    /// reranker → Degraded).
    pub critical: bool,
}

/// Build the credential-missing findings (issue #291): for every configured
/// remote provider a MODEL CAPABILITY references whose credential does NOT
/// resolve, emit one finding per capability it serves, naming the exact expected
/// `TOME_<NAME>_API_KEY` env var.
///
/// A broken EMBEDDING provider is `critical` (search cannot run without a valid
/// embedder), so it drives the overall classification to Unhealthy; a broken
/// SUMMARISER / RERANKER provider is optional (the pipeline degrades but still
/// serves) → Degraded. This mirrors doctor's existing capability-health scheme
/// (embedder failure = Unhealthy, reranker/summariser = Degraded).
///
/// Read-only: reads config + the process env only. Reuses the same
/// [`build_provider_report`] rows the report renders, so the finding set and the
/// rendered `credential_resolvable` flag can never disagree. The findings are
/// sorted `(provider, capability)` for deterministic output.
pub fn build_provider_credential_findings(cfg: &Config) -> Vec<ProviderCredentialFinding> {
    let mut findings: Vec<ProviderCredentialFinding> = build_provider_report(cfg)
        .into_iter()
        .filter(|p| !p.credential_resolvable)
        .flat_map(|p| {
            let env_var = derive_env_var_name(&p.name);
            let name = p.name;
            p.capabilities.into_iter().map(move |capability| {
                let critical = capability == "embedding";
                ProviderCredentialFinding {
                    provider: name.clone(),
                    capability,
                    env_var: env_var.clone(),
                    critical,
                }
            })
        })
        .collect();
    findings.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.capability.cmp(&b.capability))
    });
    findings
}

/// `--verify` reachability (FR-018): for each provider in `report`, perform ONE
/// lightweight real round-trip against a capability it serves and set
/// `reachable = Some(ok)`. Read-only — the round-trips never persist anything (a
/// remote embedder establishes its dimension in memory; the reranker is
/// stateless; the summariser writes nothing). A round-trip failure sets
/// `reachable = Some(false)` (NOT an error — doctor never crashes on a probe).
///
/// The capability chosen per provider is the FIRST it serves, in the fixed
/// order summariser → embedding → reranker, so the probe is deterministic.
///
/// Issue #291: a missing credential is detectable statically, so `--verify`
/// SKIPS the live probe for any provider whose credential does not resolve —
/// leaving `reachable = None` — rather than making a doomed network call that
/// would 401/403. The distinct credential SuggestedFix (see
/// [`build_provider_credential_findings`] + the mod-level applier) is the clear
/// finding surfaced in its place; a doomed `reachable: false` would only be
/// noise.
pub fn verify_provider_reachability(report: &mut [ProviderReport], cfg: &Config, paths: &Paths) {
    for entry in report.iter_mut() {
        // No credential resolves → skip the doomed live probe (issue #291). The
        // credential finding + report line name the exact env var; a network
        // round-trip here would only 401.
        if !entry.credential_resolvable {
            continue;
        }
        // Pick the first capability the provider serves (deterministic order).
        let capability = entry.capabilities.iter().find_map(|c| match c.as_str() {
            "summariser" => Some(Capability::Summariser),
            "embedding" => Some(Capability::Embedding),
            "reranker" => Some(Capability::Reranker),
            _ => None,
        });
        let Some(capability) = capability else {
            continue;
        };
        entry.reachable = Some(probe_capability(cfg, paths, capability).is_ok());
    }
}

/// ONE lightweight real round-trip for a capability, used by the `--verify`
/// provider probe. Builds the active model via the shared `build_*` helpers and
/// performs a single embed / summarise / rerank. Read-only by construction.
fn probe_capability(cfg: &Config, paths: &Paths, capability: Capability) -> Result<(), TomeError> {
    match capability {
        Capability::Embedding => {
            // Read the persisted dimension read-only so the probe validates
            // against the index's expected length; absent ⇒ establish in-memory.
            let (active, persisted) = if paths.index_db.is_file() {
                let conn = index::open_read_only(&paths.index_db)?;
                (
                    crate::index::meta::active_embedder(&conn)?,
                    crate::index::meta::read_embedder_dimension(&conn)?,
                )
            } else {
                (
                    crate::embedding::profile::embedder_for(crate::embedding::Profile::DEFAULT),
                    None,
                )
            };
            let embedder = crate::embedding::build_embedder(cfg, paths, active, persisted)?;
            let v = embedder.embed("connectivity check")?;
            if v.is_empty() {
                return Err(TomeError::RemoteEmbeddingInvalid {
                    detail: "provider verify: empty embedding".to_string(),
                });
            }
            Ok(())
        }
        Capability::Summariser => {
            let summariser = crate::summarise::build_summariser(cfg, paths, false)?;
            let input = crate::summarise::PluginSummariesInput {
                plugins: vec![crate::summarise::PluginSummaryItem {
                    catalog: "test".to_string(),
                    plugin: "connectivity".to_string(),
                    description: "doctor --verify connectivity probe".to_string(),
                    skills: vec![crate::summarise::SkillSummaryItem {
                        name: "ping".to_string(),
                        description: "verify the summariser is reachable".to_string(),
                    }],
                }],
            };
            let out = summariser.summarise(&input, crate::summarise::LONG_MAX_CHARS)?;
            if out.short.trim().is_empty() || out.long.trim().is_empty() {
                return Err(TomeError::SummariserFailure {
                    kind: crate::error::SummariserFailureKind::OutputEmpty {
                        which: crate::error::ShortOrLong::Short,
                    },
                });
            }
            Ok(())
        }
        Capability::Reranker => {
            // Resolve the ACTIVE profile's reranker (mirroring the embedding
            // probe + `models test`'s `active_reranker_entry`) so the probe
            // targets the in-use bundled model on a non-default profile rather
            // than always `Profile::DEFAULT`. Harmless for a remote Voyage
            // reranker (the registry entry is ignored once a provider resolves),
            // but correct for parity.
            let active = if paths.index_db.is_file() {
                let conn = index::open_read_only(&paths.index_db)?;
                crate::index::meta::active_reranker(&conn)?
            } else {
                crate::embedding::profile::reranker_for(crate::embedding::Profile::DEFAULT)
            };
            let reranker = crate::embedding::build_reranker(cfg, paths, active)?;
            let candidates: Vec<crate::index::query::Candidate> = ["alpha", "bravo"]
                .iter()
                .enumerate()
                .map(|(i, name)| crate::index::query::Candidate {
                    skill_id: i as i64,
                    catalog: "test".to_string(),
                    plugin: "connectivity".to_string(),
                    name: (*name).to_string(),
                    kind: crate::plugin::identity::EntryKind::Skill,
                    description: format!("probe candidate {name}"),
                    plugin_version: "0.0.0".to_string(),
                    path: format!("/dev/null/{name}"),
                    distance: 0.0,
                })
                .collect();
            let scored = reranker.rerank("test query", candidates)?;
            if scored.is_empty() {
                return Err(TomeError::RerankingFailure(
                    "provider verify: reranker returned no scored candidates".to_string(),
                ));
            }
            Ok(())
        }
        // US6.2 — HookPrompt is a runtime chat capability for the hook dispatcher;
        // the doctor probe for chat capabilities is not yet implemented (US11).
        // The doctor's capability string parser ("summariser"/"embedding"/"reranker")
        // never produces this variant, so this arm is structural exhaustiveness only.
        Capability::HookPrompt => Ok(()),
    }
}

/// Corrupt-remote-index verdict (FR-017): a stored-vector dimension that
/// disagrees with the persisted `meta.embedder_dimension`.
///
/// `None` means "not applicable / no mismatch":
/// - no index DB on disk, or it is unreadable (the index/embedder subsystem
///   checks classify the underlying failure);
/// - `meta.embedder_dimension` is absent (bundled / never-remote-reindexed —
///   the dimension-free storage is fine);
/// - no stored vectors yet (nothing to compare);
/// - the stored dimension matches the meta dimension.
///
/// `Some(CorruptIndex { stored, expected })` means a sampled `skill_embeddings`
/// BLOB is `stored` f32s long while `meta.embedder_dimension` says `expected`.
/// This is the silent-mis-index the pre-mortem targets: a remote model whose
/// output length changed since the last reindex.
///
/// Read-only: opens the index read-only, reads one BLOB length + one meta row.
pub fn check_corrupt_index(paths: &Paths) -> Option<CorruptIndex> {
    if !paths.index_db.is_file() {
        return None;
    }
    let conn = match index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "doctor: corrupt-index check open failed; skipping");
            return None;
        }
    };
    // Only applicable when a remote reindex persisted an expected dimension.
    let expected = match crate::index::meta::read_embedder_dimension(&conn) {
        Ok(Some(d)) => d,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "doctor: read meta.embedder_dimension failed; skipping");
            return None;
        }
    };
    // Sample one stored vector's byte length. `LENGTH(BLOB)` is the byte count;
    // an f32 is 4 bytes, so `stored_dim = blob_len / 4`. No rows ⇒ nothing to
    // compare.
    let blob_len: i64 = conn
        .query_row(
            "SELECT LENGTH(embedding) FROM skill_embeddings LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok()?;
    if blob_len < 0 {
        return None;
    }
    let stored = (blob_len as usize) / 4;
    if stored == expected {
        None
    } else {
        Some(CorruptIndex { stored, expected })
    }
}

// ---------------------------------------------------------------------------
// Phase 2 (native-agent expansion): unrepresented agents drop-report.
// ---------------------------------------------------------------------------

/// Build the Phase 2 drop-report: every enabled agent paired with the
/// rules-only harnesses that cannot represent it natively. Read-only.
///
/// A rules-only harness is one where `supports_native_agents()` is `false`
/// AND `is_opt_in_target()` is also `false`. The returned list is sorted
/// alphabetically. The returned `agents` list preserves the DB enumeration
/// order (catalog, plugin, name).
pub fn build_unrepresented_agents_report(
    conn: &rusqlite::Connection,
    workspace: &crate::workspace::WorkspaceName,
) -> Result<UnrepresentedAgentsReport, TomeError> {
    let enabled = crate::index::skills::enabled_agents_for_workspace(conn, workspace.as_str())?;
    let agents: Vec<UnrepresentedAgentEntry> = enabled
        .iter()
        .map(|a| UnrepresentedAgentEntry {
            catalog: a.catalog.clone(),
            plugin: a.plugin.clone(),
            name: a.name.clone(),
        })
        .collect();

    let mut rules_only_harnesses: Vec<String> = Vec::new();
    crate::harness::with_effective_modules(|mods| {
        for m in mods {
            if !m.supports_native_agents() && !m.is_opt_in_target() {
                rules_only_harnesses.push(m.name().to_string());
            }
        }
    });
    rules_only_harnesses.sort();

    Ok(UnrepresentedAgentsReport {
        rules_only_harnesses,
        agents,
    })
}

// ---------------------------------------------------------------------------
// Issue #292 (translation-fidelity loss): unrepresented hooks drop-report.
// ---------------------------------------------------------------------------

/// Build the issue #292 drop-report: enabled plugin hooks (by portable event)
/// that no rules-only-for-hooks harness in scope can deliver natively. Read-only
/// (FR-124): never writes, never creates directories.
///
/// The hooks analogue of [`build_unrepresented_agents_report`]. It mirrors that
/// function's shape exactly:
///
/// * `rules_only_harnesses` — the in-scope harnesses that render `GUARDRAILS.md`
///   prose only, resolved through the SSOT predicate
///   [`crate::harness::HarnessModule::is_rules_only_for_hooks`] (Claude Code's
///   `RealJson` sink and the five `#318` dispatcher harnesses are excluded — they
///   DO translate hooks). Resolved through `with_effective_modules` so the
///   `HARNESS_MODULES_OVERRIDE` test seam is honoured — identical to the
///   mechanism `build_hook_translation_report` and `status` use.
///
/// * `hooks` — the distinct `(catalog, plugin, event)` tuples enumerated by the
///   dispatch SSOT [`crate::harness::reconcile::hooks::resolve_enabled_canonical_hooks`]
///   (the SAME enumeration `sync` and `preview` consume), so the report never
///   re-implements the native-vs-guardrails decision. On a rules-only-for-hooks
///   harness EVERY declared portable event is unrepresented, so the distinct
///   canonical events ARE the unrepresented set.
///
/// `hooks` is empty (and the caller emits `None`) when no rules-only-for-hooks
/// harness is in scope OR no enabled plugin ships hooks — keeping the byte-stable
/// wire shape minimal (like the agents report).
///
/// `effective` is the scope's resolved harness list (the same
/// [`EffectiveHarnessList`](crate::settings::resolver::EffectiveHarnessList)
/// `harness_mcp`, `status`, and `build_hook_translation_report` use). When it is
/// `None`, no harness is in scope → an empty report.
pub fn build_unrepresented_hooks_report(
    paths: &Paths,
    workspace: &WorkspaceName,
    home: &Path,
    effective: Option<&crate::settings::resolver::EffectiveHarnessList>,
) -> Result<UnrepresentedHooksReport, TomeError> {
    use crate::harness::sync::SyncDeps;

    // Which of the in-scope harnesses are rules-only for hooks. Resolved via
    // `with_effective_modules` (the same override-honouring path status +
    // doctor's hook-translation surface use), intersected with the effective
    // (scope) set so a harness a user never declared adds no noise.
    let effective_names: HashSet<&str> = effective
        .map(|e| e.harnesses.iter().map(|h| h.name.as_str()).collect())
        .unwrap_or_default();

    let mut rules_only_harnesses: Vec<String> = Vec::new();
    crate::harness::with_effective_modules(|mods| {
        for m in mods {
            if effective_names.contains(m.name()) && m.is_rules_only_for_hooks() {
                rules_only_harnesses.push(m.name().to_string());
            }
        }
    });
    rules_only_harnesses.sort();
    rules_only_harnesses.dedup();

    // No rules-only-for-hooks harness in scope → nothing is unrepresented.
    if rules_only_harnesses.is_empty() {
        return Ok(UnrepresentedHooksReport {
            rules_only_harnesses,
            hooks: Vec::new(),
        });
    }

    // Enumerate the enabled plugins' canonical hooks via the dispatch SSOT.
    // `resolve_enabled_canonical_hooks` opens the DB read-only itself and
    // records (but never propagates) a per-plugin parse error via `first_error`.
    // A malformed `hooks/hooks.json` is skipped (forward progress) — the report
    // stays honest about what IS enumerable, matching sync/preview.
    let deps = SyncDeps {
        paths,
        home_root: home,
        workspace_name: workspace,
        force: false,
        only_harness: None,
        dry_run: false,
    };
    let mut first_error: Option<TomeError> = None;
    let canonical =
        crate::harness::reconcile::hooks::resolve_enabled_canonical_hooks(&deps, &mut first_error)?;

    // Distinct (catalog, plugin, event) tuples, deterministically ordered.
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut hooks: Vec<UnrepresentedHookEntry> = Vec::new();
    for h in &canonical {
        let event = h.event.cc_name().to_string();
        let key = (h.catalog.clone(), h.plugin.clone(), event.clone());
        if seen.insert(key) {
            hooks.push(UnrepresentedHookEntry {
                catalog: h.catalog.clone(),
                plugin: h.plugin.clone(),
                event,
            });
        }
    }
    hooks.sort_by(|a, b| {
        (a.catalog.as_str(), a.plugin.as_str(), a.event.as_str()).cmp(&(
            b.catalog.as_str(),
            b.plugin.as_str(),
            b.event.as_str(),
        ))
    });

    Ok(UnrepresentedHooksReport {
        rules_only_harnesses,
        hooks,
    })
}

/// A corrupt-remote-index finding: the stored vector dimension (`blob_len/4`)
/// disagrees with `meta.embedder_dimension`. Carried internally by the
/// assembler to drive the `corrupt-remote-index` suggested fix; not a
/// `DoctorReport` field (the suggested-fix list IS the surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorruptIndex {
    /// The stored vectors' dimension (`LENGTH(embedding) / 4`).
    pub stored: usize,
    /// The dimension `meta.embedder_dimension` says they should be.
    pub expected: usize,
}

// ---------------------------------------------------------------------------
// Phase 13 (native-agent model-registry): read-only registry report.
// ---------------------------------------------------------------------------

use crate::doctor::report::ModelRegistryReport;

/// Build the read-only model-registry subsystem report for `tome doctor`.
///
/// Calls [`crate::model_registry::ModelRegistry::load`] (which prefers a
/// valid override over the baked asset) to determine the active source,
/// then [`crate::model_registry::override_health`] (independently, so a
/// corrupt override is surfaced even though `load` already fell back to
/// baked). Read-only — no fetch, no writes.
pub fn check_model_registry(paths: &Paths) -> ModelRegistryReport {
    let active = crate::model_registry::ModelRegistry::load(paths);
    let info = active.info();
    let override_corrupt = matches!(
        crate::model_registry::override_health(paths),
        crate::model_registry::OverrideHealth::Corrupt
    );
    ModelRegistryReport {
        source: match info.source {
            crate::model_registry::RegistrySource::Baked => "baked".to_owned(),
            crate::model_registry::RegistrySource::Override => "override".to_owned(),
        },
        fetched_at: info.fetched_at,
        model_count: info.model_count,
        override_corrupt,
    }
}

// ---------------------------------------------------------------------------
// US11 (native plugin-hook translation): read-only hook-translation surface.
// ---------------------------------------------------------------------------

/// Build the US11 plugin-hook translation surface: per-harness dispatch state
/// derived from on-disk manifests + config. Read-only (FR-124): never writes,
/// never creates directories. Missing or unreadable manifests are treated as
/// "no registered events" — not an error (unsynced or no hooks yet).
///
/// `effective` is the scope's resolved harness list (same
/// `EffectiveHarnessList` that `harness_mcp` and `status` use). A harness
/// row is emitted iff:
///
/// * **(a) Registered case** — the harness is in `effective` AND
///   `hook_support().is_some()`: the translation is active (or would be if
///   `translate_plugin_hooks` is on).
/// * **(b) Stale/drift case** — the harness has a dispatch manifest file on
///   disk even though it is NOT in `effective` (or was globally disabled): the
///   `manifest_stale` signal tells operators a `tome sync` is needed to remove
///   the orphaned manifest.
///
/// Harnesses that are neither in scope nor have an on-disk manifest are
/// silently omitted — this removes the noise of the other hook-capable modules
/// that a user never declared. Returns an empty `per_harness` when no harness
/// qualifies under either rule.
///
/// `hook_support()` is resolved through `with_effective_modules` (honours the
/// `HARNESS_MODULES_OVERRIDE` test seam) so this surface stays consistent with
/// `status`'s count (which uses the same resolution path after the P11 fix).
pub fn build_hook_translation_report(
    paths: &Paths,
    workspace: &crate::workspace::WorkspaceName,
    cfg: &crate::config::Config,
    effective: Option<&crate::settings::resolver::EffectiveHarnessList>,
) -> crate::doctor::report::HookTranslationReport {
    use crate::doctor::report::{HookHarnessStatus, HookTranslationReport};
    use crate::harness::hooks_ir::{PortableEvent, read_manifest};

    let cfg_enabled = cfg.hooks.translate_plugin_hooks.unwrap_or(true);
    let has_prompt_settings =
        cfg.hooks.prompt_provider.is_some() || cfg.hooks.prompt_model.is_some();

    // Build a lookup set of the scope-effective harness names so the inner
    // loop doesn't iterate `effective` twice per module.
    let effective_names: std::collections::HashSet<&str> = effective
        .map(|e| e.harnesses.iter().map(|h| h.name.as_str()).collect())
        .unwrap_or_default();

    let mut per_harness: Vec<HookHarnessStatus> = Vec::new();

    // Resolve `hook_support()` through `with_effective_modules` so the
    // `HARNESS_MODULES_OVERRIDE` test seam is honoured — identical to the
    // mechanism `status` uses after the P11 fix.
    crate::harness::with_effective_modules(|mods| {
        for m in mods {
            let Some(hs) = m.hook_support() else {
                continue;
            };

            let in_effective = effective_names.contains(m.name());
            let manifest_path = paths.hooks_manifest(workspace, m.name());
            let manifest_exists = manifest_path.exists();

            // Row inclusion rule (see doc comment above):
            // (a) in scope AND hook-capable → registered row.
            // (b) not in scope but manifest on disk → stale/drift row.
            // Anything else → skip (no noise for never-touched registry modules).
            if !in_effective && !manifest_exists {
                continue;
            }

            let manifest = read_manifest(&manifest_path).ok();

            // Events present in the on-disk dispatch manifest (successfully translated).
            let registered_events: Vec<String> = manifest
                .as_ref()
                .map(|mf| mf.events.keys().cloned().collect())
                .unwrap_or_default();

            // Events the harness CANNOT translate → go to GUARDRAILS.
            // These are portable events NOT in the harness's supported-event list.
            let dropped_to_guardrails: Vec<String> = PortableEvent::ALL
                .iter()
                .filter(|e| !hs.events.contains(e))
                .map(|e| e.cc_name().to_string())
                .collect();

            // enabled: config allows translation AND this harness is in scope.
            // A stale-row harness (b) reports `enabled = false` so consumers can
            // distinguish it from an active registered row.
            let scope_enabled = cfg_enabled && in_effective;

            // Stale: manifest on disk but the harness is not scope-enabled
            // (either the harness was removed from the effective list, or
            // `translate_plugin_hooks` was turned off). Needs a `tome sync`.
            let manifest_stale = !scope_enabled && manifest_exists;

            per_harness.push(HookHarnessStatus {
                harness: m.name().to_string(),
                enabled: scope_enabled,
                registered_events,
                dropped_to_guardrails,
                manifest_stale,
                trust_prompt_note: has_prompt_settings,
            });
        }
    });

    HookTranslationReport { per_harness }
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
                profile: None,
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

    // --- Phase 2 (native-agent expansion): unrepresented agents report ------

    #[test]
    fn unrepresented_report_lists_enabled_agents_and_rules_only_harnesses() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let workspace = crate::workspace::WorkspaceName::global();

        // Seed one enabled agent in the global workspace.
        let (e, r, s) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &crate::index::OpenOptions {
                embedder: e,
                reranker: r,
                summariser: s,
                profile: None,
            },
        )
        .unwrap();
        // Insert the agent row and enrol it.
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES ('mycat', 'myplugin', 'myagent', 'agent', 'd', '0.0.0', 'agents/myagent.md', 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
            [],
        )
        .expect("insert agent");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog='mycat' AND plugin='myplugin' AND kind='agent' AND name='myagent'",
                [],
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
            rusqlite::params![ws_id, skill_id],
        )
        .expect("enrol agent");

        let report = build_unrepresented_agents_report(&conn, &workspace)
            .expect("build_unrepresented_agents_report");

        // The one enabled agent must appear.
        assert_eq!(report.agents.len(), 1);
        assert_eq!(report.agents[0].catalog, "mycat");
        assert_eq!(report.agents[0].plugin, "myplugin");
        assert_eq!(report.agents[0].name, "myagent");

        // The rules-only harnesses from the default registry must include
        // "cline" and "junie" (and others), sorted alphabetically.
        assert!(
            report.rules_only_harnesses.contains(&"cline".to_owned()),
            "cline must be in rules_only_harnesses: {:?}",
            report.rules_only_harnesses,
        );
        assert!(
            report.rules_only_harnesses.contains(&"junie".to_owned()),
            "junie must be in rules_only_harnesses: {:?}",
            report.rules_only_harnesses,
        );
        // The list must be sorted.
        let mut sorted = report.rules_only_harnesses.clone();
        sorted.sort();
        assert_eq!(
            report.rules_only_harnesses, sorted,
            "rules_only_harnesses must be sorted"
        );
    }

    // --- Issue #292: unrepresented hooks report ----------------------------

    /// A hook-capable harness (`codex`, which declares `hook_support()`) and
    /// Claude Code (the `RealJson` sink) are NOT rules-only for hooks → they are
    /// excluded from `rules_only_harnesses`, so a scope of only those harnesses
    /// yields an empty report even with hooks enabled.
    #[test]
    fn unrepresented_hooks_excludes_hook_capable_and_realjson_harnesses() {
        use crate::settings::resolver::{EffectiveHarness, EffectiveHarnessList};

        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let workspace = crate::workspace::WorkspaceName::parse("test-ws").unwrap();

        // Scope: codex (has hook_support) + claude-code (RealJson sink). Neither
        // is rules-only for hooks.
        let effective = EffectiveHarnessList {
            harnesses: vec![
                EffectiveHarness {
                    name: "codex".to_owned(),
                    source_chain: vec!["project".to_owned()],
                },
                EffectiveHarness {
                    name: "claude-code".to_owned(),
                    source_chain: vec!["project".to_owned()],
                },
            ],
            excluded: vec![],
        };

        let report =
            build_unrepresented_hooks_report(&paths, &workspace, tmp.path(), Some(&effective))
                .expect("build_unrepresented_hooks_report");

        assert!(
            report.rules_only_harnesses.is_empty(),
            "codex (hook_support) + claude-code (RealJson) must not be rules-only for hooks: {:?}",
            report.rules_only_harnesses,
        );
        assert!(
            report.hooks.is_empty(),
            "no rules-only-for-hooks harness in scope → no unrepresented hooks",
        );
    }

    /// A rules-only-for-hooks harness (`cline`: no `RealJson`, no
    /// `hook_support()`, not opt-in) in scope with an enabled plugin that ships
    /// `hooks/hooks.json` → the declared events ARE reported unrepresented. Uses
    /// the SAME enumeration SSOT (`resolve_enabled_canonical_hooks`) sync/preview
    /// use, so the report reflects what sync actually delivers.
    #[test]
    fn unrepresented_hooks_reports_events_on_rules_only_harness() {
        use crate::settings::resolver::{EffectiveHarness, EffectiveHarnessList};

        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let workspace = crate::workspace::WorkspaceName::global();

        // Enrol a catalog + enable one plugin, and seed its hooks/hooks.json on
        // disk (no catalog manifest → plugin_root_dir falls back to
        // <cache>/<plugin>, matching the source seeder below).
        let url = "https://example.invalid/hookplug";
        seed_enrolment(&paths, "hookcat", url);
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: registry_seeds().0,
                reranker: registry_seeds().1,
                summariser: registry_seeds().2,
                profile: None,
            },
        )
        .unwrap();
        // Enable a skill row so the plugin is "enabled" for the workspace.
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES ('hookcat', 'hookplug', 's1', 'skill', 'd', '0.0.0', 'skills/s1/SKILL.md', 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
            [],
        )
        .expect("insert skill");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog='hookcat' AND plugin='hookplug' AND name='s1'",
                [],
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
            rusqlite::params![ws_id, skill_id],
        )
        .expect("enrol skill");

        // Seed the on-disk hooks source (PreToolUse + Stop).
        let hooks_dir = paths.cache_dir_for(url).join("hookplug").join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("mk hooks dir");
        std::fs::write(
            hooks_dir.join("hooks.json"),
            r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/g.sh" } ] } ], "Stop": [ { "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/s.sh" } ] } ] }"#,
        )
        .expect("write hooks.json");

        let effective = EffectiveHarnessList {
            harnesses: vec![EffectiveHarness {
                name: "cline".to_owned(),
                source_chain: vec!["project".to_owned()],
            }],
            excluded: vec![],
        };

        let report =
            build_unrepresented_hooks_report(&paths, &workspace, tmp.path(), Some(&effective))
                .expect("build_unrepresented_hooks_report");

        // Tolerant `contains` (matching the sibling `unrepresented_agents` +
        // `hook_translation` tests): the harness set is resolved through
        // `with_effective_modules`, which reads the process-global
        // `HARNESS_MODULES_OVERRIDE`. The `agents.rs` lib tests in this same
        // binary install that override without a shared serialization mutex (a
        // pre-existing latent race, out of scope for #292), so an exact-equality
        // on the list would narrow the safe window. `cline` must be present.
        assert!(
            report.rules_only_harnesses.contains(&"cline".to_owned()),
            "cline is rules-only for hooks and in scope: {:?}",
            report.rules_only_harnesses,
        );
        // Both declared events must appear, distinct + sorted by (cat, plug, event).
        let events: Vec<&str> = report.hooks.iter().map(|h| h.event.as_str()).collect();
        assert_eq!(
            events,
            vec!["PreToolUse", "Stop"],
            "both declared events must be reported unrepresented (sorted): {:?}",
            report.hooks,
        );
        assert!(
            report
                .hooks
                .iter()
                .all(|h| h.catalog == "hookcat" && h.plugin == "hookplug"),
            "provenance must be carried: {:?}",
            report.hooks,
        );
    }

    /// No rules-only-for-hooks harness in scope (empty effective) → empty report,
    /// even if plugins ship hooks. Mirrors the agents report's minimal-shape gate.
    #[test]
    fn unrepresented_hooks_empty_when_no_effective_harness() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let workspace = crate::workspace::WorkspaceName::global();

        let report = build_unrepresented_hooks_report(&paths, &workspace, tmp.path(), None)
            .expect("build_unrepresented_hooks_report");
        assert!(report.rules_only_harnesses.is_empty());
        assert!(report.hooks.is_empty());
    }

    // --- Phase 12 / US4: provider report (FR-018) --------------------------

    use crate::config::{Config, ProviderEntry, ProviderKind, Secret};
    use std::sync::Mutex;

    /// Serialises tests mutating `TOME_<NAME>_API_KEY` (process-global env).
    static PROVIDER_ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn config_with_provider(name: &str, kind: ProviderKind, inline_key: Option<&str>) -> Config {
        let mut cfg = Config::default();
        cfg.providers.insert(
            name.to_string(),
            ProviderEntry {
                kind,
                base_url: None,
                api_key: inline_key.map(|k| Secret::from(k.to_string())),
            },
        );
        cfg
    }

    #[test]
    fn provider_report_omits_unreferenced_providers() {
        // A provider defined but referenced by no capability is omitted.
        let cfg = config_with_provider("p", ProviderKind::Openai, Some("sk"));
        let report = build_provider_report(&cfg);
        assert!(
            report.is_empty(),
            "an unreferenced provider must not appear in the report: {report:?}"
        );
    }

    #[test]
    fn provider_report_surfaces_referencing_capabilities_and_credential() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure no env override interferes.
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        let mut cfg = config_with_provider("p", ProviderKind::Openai, Some("sk-inline"));
        cfg.summariser.provider = Some("p".to_string());
        cfg.summariser.model = Some("gpt-4o".to_string());
        cfg.embedding.provider = Some("p".to_string());
        cfg.embedding.model = Some("text-embed".to_string());

        let report = build_provider_report(&cfg);
        assert_eq!(report.len(), 1);
        let p = &report[0];
        assert_eq!(p.name, "p");
        assert_eq!(p.kind, "openai");
        // Both capabilities reference it; sorted + deduped.
        assert_eq!(p.capabilities, vec!["embedding", "summariser"]);
        assert!(p.credential_resolvable, "inline key resolves");
        assert_eq!(p.reachable, None, "reachable is None without --verify");
    }

    #[test]
    fn provider_report_credential_not_resolvable_without_env_or_inline() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_VP_API_KEY");
        }
        let mut cfg = config_with_provider("vp", ProviderKind::Voyage, None);
        cfg.reranker.provider = Some("vp".to_string());
        cfg.reranker.model = Some("rerank-2".to_string());

        let report = build_provider_report(&cfg);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "voyage");
        assert!(
            !report[0].credential_resolvable,
            "no env + no inline → not resolvable"
        );
    }

    #[test]
    fn provider_report_env_var_resolves_credential() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::set_var("TOME_VP_API_KEY", "env-secret");
        }
        let mut cfg = config_with_provider("vp", ProviderKind::Voyage, None);
        cfg.reranker.provider = Some("vp".to_string());
        cfg.reranker.model = Some("rerank-2".to_string());

        let report = build_provider_report(&cfg);
        assert!(
            report[0].credential_resolvable,
            "env override resolves the credential"
        );
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_VP_API_KEY");
        }
    }

    // --- Issue #291: provider credential findings --------------------------

    #[test]
    fn credential_findings_empty_when_no_providers() {
        // A clean install with no `[providers]` → no findings (byte-stable pin).
        let cfg = Config::default();
        assert!(build_provider_credential_findings(&cfg).is_empty());
    }

    #[test]
    fn credential_findings_empty_when_credential_resolves_inline() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        let mut cfg = config_with_provider("p", ProviderKind::Openai, Some("sk-inline"));
        cfg.embedding.provider = Some("p".to_string());
        cfg.embedding.model = Some("text-embed".to_string());

        assert!(
            build_provider_credential_findings(&cfg).is_empty(),
            "an inline api_key resolves → no finding"
        );
    }

    #[test]
    fn credential_finding_names_exact_env_var_and_marks_embedding_critical() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_MY_PROV_API_KEY");
        }
        // Provider named `my-prov` derives `TOME_MY_PROV_API_KEY` — no inline key
        // and env unset → the finding must name that exact var.
        let mut cfg = config_with_provider("my-prov", ProviderKind::Openai, None);
        cfg.embedding.provider = Some("my-prov".to_string());
        cfg.embedding.model = Some("text-embed".to_string());

        let findings = build_provider_credential_findings(&cfg);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.provider, "my-prov");
        assert_eq!(f.capability, "embedding");
        assert_eq!(
            f.env_var, "TOME_MY_PROV_API_KEY",
            "the finding must name the exact derived env var (not hardcoded)"
        );
        assert!(f.critical, "embedding must be health-critical (Unhealthy)");
    }

    #[test]
    fn credential_finding_summariser_and_reranker_are_not_critical() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_S_API_KEY");
            std::env::remove_var("TOME_R_API_KEY");
        }
        let mut cfg = Config::default();
        cfg.providers.insert(
            "s".to_string(),
            ProviderEntry {
                kind: ProviderKind::Openai,
                base_url: None,
                api_key: None,
            },
        );
        cfg.providers.insert(
            "r".to_string(),
            ProviderEntry {
                kind: ProviderKind::Voyage,
                base_url: None,
                api_key: None,
            },
        );
        cfg.summariser.provider = Some("s".to_string());
        cfg.summariser.model = Some("gpt-4o".to_string());
        cfg.reranker.provider = Some("r".to_string());
        cfg.reranker.model = Some("rerank-2".to_string());

        let findings = build_provider_credential_findings(&cfg);
        assert_eq!(findings.len(), 2, "one summariser + one reranker finding");
        // Sorted by (provider, capability): "r"/reranker then "s"/summariser.
        assert_eq!(findings[0].provider, "r");
        assert_eq!(findings[0].capability, "reranker");
        assert!(!findings[0].critical, "reranker → Degraded, not Unhealthy");
        assert_eq!(findings[1].provider, "s");
        assert_eq!(findings[1].capability, "summariser");
        assert!(
            !findings[1].critical,
            "summariser → Degraded, not Unhealthy"
        );
    }

    #[test]
    fn credential_finding_env_var_resolves_suppresses_finding() {
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::set_var("TOME_P_API_KEY", "env-secret");
        }
        let mut cfg = config_with_provider("p", ProviderKind::Openai, None);
        cfg.embedding.provider = Some("p".to_string());
        cfg.embedding.model = Some("text-embed".to_string());

        let findings = build_provider_credential_findings(&cfg);
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        assert!(
            findings.is_empty(),
            "a resolving env var suppresses the finding"
        );
    }

    #[test]
    fn credential_findings_never_carry_the_value() {
        // Structural: the finding struct has no field capable of holding a
        // credential value — only the env-var NAME and provider/capability. An
        // inline api_key that DOES resolve produces no finding at all, so it can
        // never leak. Assert both directions here.
        let _g = PROVIDER_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by PROVIDER_ENV_MUTEX.
        unsafe {
            std::env::remove_var("TOME_P_API_KEY");
        }
        // Missing credential → a finding, but with no value anywhere.
        let mut cfg = config_with_provider("p", ProviderKind::Openai, None);
        cfg.embedding.provider = Some("p".to_string());
        cfg.embedding.model = Some("text-embed".to_string());
        let findings = build_provider_credential_findings(&cfg);
        let rendered = format!("{findings:?}");
        assert!(
            !rendered.contains("sk-"),
            "no credential-shaped value in the finding: {rendered}"
        );
        assert!(rendered.contains("TOME_P_API_KEY"), "{rendered}");
    }

    // --- US11 (P11 fix): doctor–status hook-translation surface agreement ----

    /// Verify that `build_hook_translation_report` (doctor) and the
    /// `fill_hook_translation_harnesses` resolution strategy (status) agree
    /// on the number of active (scope-enabled, hook-capable) harnesses.
    ///
    /// This is the regression net for the P11 bug where doctor iterated the
    /// FULL module registry while status iterated only the scope-effective
    /// subset, causing them to report different counts for the same scope.
    ///
    /// The test uses REAL production harness modules (no override needed)
    /// because it passes the effective list explicitly to `build_hook_translation_report`
    /// and uses `with_effective_modules` for the status-side count — both
    /// paths honour the override seam and produce the same result with the
    /// real registry since the effective list gates inclusion.
    #[test]
    fn hook_translation_doctor_and_status_agree_on_scope_effective_set() {
        use crate::settings::resolver::{EffectiveHarness, EffectiveHarnessList};

        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let workspace = crate::workspace::WorkspaceName::parse("test-ws").unwrap();
        let cfg = crate::config::Config::default(); // translate_plugin_hooks = None (defaults on)

        // ---- scenario A: only gemini enabled --------------------------------

        let effective_gemini = EffectiveHarnessList {
            harnesses: vec![EffectiveHarness {
                name: "gemini".to_owned(),
                source_chain: vec!["project".to_owned()],
            }],
            excluded: vec![],
        };

        let report_a =
            build_hook_translation_report(&paths, &workspace, &cfg, Some(&effective_gemini));

        // Doctor must report exactly one row: gemini.
        assert_eq!(
            report_a.per_harness.len(),
            1,
            "scenario A (gemini only): doctor must have 1 row, got harnesses: {:?}",
            report_a
                .per_harness
                .iter()
                .map(|h| &h.harness)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            report_a.per_harness[0].harness, "gemini",
            "scenario A: the one doctor row must be gemini",
        );
        assert!(
            report_a.per_harness[0].enabled,
            "scenario A: gemini row must be enabled (config defaults on + in scope)",
        );

        // Status side: count via with_effective_modules (same override-aware path).
        let status_count_a = crate::harness::with_effective_modules(|mods| {
            effective_gemini
                .harnesses
                .iter()
                .filter(|h| {
                    mods.iter()
                        .any(|m| m.name() == h.name && m.hook_support().is_some())
                })
                .count()
        });
        assert_eq!(status_count_a, 1, "scenario A: status-side count must be 1");

        // Agreement: the two surfaces report the same active harness count.
        let doctor_active_a = report_a.per_harness.iter().filter(|h| h.enabled).count();
        assert_eq!(
            doctor_active_a, status_count_a,
            "scenario A: doctor active-row count ({doctor_active_a}) != status count ({status_count_a})",
        );

        // ---- scenario B: gemini + codex enabled → both surfaces report 2 ---

        let effective_two = EffectiveHarnessList {
            harnesses: vec![
                EffectiveHarness {
                    name: "gemini".to_owned(),
                    source_chain: vec!["project".to_owned()],
                },
                EffectiveHarness {
                    name: "codex".to_owned(),
                    source_chain: vec!["project".to_owned()],
                },
            ],
            excluded: vec![],
        };

        let report_b =
            build_hook_translation_report(&paths, &workspace, &cfg, Some(&effective_two));

        assert_eq!(
            report_b.per_harness.len(),
            2,
            "scenario B (gemini+codex): doctor must have 2 rows, got: {:?}",
            report_b
                .per_harness
                .iter()
                .map(|h| &h.harness)
                .collect::<Vec<_>>(),
        );

        let status_count_b = crate::harness::with_effective_modules(|mods| {
            effective_two
                .harnesses
                .iter()
                .filter(|h| {
                    mods.iter()
                        .any(|m| m.name() == h.name && m.hook_support().is_some())
                })
                .count()
        });
        assert_eq!(status_count_b, 2, "scenario B: status-side count must be 2");

        let doctor_active_b = report_b.per_harness.iter().filter(|h| h.enabled).count();
        assert_eq!(
            doctor_active_b, status_count_b,
            "scenario B: doctor active-row count ({doctor_active_b}) != status count ({status_count_b})",
        );
    }

    // --- Phase 12 / US4: corrupt-index check (FR-017) ----------------------

    /// Seed a bootstrapped DB and insert one `skill_embeddings` BLOB of
    /// `blob_dim` f32s, plus a `meta.embedder_dimension` of `meta_dim`.
    fn seed_index_with_dim(paths: &Paths, meta_dim: Option<usize>, blob_dim: Option<usize>) {
        let (e, r, s) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: e,
                reranker: r,
                summariser: s,
                profile: None,
            },
        )
        .unwrap();
        if let Some(d) = meta_dim {
            crate::index::meta::write_embedder_dimension(&conn, d).unwrap();
        }
        if let Some(d) = blob_dim {
            // Insert a skill row then a matching embedding BLOB of `d` f32s.
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            conn.execute(
                "INSERT INTO skills
                   (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
                 VALUES ('c', 'p', 's', 'd', '0.0.0', '/dev/null', 'h', ?1)",
                rusqlite::params![now],
            )
            .unwrap();
            let skill_id = conn.last_insert_rowid();
            let bytes: Vec<u8> = vec![0u8; d * 4]; // d f32s little-endian
            conn.execute(
                "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![skill_id, bytes],
            )
            .unwrap();
        }
    }

    #[test]
    fn corrupt_index_none_when_no_db() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        assert_eq!(check_corrupt_index(&paths), None);
    }

    #[test]
    fn corrupt_index_none_when_meta_dim_absent() {
        // Bundled / never-remote-reindexed: no meta dim → not applicable.
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        seed_index_with_dim(&paths, None, Some(384));
        assert_eq!(check_corrupt_index(&paths), None);
    }

    #[test]
    fn corrupt_index_none_when_dims_match() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        seed_index_with_dim(&paths, Some(1024), Some(1024));
        assert_eq!(check_corrupt_index(&paths), None);
    }

    #[test]
    fn corrupt_index_none_when_no_rows() {
        // Meta dim set but no stored vectors yet → nothing to compare.
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        seed_index_with_dim(&paths, Some(1024), None);
        assert_eq!(check_corrupt_index(&paths), None);
    }

    #[test]
    fn corrupt_index_detected_on_dimension_mismatch() {
        // Stored vectors are 768-d but meta says 1024-d → corrupt-remote-index.
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        seed_index_with_dim(&paths, Some(1024), Some(768));
        let ci = check_corrupt_index(&paths).expect("mismatch must be detected");
        assert_eq!(ci.stored, 768);
        assert_eq!(ci.expected, 1024);
    }
}
