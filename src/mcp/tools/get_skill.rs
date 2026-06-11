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

use crate::error::TomeError;
use crate::index::skills;
use crate::mcp::state::McpState;
use crate::plugin::frontmatter;
use crate::substitution::{self, SubstitutionContext, SubstitutionError};

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
/// 1. Verify the resolved scope's `workspace_catalogs` DB enrolment has
///    the named catalog (`unknown_catalog` per contract).
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

    // FF3: catalog existence is resolved from the `workspace_catalogs` DB
    // (inside `lookup_skill`, below), not `config.toml [catalogs]` — the
    // latter is never written in production (`tome catalog add` enrols only
    // into the DB), so reading it here returned `unknown_catalog` for every
    // enrolled catalog on a fresh install.
    //
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
            .map_err(|e| {
                // C-L1: best-effort MCP-surface `tome.error` (closed category
                // only), with this session's `calling_harness`. Never alters the
                // returned `McpError`. This is the one `TomeError`→`McpError`
                // conversion in this handler (the other error arms are non-`TomeError`
                // lookup/read outcomes already shaped to the contract codes).
                crate::mcp::enqueue_tool_error(&state, e.category());
                // US4 deferral: no clean plugin context at this error boundary.
                // The attributed `catalog.<id>.error` requires a (non-optional)
                // `plugin_version`, but a *lookup* failure here means the entry
                // row was never resolved — there is no trustworthy version to
                // attribute, and the catalog/plugin may not even resolve to an
                // allowlisted source. Fabricating a version would be worse than
                // the anonymous-only `tome.error` already emitted above, so the
                // attributed error stays deferred at this boundary.
                // FR-050: nudge the off-path flush timer on the ≥50 crossing.
                state.note_enqueue();
                internal(&input, started, e.to_string(), e.category().as_str())
            })?;

    let hit = match lookup {
        LookupOutcome::Found(hit) => hit,
        LookupOutcome::UnknownCatalog => {
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

    let LookupHit {
        body_path,
        plugin_version,
    } = hit;
    let skill_path = body_path;
    // Capture the version before it is moved into the substitution closure
    // below — the catalog-attributed emit (further down) needs it, and it is a
    // PUBLISHED manifest value (the FR-059 carve-out), not a secret.
    let attributed_plugin_version = plugin_version.clone();

    // The actual file read + frontmatter strip + sibling walk is all
    // synchronous I/O; do it on the blocking pool.
    let read_input = input.clone_for_log();
    let read_path = skill_path.clone();
    let body_and_resources =
        tokio::task::spawn_blocking(move || read_skill_and_resources(&read_path))
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

    let (raw_content, resources) = body_and_resources;

    // Phase 5 / US2.c (FR-101): run the substitution pipeline over the
    // frontmatter-stripped body so callers see Stage 1 (built-ins) +
    // Stage 2 (env passthrough) values. `get_skill` never receives args
    // (it's the read-side; Stage 3 + Stage 4 are exercised via the
    // `prompts/get` MCP surface in `mcp::prompts`), so `args = None`
    // and `declared_args = []`.
    //
    // Build + render are pure compute (built-ins read context fields;
    // env reads `std::env::var`; the data-dir built-ins call
    // `create_dir_all` which is sync). Run on the blocking pool to keep
    // the runtime responsive per the sync-boundary discipline.
    let ctx_state = state.clone();
    let ctx_input = input.clone_for_log();
    let ctx_skill_path = skill_path.clone();
    let ctx_plugin_version = plugin_version;
    let rendered_result = tokio::task::spawn_blocking(move || {
        let ctx = build_substitution_context(
            &ctx_state,
            &ctx_input,
            &ctx_skill_path,
            ctx_plugin_version,
        )?;
        substitution::render(&raw_content, &ctx).map_err(map_substitution_error)
    })
    .await
    .map_err(|e| internal(&input, started, format!("render join: {e}"), "internal"))?;

    let content = match rendered_result {
        Ok(s) => s,
        Err((code, err)) => return Err(emit_error(&input, started, code, err)),
    };

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

    // FR-027/FR-028: `tome.entry_invoked` once the entry body is fetched. The
    // `entry_kind` is always `Skill` — `get_skill` only ever resolves skills
    // (FR-084; `lookup_skill` hardcodes `EntryKind::Skill`). The `rank_bucket`
    // is THIS entry's position in the preceding search this session (the funnel
    // join; `None` when no search ranked it). Best-effort enqueue — a sub-ms
    // local append that never blocks the tool call or flushes.
    crate::telemetry::enqueue(crate::telemetry::event::EntryInvoked {
        entry_kind: crate::telemetry::event::EntryKind::Skill,
        rank_bucket: crate::mcp::rank_bucket_for(&state, &input.name),
        calling_harness: crate::mcp::calling_harness(&state),
    });

    // FR-052: ALONGSIDE the anonymous `tome.entry_invoked`, emit the attributed
    // `catalog.<id>.entry_invoked` ONLY when this entry's catalog resolves — by
    // SOURCE, at emit time — to an allowlisted catalog. The attribution read
    // opens the index read-only with no lock (NFR-009); `None` ⇒ anonymous only.
    // The artefact names (entry/plugin name + version) are PUBLISHED values
    // (FR-059), never secrets. Best-effort — never alters the tool result.
    if let Some(catalog_id) = crate::telemetry::resolve_attribution(&state.scope, &input.catalog) {
        crate::telemetry::enqueue_attributed(crate::telemetry::event::AttributedEntryInvoked {
            entry_name: input.name.clone(),
            entry_kind: crate::telemetry::event::EntryKind::Skill,
            plugin_name: input.plugin.clone(),
            plugin_version: attributed_plugin_version,
            catalog_id,
            calling_harness: crate::mcp::calling_harness(&state),
        });
    }
    // FR-050: nudge the off-path flush timer on the ≥50-enqueue crossing.
    state.note_enqueue();

    // The `Output.path` field is documented as the absolute path to the
    // skill body (see the `Output` struct's doc comment) — emit the
    // resolved `skill_path` (which is absolute) rather than the row's
    // catalog-relative stored form. Pre-US1.c this returned the raw row
    // value, which only happened to be correct when absolute-path
    // legacy data was indexed.
    //
    // R-M5 (US1.d reviewer pass): the boxed `SkillRecord` carried on
    // `LookupHit` was dropped — it was kept as a "future extensions"
    // placeholder but never read here, costing an Arc-equivalent heap
    // allocation per call.
    Ok(Output {
        content,
        path: skill_path.display().to_string(),
        resources,
    })
}

/// Lookup outcome carrying the resolved absolute body path so the read
/// step doesn't have to open the DB a second time.
///
/// The `body_path` is computed via
/// [`skills::resolve_entry_body_path`] — `skills.path` stores the
/// **relative** path under the plugin's catalog directory; resolving it
/// in the same `spawn_blocking` as the row lookup keeps the read path
/// honest. (Pre-US1.c this module used `PathBuf::from(&row.path)`
/// directly, which only worked when the index was populated via a
/// codepath that happened to store an absolute string — never the case
/// post-F11b. The bug was latent because no in-tree integration test
/// exercised the file-read branch.)
///
/// R-M5 (US1.d reviewer pass): the boxed `SkillRecord` field was
/// removed; it was a "future extensions" placeholder costing a heap
/// allocation per call with no read site.
///
/// US2.c (Phase 5): re-added `plugin_version` as a single scalar field
/// (not the whole `SkillRecord`) so the substitution context can be
/// built without a second DB read. Mirrors the registry-cached
/// `PromptEntry.plugin_version` shape in `mcp::prompts`.
struct LookupHit {
    body_path: PathBuf,
    plugin_version: String,
}

enum LookupOutcome {
    UnknownCatalog,
    Found(LookupHit),
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
    let db_path = paths.index_db.clone();
    let conn = crate::index::db::open_read_only(&db_path)?;
    let workspace_name = scope.name().as_str();
    // FF3: catalog existence resolves from `workspace_catalogs`, not
    // `config.toml`. Checked FIRST so an unknown catalog takes precedence
    // over unknown_plugin/unknown_skill — preserving the contract ordering
    // the old `config.catalogs.contains_key` gate enforced before the
    // index lookup.
    if crate::index::workspace_catalogs::find(&conn, workspace_name, catalog)?.is_none() {
        return Ok(LookupOutcome::UnknownCatalog);
    }
    // Phase 5: `get_skill` defaults to the `Skill` kind (FR-084) — the
    // tool only surfaces skills, not commands.
    match skills::find(
        &conn,
        workspace_name,
        catalog,
        plugin,
        crate::plugin::identity::EntryKind::Skill,
        name,
    )? {
        Some(row) if row.enabled => {
            // Resolve the row's stored relative path to an absolute
            // body path via the shared helper. A failure here means the
            // catalog enrolment exists but the on-disk plugin directory
            // is gone (cache evicted, manifest drift, …); we surface
            // this through the existing `skill_file_missing` envelope
            // downstream rather than `unknown_skill` because the
            // index entry is real — the filesystem isn't.
            let body_path = skills::resolve_entry_body_path(
                &conn,
                paths,
                workspace_name,
                catalog,
                plugin,
                &row.path,
            )?;
            Ok(LookupOutcome::Found(LookupHit {
                body_path,
                plugin_version: row.plugin_version,
            }))
        }
        Some(_) => Ok(LookupOutcome::UnknownSkill),
        None => {
            // Distinguish "plugin not enabled at all" from "plugin
            // enabled but doesn't have this skill name". The shipping
            // contract treats zero (catalog, plugin) rows as
            // `unknown_plugin`. `list_for_plugin` scoped to the resolved
            // workspace is what determines "enabled" here.
            let any = skills::list_for_plugin(&conn, workspace_name, catalog, plugin)?;
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

/// FR-S-02: walk the skill's directory tree and collect every file
/// path, but **reject symlinks** outright. A hostile catalog author can
/// commit `skills/foo/credentials -> ~/.ssh/id_rsa`; without this guard
/// the agent client receives that path as a "skill resource" and the
/// file-reading tool will follow the symlink. The defence in depth is
/// `entry.file_type()` (which uses `lstat` and does NOT follow
/// symlinks) plus an explicit `is_symlink()` skip.
///
/// Returned-but-not-followed symlinks could still be sniffed by an
/// agent if the agent's file tool resolves them — Tome can't prevent
/// that, but we can at least not enumerate them ourselves.
fn walk_dir(dir: &Path, exclude: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            // Skip silently — `resources` is informational. We don't
            // log here (would flood under hostile-catalog scenarios)
            // but the symlink is invisible to the agent client.
            continue;
        }
        if ft.is_dir() {
            walk_dir(&path, exclude, out)?;
        } else if path != exclude {
            out.push(path.display().to_string());
        }
    }
    Ok(())
}

/// Build the [`SubstitutionContext`] for one `get_skill` call.
///
/// Mirrors `mcp::prompts::build_get_context` for fields shared between
/// the two surfaces (catalog/plugin/entry scalars, paths, clock, lazy
/// data-dir slots). The two divergences from prompts:
///
/// - `args` is always `None` and `declared_args` always empty (get_skill
///   never accepts args — Stage 3 + Stage 4 are unreachable here).
/// - `plugin_version` is sourced from the `SkillRecord.plugin_version`
///   captured in `LookupHit`, not the registry cache (registry is the
///   prompts-side construct).
fn build_substitution_context(
    state: &McpState,
    input: &Input,
    skill_path: &Path,
    plugin_version: String,
) -> Result<SubstitutionContext, (&'static str, McpError)> {
    // Polish M-2 (Phase 5): delegates to the shared
    // `mcp::substitution_helpers::build_context_for_entry` — same body
    // shape as `prompts::build_get_context` modulo the
    // args/declared_args constants (get_skill never accepts args).
    crate::mcp::substitution_helpers::build_context_for_entry(
        input.catalog.clone(),
        input.plugin.clone(),
        plugin_version,
        input.name.clone(),
        skill_path.to_path_buf(),
        state.scope.scope.name(),
        state.scope.project_root.clone(),
        state.paths.clone(),
        None,
        Vec::new(),
    )
    .map_err(|e| {
        (
            "substitution_failed",
            McpError::internal_error(
                format!("substitution context build failed: {e}"),
                Some(json!({ "code": "substitution_failed" })),
            ),
        )
    })
}

/// Map a [`SubstitutionError`] surfaced by the render pipeline to a
/// (`code`, [`McpError`]) tuple. Mirrors the variant routing in
/// `mcp::prompts::emit_tome_error_for_get` so both MCP surfaces agree
/// on `data.code` slugs.
///
/// `InvalidArgumentFrontmatter` and `PromptArgumentMismatch` are
/// defensively mapped even though `get_skill` never supplies args
/// (declared_args is empty and Stage 3 is unreachable) — keeps the
/// match exhaustive against the closed `SubstitutionError` enum so a
/// future variant addition surfaces as a compile error here.
fn map_substitution_error(err: SubstitutionError) -> (&'static str, McpError) {
    match err {
        SubstitutionError::PluginDataDirCreationFailed { path, source } => (
            "plugin_data_dir_write_failed",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "plugin data dir creation failed at {}: {source}",
                    path.display()
                ),
                Some(json!({
                    "code": "plugin_data_dir_write_failed",
                    "path": path.display().to_string(),
                })),
            ),
        ),
        SubstitutionError::WorkspaceDataDirCreationFailed { path, source } => (
            "workspace_data_dir_write_failed",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "workspace data dir creation failed at {}: {source}",
                    path.display()
                ),
                Some(json!({
                    "code": "workspace_data_dir_write_failed",
                    "path": path.display().to_string(),
                })),
            ),
        ),
        SubstitutionError::InvalidArgumentFrontmatter { file, reason } => (
            "invalid_argument_frontmatter",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "invalid argument frontmatter in {}: {reason}",
                    file.display()
                ),
                Some(json!({
                    "code": "invalid_argument_frontmatter",
                    "file": file.display().to_string(),
                })),
            ),
        ),
        SubstitutionError::PromptArgumentMismatch { expected, supplied } => (
            "prompt_argument_mismatch",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("prompt argument mismatch: expected {expected}, supplied {supplied}"),
                Some(json!({
                    "code": "prompt_argument_mismatch",
                    "expected": expected,
                    "supplied": supplied,
                })),
            ),
        ),
    }
}

/// Build the `internal_error` envelope plus an error log event.
fn internal(input: &Input, started: Instant, msg: String, code: &str) -> McpError {
    // FR-M-LOG-1: scrub error chains before logging — reqwest / git
    // error messages can carry signed URLs.
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog,
        plugin = input.plugin,
        name = input.name,
        error_code = code,
        error_message = %scrubbed,
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
