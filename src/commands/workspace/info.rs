//! `tome workspace info [<name>]` — read-only report on one workspace.
//!
//! Phase 4 / US2.a-1 widens the report with the new junction-table
//! fields: enrolled catalogs, enabled plugins, bound projects, cached
//! summary lengths. Also accepts an optional `<name>` argument so the
//! command can target any workspace, not just the resolved scope.
//!
//! Read-only; never acquires the advisory lock; never bootstraps the
//! schema. Distinct from `tome status` which is the broader
//! diagnostic.

use std::io::Write;
use std::path::PathBuf;

use crate::cli::WorkspaceInfoArgs;
use crate::error::TomeError;
use crate::index::meta::MetaKey;
use crate::index::{self, integrity, meta, workspace_catalogs};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::settings::{self, WorkspaceSettings};
use crate::workspace::{
    EnabledPluginRecord, ModelIdentity, ResolvedScope, ScopeKind, ScopeSource, SummaryCacheState,
    WorkspaceCatalogEntry, WorkspaceInfo, WorkspaceName,
};

pub fn run(
    args: WorkspaceInfoArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    // Pick the target workspace name. With no positional, use the
    // resolved scope. With a positional, parse + verify membership in
    // the central DB (exit 13 if absent).
    let info = match args.name.as_deref() {
        None => assemble_with_details(scope, paths, args.details)?,
        Some(raw) => {
            let name = WorkspaceName::parse(raw)?;
            assemble_for_name(&name, paths, args.details)?
        }
    };
    emit(&info, mode)
}

/// Build a [`WorkspaceInfo`] for the resolved scope. Equivalent to
/// [`assemble_with_details`] with `details = false`.
pub fn assemble(scope: &ResolvedScope, paths: &Paths) -> Result<WorkspaceInfo, TomeError> {
    assemble_with_details(scope, paths, false)
}

/// Build a [`WorkspaceInfo`] for the resolved scope. When `details` is set,
/// the report's `plugin_details` field is populated with a per-plugin
/// breakdown of skills / commands / agents and their routing tiers.
pub fn assemble_with_details(
    scope: &ResolvedScope,
    paths: &Paths,
    details: bool,
) -> Result<WorkspaceInfo, TomeError> {
    let (scope_kind, path) = if scope.scope.is_global() {
        (ScopeKind::Global, None)
    } else {
        (ScopeKind::Workspace, scope.project_root.clone())
    };
    let name = scope.scope.name();
    compute_info(name, paths, scope_kind, scope.source, path, details)
}

/// Build a [`WorkspaceInfo`] for an explicitly-named workspace. The
/// `<name>` positional uses this — `source` is [`ScopeSource::Flag`] (the
/// positional is functionally a flag) and `path` is unset.
fn assemble_for_name(
    name: &WorkspaceName,
    paths: &Paths,
    details: bool,
) -> Result<WorkspaceInfo, TomeError> {
    // The membership check + the per-field reads share one DB handle;
    // delegate.
    let scope_kind = if name.is_reserved() {
        ScopeKind::Global
    } else {
        ScopeKind::Workspace
    };
    compute_info(name, paths, scope_kind, ScopeSource::Flag, None, details)
}

