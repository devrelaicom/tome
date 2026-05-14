//! `get_skill` MCP tool — input/output schemas + handler.
//!
//! Contract: [`mcp-tools.md` §get_skill](../../../specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::catalog::store;
use crate::error::TomeError;
use crate::index::skills;
use crate::mcp::state::McpState;
use crate::plugin::frontmatter;

/// The tool description per `mcp-tools.md` §get_skill lives on the
/// `#[tool]`-decorated method in `mcp::server` as a doc comment.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub catalog: String,
    pub plugin: String,
    /// The skill `name` field as returned by `search_skills`.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// SKILL.md body with YAML frontmatter stripped. Body is otherwise
    /// verbatim — no normalisation, no rewrites, no path-relative-to-
    /// absolute resolution in code blocks.
    pub content: String,
    /// Absolute path to the SKILL.md file.
    pub path: String,
    /// Absolute paths of every OTHER file in the skill's directory
    /// (recursive). The agent may load any of them via its own
    /// file-reading tools.
    pub resources: Vec<String>,
}

/// Pipeline:
///
/// 1. Verify the resolved scope's config has the named catalog
///    (`unknown_catalog` per contract).
/// 2. Look up `(catalog, plugin, name)` in the index. Distinguish
///    `unknown_plugin` (no rows for that catalog+plugin pair) from
///    `unknown_skill` (no row, or row exists but `enabled = 0`).
/// 3. Read SKILL.md, strip frontmatter via `plugin::frontmatter` (the
///    same parser the enable pipeline uses).
/// 4. Walk the SKILL.md's parent directory recursively, gather every
///    other file's absolute path, sort lexicographically.
/// 5. Return.
pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    if input.catalog.is_empty() || input.plugin.is_empty() || input.name.is_empty() {
        return Err(McpError::invalid_params(
            "catalog, plugin, and name must be non-empty",
            None,
        ));
    }

    let config = store::load(&state.paths.config_file_for(&state.scope.scope))
        .map_err(|e| internal(&input, started, e.to_string(), e.category()))?;

    if !config.catalogs.contains_key(&input.catalog) {
        return Err(emit_error(
            &input,
            started,
            "unknown_catalog",
            McpError::invalid_params(
                format!(
                    "catalog `{}` is not enabled in the resolved scope",
                    input.catalog
                ),
                Some(json!({ "code": "unknown_catalog", "catalog": input.catalog })),
            ),
        ));
    }

    // The index read needs the resolved scope's DB. Run inside a
    // `spawn_blocking` so rusqlite doesn't block the runtime.
    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();
    let catalog = input.catalog.clone();
    let plugin = input.plugin.clone();
    let name = input.name.clone();

    let lookup =
        tokio::task::spawn_blocking(move || lookup_skill(&paths, &scope, &catalog, &plugin, &name))
            .await
            .map_err(|e| internal(&input, started, format!("lookup join: {e}"), "internal"))?
            .map_err(|e| internal(&input, started, e.to_string(), e.category()))?;

    let row = match lookup {
        LookupOutcome::Found(row) => row,
        LookupOutcome::UnknownPlugin => {
            return Err(emit_error(
                &input,
                started,
                "unknown_plugin",
                McpError::invalid_params(
                    format!(
                        "plugin `{}/{}` is not enabled in the resolved scope",
                        input.catalog, input.plugin
                    ),
                    Some(json!({
                        "code": "unknown_plugin",
                        "catalog": input.catalog,
                        "plugin": input.plugin,
                    })),
                ),
            ));
        }
        LookupOutcome::UnknownSkill => {
            return Err(emit_error(
                &input,
                started,
                "unknown_skill",
                McpError::invalid_params(
                    format!(
                        "skill `{}/{}/{}` is not enabled in the resolved scope",
                        input.catalog, input.plugin, input.name,
                    ),
                    Some(json!({
                        "code": "unknown_skill",
                        "catalog": input.catalog,
                        "plugin": input.plugin,
                        "name": input.name,
                    })),
                ),
            ));
        }
    };

    let skill_path = PathBuf::from(&row.path);

    // The actual file read + frontmatter strip + sibling walk is all
    // synchronous I/O; do it on the blocking pool.
    let read_input = input.clone_for_log();
    let body_and_resources =
        tokio::task::spawn_blocking(move || read_skill_and_resources(&skill_path))
            .await
            .map_err(|e| internal(&read_input, started, format!("read join: {e}"), "internal"))?
            .map_err(|e| match e {
                ReadError::SkillFileMissing(p) => emit_error(
                    &read_input,
                    started,
                    "skill_file_missing",
                    McpError::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("skill file is missing: {}", p.display()),
                        Some(json!({
                            "code": "skill_file_missing",
                            "path": p.display().to_string(),
                        })),
                    ),
                ),
                ReadError::FrontmatterStripFailed(detail) => emit_error(
                    &read_input,
                    started,
                    "frontmatter_strip_failed",
                    McpError::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("frontmatter parse failed: {detail}"),
                        Some(json!({ "code": "frontmatter_strip_failed" })),
                    ),
                ),
                ReadError::Io(io) => internal(&read_input, started, io.to_string(), "io"),
            })?;

    let (content, resources) = body_and_resources;

    info!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        result = "ok",
        body_bytes = content.len(),
        resource_count = resources.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(Output {
        content,
        path: row.path,
        resources,
    })
}

