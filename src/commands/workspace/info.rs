//! `tome workspace info` — narrow read-only report on the resolved scope.
//!
//! Contract: `contracts/workspace-info.md`. Bootstrap-not-yet (the index
//! file is absent) is informational, not an error.
//!
//! Read-only; never acquires the advisory lock; never bootstraps the
//! schema. Distinct from `tome status` which is the broader diagnostic.

use std::io::Write;

use crate::error::TomeError;
use crate::index::meta::MetaKey;
use crate::index::{self, integrity, meta};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{
    ModelIdentity, ResolvedScope, Scope, ScopeKind, ScopeSource, WorkspaceInfo,
};

pub fn run(scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let info = assemble(scope, paths)?;
    emit(&info, mode)
}

/// Build a [`WorkspaceInfo`] from the on-disk state. Pure compute over
/// `(scope, paths)`; tests call this directly. Behaves identically whether
/// the index DB is missing, empty, or populated — bootstrap-not-yet
/// surfaces as `None` schema/embedder + zero counts (FR-103).
pub fn assemble(scope: &ResolvedScope, paths: &Paths) -> Result<WorkspaceInfo, TomeError> {
    let (scope_kind, path) = match &scope.scope {
        Scope::Global => (ScopeKind::Global, None),
        Scope::Workspace(root) => (ScopeKind::Workspace, Some(root.clone())),
    };

    let catalogs = catalog_count(scope, paths)?;
    let (plugins_total, plugins_enabled, skills_indexed, schema_version, embedder) =
        index_facts(scope, paths)?;

    Ok(WorkspaceInfo {
        scope: scope_kind,
        path,
        source: scope.source,
        catalogs,
        plugins_total,
        plugins_enabled,
        skills_indexed,
        schema_version,
        embedder,
    })
}

fn catalog_count(_scope: &ResolvedScope, paths: &Paths) -> Result<u32, TomeError> {
    let config_path = paths.global_config_file.clone();
    if !config_path.is_file() {
        return Ok(0);
    }
    let body = std::fs::read_to_string(&config_path)?;
    let parsed: crate::config::Config =
        toml::from_str(&body).map_err(|e| TomeError::WorkspaceMalformed {
            path: config_path.clone(),
            reason: format!("config.toml: {e}"),
        })?;
    Ok(u32::try_from(parsed.catalogs.len()).unwrap_or(u32::MAX))
}

type IndexFacts = (u32, u32, u32, Option<u32>, Option<ModelIdentity>);

fn index_facts(_scope: &ResolvedScope, paths: &Paths) -> Result<IndexFacts, TomeError> {
    let db_path = paths.index_db.clone();
    if !db_path.is_file() {
        return Ok((0, 0, 0, None, None));
    }
    let conn = index::open_read_only(&db_path)?;

    // Integrity gate: surface a corrupted index as code 35 (the contract's
    // explicit exit code for this command). The integrity check itself is
    // cheap; pessimistically running it costs nothing on a healthy DB.
    integrity::check(&conn)?;

    // Schema-version gate. The v2-shaped queries below (`JOIN
    // workspace_skills`) reference tables that don't exist in an older
    // on-disk schema. A stale-schema DB is not an error here — `tome
    // workspace info` is a read-only narrow report; the doctor's
    // schema-fix suggestion is the user-facing repair path. Return
    // zeros for the workspace-aware counts and let the caller (doctor)
    // emit `subsystem: "schema"` via `build_suggested_fixes`.
    let schema_version = match index::current_schema_version(&conn) {
        Ok(Some(v)) => Some(v),
        Ok(None) => Some(index::SCHEMA_VERSION),
        Err(_) => None,
    };
    if let Some(v) = schema_version
        && v < index::SCHEMA_VERSION
    {
        let embedder = read_embedder_identity(&conn)?;
        return Ok((0, 0, 0, schema_version, embedder));
    }

    let plugins_total: i64 = conn
        .query_row("SELECT COUNT(DISTINCT plugin) FROM skills", [], |r| {
            r.get(0)
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("plugins_total: {e}")))?;
    let plugins_enabled: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT s.plugin)
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE w.name = 'global'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("plugins_enabled: {e}")))?;
    let skills_indexed: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM skills AS s
             JOIN workspace_skills AS ws ON ws.skill_id = s.id
             JOIN workspaces       AS w  ON w.id = ws.workspace_id
             WHERE w.name = 'global'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("skills_indexed: {e}")))?;

    let embedder = read_embedder_identity(&conn)?;

    Ok((
        u32::try_from(plugins_total).unwrap_or(u32::MAX),
        u32::try_from(plugins_enabled).unwrap_or(u32::MAX),
        u32::try_from(skills_indexed).unwrap_or(u32::MAX),
        schema_version,
        embedder,
    ))
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
        (ScopeKind::Workspace, None) => "(unknown)".to_owned(),
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
    Ok(())
}

fn source_label(source: ScopeSource) -> &'static str {
    match source {
        ScopeSource::Flag => "--workspace flag",
        ScopeSource::GlobalFlag => "--global flag",
        ScopeSource::Env => "TOME_WORKSPACE env",
        ScopeSource::CwdWalk => "CWD walk",
        ScopeSource::GlobalFallback => "global fallback",
    }
}