fn compute_info(
    name: &WorkspaceName,
    paths: &Paths,
    scope_kind: ScopeKind,
    source: ScopeSource,
    path: Option<PathBuf>,
    details: bool,
) -> Result<WorkspaceInfo, TomeError> {
    // Bootstrap-not-yet path: DB file missing. We can't validate the
    // workspace's central-registry membership, so we treat this as the
    // permissive "the DB hasn't been created yet" case and return zero
    // counts regardless of which workspace name was requested. Once
    // `index.db` exists (any write path), the membership check below
    // fires.
    if !paths.index_db.is_file() {
        return Ok(WorkspaceInfo {
            scope: scope_kind,
            path,
            source,
            catalogs: 0,
            plugins_total: 0,
            plugins_enabled: 0,
            skills_indexed: 0,
            schema_version: None,
            embedder: None,
            enrolled_catalogs: Vec::new(),
            enabled_plugins: Vec::new(),
            bound_projects: Vec::new(),
            summary_cache: None,
            plugin_details: None,
        });
    }

    let conn = index::open_read_only(&paths.index_db)?;

    // Integrity gate, mirrored from Phase 3.
    integrity::check(&conn)?;

    // Schema-version gate. Stale-schema is informational here; defer the
    // repair to `tome doctor --fix` (US5). MUST come BEFORE the
    // workspace-membership check because the `workspaces` table itself
    // is part of v2 — a stale-v1 DB has no such table.
    let schema_version = match index::current_schema_version(&conn) {
        Ok(Some(v)) => Some(v),
        Ok(None) => Some(index::SCHEMA_VERSION),
        Err(_) => None,
    };

    let embedder = read_embedder_identity(&conn)?;

    // Stale-schema → workspace-aware queries below may target tables
    // that don't exist. Collapse to zeros (the schema-fix suggestion
    // surfaces via doctor).
    if let Some(v) = schema_version
        && v < index::SCHEMA_VERSION
    {
        return Ok(WorkspaceInfo {
            scope: scope_kind,
            path,
            source,
            catalogs: 0,
            plugins_total: 0,
            plugins_enabled: 0,
            skills_indexed: 0,
            schema_version,
            embedder,
            enrolled_catalogs: Vec::new(),
            enabled_plugins: Vec::new(),
            bound_projects: Vec::new(),
            summary_cache: None,
            plugin_details: None,
        });
    }

    // Membership check: a named workspace must have a row. The
    // privileged `global` workspace is seeded on bootstrap.
    // Polish R-M7: route through the consolidated helper.
    let workspace_id: Option<i64> = crate::index::workspaces::resolve_id_optional(&conn, name)?;
    let Some(workspace_id) = workspace_id else {
        // The privileged `global` workspace is seeded on bootstrap. If
        // the DB is v2-shaped but somehow missing the `global` row,
        // that's an integrity failure — return zeros rather than
        // confusing the user with "global not found". For non-global
        // names, the workspace-not-found verdict is correct.
        if name.is_reserved() {
            return Ok(WorkspaceInfo {
                scope: scope_kind,
                path,
                source,
                catalogs: 0,
                plugins_total: 0,
                plugins_enabled: 0,
                skills_indexed: 0,
                schema_version,
                embedder,
                enrolled_catalogs: Vec::new(),
                enabled_plugins: Vec::new(),
                bound_projects: Vec::new(),
                summary_cache: None,
                plugin_details: None,
            });
        }
        return Err(TomeError::WorkspaceNotFound {
            name: name.as_str().to_owned(),
        });
    };

    // Catalog count: junction table (FR-360 / F11b).
    let catalogs_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_catalogs WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count catalogs: {e}")))?;

    // plugins_total = distinct (catalog, plugin) pairs in `skills`
    // (workspace-agnostic) — useful for the "how many plugins indexed
    // at all" diagnostic.
    let plugins_total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT DISTINCT catalog, plugin FROM skills
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count plugins_total: {e}")))?;

    // plugins_enabled = distinct (catalog, plugin) for this workspace's
    // enabled rows.
    let plugins_enabled: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT s.catalog || '/' || s.plugin)
             FROM workspace_skills AS ws
             JOIN skills           AS s  ON s.id = ws.skill_id
             WHERE ws.workspace_id = ?1",
            rusqlite::params![workspace_id],
            |row| row.get(0),
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("count plugins_enabled: {e}"))
        })?;

    let skills_indexed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_skills WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
            |row| row.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count skills_indexed: {e}")))?;

    let enrolled_catalogs = list_enrolled_catalogs(&conn, name)?;
    let enabled_plugins = list_enabled_plugins(&conn, workspace_id)?;
    let bound_projects = list_bound_projects(&conn, workspace_id)?;
    let summary_cache = read_summary_cache(paths, name);
    let plugin_details = if details {
        Some(assemble_plugin_details(&conn, name.as_str())?)
    } else {
        None
    };

    Ok(WorkspaceInfo {
        scope: scope_kind,
        path,
        source,
        catalogs: u32::try_from(catalogs_count).unwrap_or(u32::MAX),
        plugins_total: u32::try_from(plugins_total).unwrap_or(u32::MAX),
        plugins_enabled: u32::try_from(plugins_enabled).unwrap_or(u32::MAX),
        skills_indexed: u32::try_from(skills_indexed).unwrap_or(u32::MAX),
        schema_version,
        embedder,
        enrolled_catalogs,
        enabled_plugins,
        bound_projects,
        summary_cache,
        plugin_details,
    })
}