enum LookupOutcome {
    Found(Box<skills::SkillRecord>),
    UnknownPlugin,
    UnknownSkill,
}

fn lookup_skill(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::Scope,
    catalog: &str,
    plugin: &str,
    name: &str,
) -> Result<LookupOutcome, TomeError> {
    let db_path = paths.index_db_for(scope);
    let conn = crate::index::db::open_read_only(&db_path)?;
    match skills::find(&conn, catalog, plugin, name)? {
        Some(row) if row.enabled => Ok(LookupOutcome::Found(Box::new(row))),
        Some(_) => Ok(LookupOutcome::UnknownSkill),
        None => {
            // Distinguish "plugin not enabled at all" from "plugin
            // enabled but doesn't have this skill name". The shipping
            // contract treats zero (catalog, plugin) rows as
            // `unknown_plugin`.
            let any = skills::list_for_plugin(&conn, catalog, plugin)?;
            if any.is_empty() {
                Ok(LookupOutcome::UnknownPlugin)
            } else {
                Ok(LookupOutcome::UnknownSkill)
            }
        }
    }
}

enum ReadError {
    SkillFileMissing(PathBuf),
    FrontmatterStripFailed(String),
    Io(std::io::Error),
}

fn read_skill_and_resources(skill_path: &Path) -> Result<(String, Vec<String>), ReadError> {
    if !skill_path.is_file() {
        return Err(ReadError::SkillFileMissing(skill_path.to_path_buf()));
    }

    let parsed = frontmatter::parse_skill_frontmatter(skill_path).map_err(|e| {
        // The enable pipeline rejects skills whose frontmatter is
        // unparsable, so this branch is genuinely unreachable for an
        // indexed skill — but the contract names it so we surface it.
        ReadError::FrontmatterStripFailed(e.to_string())
    })?;

    let parent = skill_path
        .parent()
        .ok_or_else(|| ReadError::SkillFileMissing(skill_path.to_path_buf()))?;

    let mut resources: Vec<String> = Vec::new();
    walk_dir(parent, skill_path, &mut resources).map_err(ReadError::Io)?;
    resources.sort();

    Ok((parsed.body, resources))
}

fn walk_dir(dir: &Path, exclude: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk_dir(&path, exclude, out)?;
        } else if path != exclude {
            out.push(path.display().to_string());
        }
    }
    Ok(())
}

/// Build the `internal_error` envelope plus an error log event.
fn internal(input: &Input, started: Instant, msg: String, code: &str) -> McpError {
    error!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        error_code = code,
        error_message = %msg,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(json!({ "code": code })))
}

/// Log the error variants the contract recognises, then return the
/// caller's pre-built `McpError` unchanged.
fn emit_error(input: &Input, started: Instant, code: &str, err: McpError) -> McpError {
    info!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        result = code,
        body_bytes = 0,
        resource_count = 0,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );
    err
}

impl Input {
    fn clone_for_log(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            plugin: self.plugin.clone(),
            name: self.name.clone(),
        }
    }
}