/// Group every enabled skill / command / agent for `workspace_name` into a
/// per-plugin breakdown, carrying each skill's and command's routing tier
/// (agents have no tier). Ordered deterministically by `(catalog, plugin)`.
fn assemble_plugin_details(
    conn: &rusqlite::Connection,
    workspace_name: &str,
) -> Result<Vec<crate::workspace::info::PluginDetail>, TomeError> {
    use crate::workspace::info::{DetailEntry, PluginDetail};
    use std::collections::BTreeMap;

    let tiered = crate::index::skills::tiered_entries_for_workspace(conn, workspace_name)?;
    let agents = crate::index::skills::enabled_agents_for_workspace(conn, workspace_name)?;

    let mut map: BTreeMap<(String, String), PluginDetail> = BTreeMap::new();
    for e in tiered {
        let pd = map
            .entry((e.catalog.clone(), e.plugin.clone()))
            .or_insert_with(|| PluginDetail {
                catalog: e.catalog.clone(),
                plugin: e.plugin.clone(),
                skills: Vec::new(),
                commands: Vec::new(),
                agents: Vec::new(),
            });
        let de = DetailEntry {
            name: e.name.clone(),
            kind: e.kind.as_str().to_string(),
            description: e.description.clone(),
            tier: Some(e.tier),
        };
        match e.kind {
            crate::plugin::identity::EntryKind::Command => pd.commands.push(de),
            _ => pd.skills.push(de),
        }
    }
    for a in agents {
        let pd = map
            .entry((a.catalog.clone(), a.plugin.clone()))
            .or_insert_with(|| PluginDetail {
                catalog: a.catalog.clone(),
                plugin: a.plugin.clone(),
                skills: Vec::new(),
                commands: Vec::new(),
                agents: Vec::new(),
            });
        pd.agents.push(DetailEntry {
            name: a.name.clone(),
            kind: "agent".to_string(),
            description: String::new(),
            tier: None,
        });
    }
    Ok(map.into_values().collect())
}

fn list_enrolled_catalogs(
    conn: &rusqlite::Connection,
    name: &WorkspaceName,
) -> Result<Vec<WorkspaceCatalogEntry>, TomeError> {
    let rows = workspace_catalogs::list_for_workspace(conn, name.as_str())?;
    Ok(rows
        .into_iter()
        .map(|r| WorkspaceCatalogEntry {
            name: r.catalog_name,
            url: r.url,
            pinned_ref: r.pinned_ref,
        })
        .collect())
}

fn list_enabled_plugins(
    conn: &rusqlite::Connection,
    workspace_id: i64,
) -> Result<Vec<EnabledPluginRecord>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT s.catalog, s.plugin, COUNT(*) AS skills
             FROM workspace_skills AS ws
             JOIN skills           AS s ON s.id = ws.skill_id
             WHERE ws.workspace_id = ?1
             GROUP BY s.catalog, s.plugin
             ORDER BY s.catalog, s.plugin",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare list_enabled_plugins: {e}"))
        })?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id], |row| {
            let catalog: String = row.get(0)?;
            let plugin: String = row.get(1)?;
            let skill_count: i64 = row.get(2)?;
            Ok(EnabledPluginRecord {
                catalog,
                plugin,
                skill_count: u32::try_from(skill_count).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query list_enabled_plugins: {e}"))
        })?;
    rows.collect::<Result<_, _>>().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("collect list_enabled_plugins: {e}"))
    })
}

fn list_bound_projects(
    conn: &rusqlite::Connection,
    workspace_id: i64,
) -> Result<Vec<PathBuf>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT project_path FROM workspace_projects
             WHERE workspace_id = ?1
             ORDER BY project_path",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare list_bound_projects: {e}"))
        })?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id], |row| {
            let p: String = row.get(0)?;
            Ok(PathBuf::from(p))
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query list_bound_projects: {e}"))
        })?;
    rows.collect::<Result<_, _>>().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("collect list_bound_projects: {e}"))
    })
}

fn read_summary_cache(paths: &Paths, name: &WorkspaceName) -> Option<SummaryCacheState> {
    let settings_path = paths.workspace_settings_file(name);
    if !settings_path.is_file() {
        return None;
    }
    let body =
        crate::util::bounded_read_to_string(&settings_path, crate::util::TOME_CONFIG_MAX).ok()?;
    let parsed: WorkspaceSettings = settings::parser::parse_workspace(&body).ok()?;
    let summaries = parsed.summaries?;
    use time::format_description::well_known::Rfc3339;
    let generated_at = summaries
        .generated_at
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::new());
    Some(SummaryCacheState {
        short_chars: summaries.short.chars().count(),
        long_chars: summaries.long.chars().count(),
        generated_at,
    })
}

fn read_embedder_identity(conn: &rusqlite::Connection) -> Result<Option<ModelIdentity>, TomeError> {
    let name = meta::read(conn, MetaKey::EmbedderName)?;
    let version = meta::read(conn, MetaKey::EmbedderVersion)?;
    match (name, version) {
        (Some(name), Some(version)) if !name.is_empty() => {
            Ok(Some(ModelIdentity { name, version }))
        }
        _ => Ok(None),
    }
}

fn emit(info: &WorkspaceInfo, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(info),
        Mode::Json => write_json(info),
    }
}

fn emit_human(info: &WorkspaceInfo) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let scope_label = match (info.scope, info.path.as_ref()) {
        (ScopeKind::Global, _) => "(global)".to_owned(),
        (ScopeKind::Workspace, Some(p)) => p.display().to_string(),
        (ScopeKind::Workspace, None) => "(named)".to_owned(),
    };
    writeln!(out, "Workspace:       {scope_label}")?;
    writeln!(out, "  resolved via:  {}", source_label(info.source))?;
    writeln!(out, "  catalogs:      {}", info.catalogs)?;
    writeln!(
        out,
        "  plugins:       {} total, {} enabled",
        info.plugins_total, info.plugins_enabled,
    )?;
    if info.schema_version.is_none() && info.plugins_enabled == 0 {
        writeln!(
            out,
            "  skills:        not yet bootstrapped (no enabled plugins)"
        )?;
    } else {
        writeln!(out, "  skills:        {} indexed", info.skills_indexed)?;
    }
    match info.schema_version {
        Some(v) => writeln!(out, "  schema:        v{v}")?,
        None => writeln!(out, "  schema:        —")?,
    }
    match info.embedder.as_ref() {
        Some(e) => writeln!(out, "  embedder:      {} {}", e.name, e.version)?,
        None => writeln!(out, "  embedder:      —")?,
    }
    if !info.enrolled_catalogs.is_empty() {
        writeln!(out, "  enrolled catalogs:")?;
        for c in &info.enrolled_catalogs {
            writeln!(out, "    - {} ({}) [{}]", c.name, c.url, c.pinned_ref)?;
        }
    }
    if !info.enabled_plugins.is_empty() {
        writeln!(out, "  enabled plugins:")?;
        for p in &info.enabled_plugins {
            writeln!(
                out,
                "    - {}/{} ({} skills)",
                p.catalog, p.plugin, p.skill_count
            )?;
        }
    }
    if let Some(details) = info.plugin_details.as_ref() {
        for pd in details {
            writeln!(out, "  {}/{}:", pd.catalog, pd.plugin)?;
            if !pd.skills.is_empty() {
                writeln!(out, "    skills:")?;
                for e in &pd.skills {
                    writeln!(
                        out,
                        "      - {} [tier {}]  {}",
                        e.name,
                        e.tier.unwrap_or(3),
                        e.description
                    )?;
                }
            }
            if !pd.commands.is_empty() {
                writeln!(out, "    commands:")?;
                for e in &pd.commands {
                    writeln!(
                        out,
                        "      - {} [tier {}]  {}",
                        e.name,
                        e.tier.unwrap_or(3),
                        e.description
                    )?;
                }
            }
            if !pd.agents.is_empty() {
                writeln!(out, "    agents:")?;
                for e in &pd.agents {
                    writeln!(out, "      - {}", e.name)?;
                }
            }
        }
    }
    if !info.bound_projects.is_empty() {
        writeln!(out, "  bound projects:")?;
        for p in &info.bound_projects {
            writeln!(out, "    - {}", p.display())?;
        }
    }
    match info.summary_cache.as_ref() {
        Some(s) => writeln!(
            out,
            "  summary:       short {} chars, long {} chars (generated {})",
            s.short_chars, s.long_chars, s.generated_at,
        )?,
        None => writeln!(out, "  summary:       not yet generated")?,
    }
    Ok(())
}

fn source_label(source: ScopeSource) -> &'static str {
    match source {
        ScopeSource::Flag => "--workspace flag",
        ScopeSource::Env => "TOME_WORKSPACE env",
        ScopeSource::Config => "config.toml [workspace] default",
        ScopeSource::ProjectMarker => "project marker walk",
        ScopeSource::GlobalFallback => "global fallback",
    }
}
