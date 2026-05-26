//! Phase 5 — MCP `prompts` capability surface.
//!
//! Per the T122 rmcp-verification notes
//! (`specs/005-phase-5-commands-prompts/notes/rmcp-prompts-api.md`):
//!
//! 1. We re-export `rmcp::model::Prompt` (and friends) as the wire shape;
//!    defining a parallel Tome `PromptDescriptor` would force double
//!    marshalling for zero gain.
//! 2. Prompts are NOT compile-time-known — they're driven by the resolved
//!    workspace's enabled-and-user-invocable entries at startup. We
//!    therefore build the [`rmcp::handler::server::router::prompt::PromptRouter`]
//!    by hand via `PromptRoute::new_dyn`, NOT the `#[prompt_router]` macro.
//! 3. `PromptRouter::list_all` returns `Vec<Prompt>` directly so we don't
//!    need a Tome-side `PromptListResponse` wrapper.
//! 4. The [`PromptsCapability`] is declared in
//!    [`crate::mcp::server::Server::get_info`] alongside the existing
//!    `tools` capability.
//!
//! Contract: `specs/005-phase-5-commands-prompts/contracts/mcp-prompts.md`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::prompt::PromptContext;
use rmcp::handler::server::router::prompt::{PromptRoute, PromptRouter};
use rmcp::model::ErrorCode;
use rusqlite::Connection;
use serde_json::json;
use tracing::{error, info, warn};

use crate::error::TomeError;
use crate::index::skills::{SkillRecord, resolve_entry_body_path};
use crate::index::workspaces::resolve_id_required;
use crate::mcp::prompt_collision::{CollisionRecord, EntryIdentity, resolve_collisions};
use crate::mcp::prompt_name::derive_name;
use crate::mcp::state::McpState;
use crate::paths::Paths;
use crate::plugin::frontmatter::parse_skill_frontmatter;
use crate::plugin::identity::EntryKind;
use crate::substitution::{self, ArgumentValues, SubstitutionContext, SubstitutionError};
use crate::workspace::WorkspaceName;

// --- Wire-shape re-exports ------------------------------------------------

/// Alias for rmcp's wire-level `Prompt`. The contract referred to this
/// type as `PromptDescriptor` so the alias keeps internal naming
/// continuity while delegating the schema to rmcp.
pub use rmcp::model::GetPromptResult as PromptGetResponse;
pub use rmcp::model::Prompt as PromptDescriptor;
pub use rmcp::model::PromptArgument;
pub use rmcp::model::PromptMessage;
pub use rmcp::model::PromptMessageContent as PromptContent;
pub use rmcp::model::PromptMessageRole as PromptRole;

// --- Description truncation per FR-066 ------------------------------------

/// Max length (in `char`s, not bytes) of the `description` field on each
/// prompt in `prompts/list`. Per FR-066 — see the contract's "Description
/// cap" section for the 300-vs-150-vs-`search_skills` rationale. 300 is
/// the *upper bound* applied at render-time; plugin authors who want
/// shorter blurbs can set `description` shorter explicitly.
pub const DESCRIPTION_MAX_CHARS: usize = 300;

/// Truncate `description` at a char (Unicode scalar value) boundary and
/// append U+2026 (`…`) when truncated. Mirrors FR-092 in
/// `search_skills` — same Unicode-safe approach.
fn truncate_description(s: &str) -> String {
    if s.chars().count() <= DESCRIPTION_MAX_CHARS {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(DESCRIPTION_MAX_CHARS - 1).collect();
    out.push('\u{2026}');
    out
}

// --- The catch-all argument's default description --------------------------

/// Default description for the catch-all `args` argument when an entry
/// declares no named arguments and the frontmatter carries no
/// `argument-hint`. Per the contract's "Argument schema derivation —
/// Case B".
const CATCHALL_DEFAULT_DESCRIPTION: &str =
    "Optional free-form input passed to the entry as a single positional argument.";

/// Heuristic: does `body` reference the `$ARGUMENTS` placeholder? Used
/// only to decide whether the catch-all `args` argument is surfaced in
/// `prompts/list` (US3 will replace this with a real regex check; for
/// US1.b a substring sweep is sufficient).
fn body_references_arguments(body: &str) -> bool {
    body.contains("$ARGUMENTS")
}

// --- One promoted entry, ready for registration ---------------------------

/// One resolved entry, sitting in `PromptRegistry::by_name` keyed on the
/// final prompt name (post-collision-suffixing). Carries enough state to
/// (a) emit the `PromptDescriptor` for `prompts/list` and (b) read the
/// entry body for `prompts/get` in US1.c.
#[derive(Debug, Clone)]
pub struct PromptEntry {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    /// Truncated description for `prompts/list` (already capped per
    /// FR-066).
    pub description: String,
    /// Absolute path to the entry file on disk, resolved at registry
    /// build time via [`resolve_entry_body_path`]. Always absolute (the
    /// resolver joins the plugin dir to the catalog-relative stored
    /// path). US1.d reviewer pass (R-M4) — pre-US1.d the prompts/get
    /// hot path round-tripped this through `display().to_string()` +
    /// re-resolution, both lossy (non-UTF8 paths) and pointless (the
    /// resolver short-circuits on `is_absolute`).
    pub path: PathBuf,
    /// Named arguments declared in the entry's frontmatter.
    pub arguments: Vec<String>,
    /// Frontmatter `argument-hint`, used as the `args` description in
    /// the catch-all case.
    pub argument_hint: Option<String>,
    /// `true` when the entry body contains `$ARGUMENTS` (US1.b
    /// heuristic; US3 will replace this with a real regex parse).
    pub body_uses_arguments: bool,
    /// Plugin version (`plugin.json` `version` field) cached from the
    /// `skills.plugin_version` column at registry build time. US1.d
    /// reviewer pass (R-M3) — pre-US1.d the prompts/get hot path opened
    /// a second read-only DB connection per request to fetch this.
    pub plugin_version: String,
}

impl PromptEntry {
    /// Build the rmcp [`PromptDescriptor`] (sans the prompt name, which
    /// is the registry HashMap key) including argument-schema derivation
    /// per FR-070 / FR-071 / FR-072.
    pub fn descriptor(&self, name: String) -> PromptDescriptor {
        let arguments = if !self.arguments.is_empty() {
            // Case A — named arguments. All required strings per FR-070,
            // declaration order preserved.
            Some(
                self.arguments
                    .iter()
                    .map(|n| PromptArgument::new(n.clone()).with_required(true))
                    .collect(),
            )
        } else if self.body_uses_arguments {
            // Case B — catch-all `args`. Optional. Description comes from
            // `argument-hint` if set, otherwise the documented generic
            // string.
            let description = self
                .argument_hint
                .clone()
                .unwrap_or_else(|| CATCHALL_DEFAULT_DESCRIPTION.to_owned());
            Some(vec![
                PromptArgument::new("args")
                    .with_description(description)
                    .with_required(false),
            ])
        } else {
            // No declared args and no `$ARGUMENTS` reference — surface
            // no argument schema at all.
            None
        };

        PromptDescriptor::new(name, Some(self.description.clone()), arguments)
    }
}

// --- The registry itself --------------------------------------------------

/// Built once at MCP server startup; immutable for the session.
#[derive(Debug, Clone, Default)]
pub struct PromptRegistry {
    /// Final prompt name → resolved entry. Keys are post-collision
    /// suffixing.
    pub by_name: HashMap<String, PromptEntry>,
    /// One record per collision bucket (size >= 2). Surfaced via the
    /// doctor extensions in US5.
    pub collisions: Vec<CollisionRecord>,
}

impl PromptRegistry {
    /// Build the registry for one workspace by querying enabled-and-
    /// user-invocable entries, parsing each entry's frontmatter from
    /// disk, deriving the prompt name (honouring any `prompt_name`
    /// override), and resolving collisions.
    ///
    /// Entries whose on-disk file is missing or whose frontmatter is
    /// unparsable are skipped with a `warn!` event — the rest of the
    /// registry is built so a single bad entry doesn't take the whole
    /// `prompts/list` down. Argument-name validation already ran at
    /// `tome plugin enable` time (FR-013c / exit 29) so anything in the
    /// DB has well-formed argument names.
    pub fn build_for_workspace(
        workspace_name: &WorkspaceName,
        paths: &Paths,
        conn: &Connection,
    ) -> Result<Self, TomeError> {
        let workspace_id = resolve_id_required(conn, workspace_name)?;

        // R-M3 (US1.d reviewer pass): select `plugin_version` so the
        // registry can cache it on PromptEntry; pre-US1.d the prompts/get
        // hot path opened a second read-only DB connection per request
        // to look this up.
        let mut stmt = conn
            .prepare(
                "SELECT s.catalog, s.plugin, s.name, s.kind, s.description, s.path, s.indexed_at, s.plugin_version
                 FROM skills AS s
                 JOIN workspace_skills AS ws ON ws.skill_id = s.id
                 WHERE ws.workspace_id = ?1
                   AND s.user_invocable = 1
                 ORDER BY s.catalog, s.plugin, s.kind, s.name",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("prepare prompt-registry query: {e}"))
            })?;

        let rows = stmt
            .query_map(rusqlite::params![workspace_id], |row| {
                Ok(RawRow {
                    catalog: row.get::<_, String>(0)?,
                    plugin: row.get::<_, String>(1)?,
                    name: row.get::<_, String>(2)?,
                    kind_text: row.get::<_, String>(3)?,
                    description: row.get::<_, String>(4)?,
                    path: row.get::<_, String>(5)?,
                    indexed_at: row.get::<_, String>(6)?,
                    plugin_version: row.get::<_, String>(7)?,
                })
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("query prompt-registry rows: {e}"))
            })?;

        let mut identities: Vec<EntryIdentity> = Vec::new();
        let mut hydrated: HashMap<(String, String, EntryKind, String), PromptEntry> =
            HashMap::new();

        for row in rows {
            let row = row.map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("collect prompt-registry row: {e}"))
            })?;

            let kind = match row.kind_text.parse::<EntryKind>() {
                Ok(k) => k,
                Err(msg) => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %row.catalog,
                        plugin = %row.plugin,
                        name = %row.name,
                        reason = %msg,
                        "skipping entry: unknown kind in DB",
                    );
                    continue;
                }
            };

            // Resolve the entry's absolute path via the shared helper —
            // see [`resolve_entry_body_path`] for the catalog manifest
            // walk. Missing plugin dirs / unenrolled catalogs surface as
            // `TomeError`; we collapse to a warn-and-skip here so a
            // single bad entry doesn't take the whole registry build
            // down (mirrors the frontmatter-parse-failure handling
            // below).
            let path = match resolve_entry_body_path(
                conn,
                paths,
                workspace_name.as_str(),
                &row.catalog,
                &row.plugin,
                &row.path,
            ) {
                Ok(p) => p,
                Err(err) => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %row.catalog,
                        plugin = %row.plugin,
                        name = %row.name,
                        reason = %err,
                        "skipping entry: catalog/plugin dir not resolvable on disk",
                    );
                    continue;
                }
            };

            // Re-parse the frontmatter to recover `arguments`,
            // `argument_hint`, `prompt_name` (override), and the body
            // (for the $ARGUMENTS heuristic). The schema doesn't carry
            // these fields directly.
            let parsed = match parse_skill_frontmatter(&path) {
                Ok(p) => p,
                Err(err) => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %row.catalog,
                        plugin = %row.plugin,
                        name = %row.name,
                        path = %path.display(),
                        reason = %err,
                        "skipping entry: frontmatter unreadable on disk",
                    );
                    continue;
                }
            };

            let body_uses_arguments = body_references_arguments(&parsed.body);
            let arguments = parsed.frontmatter.arguments.clone();
            let argument_hint = parsed.frontmatter.argument_hint.clone();
            let override_name = parsed.frontmatter.prompt_name.clone();

            let derived = derive_name(&row.plugin, &row.name, override_name.as_deref());

            identities.push(EntryIdentity {
                catalog: row.catalog.clone(),
                plugin: row.plugin.clone(),
                kind,
                name: row.name.clone(),
                indexed_at: row.indexed_at.clone(),
                derived_name: derived,
            });

            hydrated.insert(
                (
                    row.catalog.clone(),
                    row.plugin.clone(),
                    kind,
                    row.name.clone(),
                ),
                PromptEntry {
                    catalog: row.catalog,
                    plugin: row.plugin,
                    name: row.name,
                    kind,
                    description: truncate_description(&row.description),
                    path,
                    arguments,
                    argument_hint,
                    body_uses_arguments,
                    plugin_version: row.plugin_version,
                },
            );
        }

        let (resolved, collisions) = resolve_collisions(&identities);

        // Surface every collision at warn level per the contract's
        // "Diagnostics" section.
        for record in &collisions {
            warn!(
                target: "tome::mcp::prompts",
                derived_name = %record.base_name,
                count = record.entries.len(),
                entries = ?record.entries,
                "collision_resolved",
            );
        }

        let mut by_name: HashMap<String, PromptEntry> = HashMap::new();
        for (final_name, identity) in resolved {
            let key = (
                identity.catalog,
                identity.plugin,
                identity.kind,
                identity.name,
            );
            if let Some(entry) = hydrated.remove(&key) {
                by_name.insert(final_name, entry);
            }
        }

        Ok(Self {
            by_name,
            collisions,
        })
    }

    /// Look up an entry by its final prompt name.
    pub fn lookup(&self, name: &str) -> Option<&PromptEntry> {
        self.by_name.get(name)
    }

    /// Render every entry as a [`PromptDescriptor`]. Sorted ascending by
    /// name (rmcp's `list_all` re-sorts, but having a stable ordering at
    /// the registry layer simplifies tests + the JSON wire-shape pin).
    pub fn descriptors(&self) -> Vec<PromptDescriptor> {
        let mut out: Vec<PromptDescriptor> = self
            .by_name
            .iter()
            .map(|(name, entry)| entry.descriptor(name.clone()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// Build the per-route `prompts/get` handler closure with the HRTB
/// shape rmcp's [`PromptRoute::new_dyn`] requires. The factory captures
/// the per-route `name` (the resolved prompt name post-collision) +
/// `Arc<McpState>` (paths, scope, prompt registry) so the closure can
/// look up the entry by name, resolve the body, build a
/// [`SubstitutionContext`], and run the substitution pipeline.
///
/// Implementation note: the closure's return type is forwarded through
/// a free async fn (`get_prompt_future`) whose return type is bound to
/// the input lifetime, so the HRTB infers cleanly. An inline closure
/// with `Box::pin(async move { ... })` returns a `'static` future that
/// fails the variance check because `Box` is invariant in its element
/// type.
#[allow(clippy::type_complexity)]
fn make_get_handler<S>(
    name: String,
    state: Arc<McpState>,
) -> impl for<'a> Fn(
    PromptContext<'a, S>,
) -> Pin<Box<dyn Future<Output = Result<PromptGetResponse, McpError>> + Send + 'a>>
+ Send
+ Sync
+ 'static
where
    S: 'static,
{
    move |ctx| get_prompt_future(ctx, name.clone(), state.clone())
}

fn get_prompt_future<'a, S>(
    ctx: PromptContext<'a, S>,
    name: String,
    state: Arc<McpState>,
) -> Pin<Box<dyn Future<Output = Result<PromptGetResponse, McpError>> + Send + 'a>> {
    let arguments = ctx.arguments;
    Box::pin(async move { handle_get(state, name, arguments).await })
}

/// Real `prompts/get` handler. Per `contracts/mcp-prompts.md` § Methods
/// / `prompts/get`:
///
/// 1. Resolve `name` via `state.prompt_registry.lookup`.
/// 2. Use the registry-cached absolute entry body path (resolved via
///    [`resolve_entry_body_path`] at startup; cached on `PromptEntry.path`
///    per R-M4 in US1.d).
/// 3. Re-parse the entry's frontmatter from disk (so `declared_args`
///    and the body content are fresh — the index doesn't carry these).
/// 4. Map caller args → [`ArgumentValues`].
/// 5. Build a [`SubstitutionContext`] (12 built-ins + clock + caller args).
/// 6. Run [`substitution::render`] (F3 stub passes the body through
///    unchanged; US2 + US3 wire real stages).
/// 7. Wrap in [`PromptGetResponse`] (rmcp `GetPromptResult`).
///
/// `#[doc(hidden)] pub` so integration tests can exercise the handler
/// without going through the rmcp `PromptRouter` (which would require
/// a `Server` instance + a synthetic `PromptContext`). Test seam only —
/// production callers go through `build_router` + rmcp's
/// `get_prompt_handler` flow.
#[doc(hidden)]
pub async fn handle_get(
    state: Arc<McpState>,
    name: String,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<PromptGetResponse, McpError> {
    let started = Instant::now();

    // (1) Lookup is cheap (HashMap.get); do it on the runtime thread.
    let Some(entry) = state.prompt_registry.lookup(&name).cloned() else {
        return Err(emit_get_error(
            &name,
            started,
            "prompt_not_found",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("prompt `{name}` not found in this workspace"),
                Some(json!({ "code": "prompt_not_found", "name": name })),
            ),
        ));
    };

    // (2-3-4-5-6) Body resolve + frontmatter re-parse + arg map + render
    // are all synchronous I/O / compute. Run on the blocking pool per the
    // sync-boundary discipline (rusqlite is sync, std::fs is sync, the
    // F3 substitution stub does no I/O but US2+US3 wire create_dir_all).
    let render_state = state.clone();
    let render_entry = entry.clone();
    let render_name = name.clone();
    let render_result = tokio::task::spawn_blocking(move || {
        render_for_get(&render_state, &render_entry, &render_name, arguments)
    })
    .await
    .map_err(|e| internal_get_error(&name, started, format!("render join: {e}")))?;

    let rendered = match render_result {
        Ok(r) => r,
        Err(err) => return Err(emit_tome_error_for_get(&name, started, err)),
    };

    let description = if entry.description.is_empty() {
        None
    } else {
        Some(entry.description.clone())
    };

    info!(
        target: "tome::mcp::prompts",
        prompt = %name,
        catalog = %entry.catalog,
        plugin = %entry.plugin,
        entry_name = %entry.name,
        body_bytes = rendered.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "prompts/get ok",
    );

    let mut result =
        PromptGetResponse::new(vec![PromptMessage::new_text(PromptRole::User, rendered)]);
    if let Some(desc) = description {
        result = result.with_description(desc);
    }
    Ok(result)
}

/// Synchronous body-resolve + frontmatter re-parse + arg map + render.
/// Returns the rendered body or a [`TomeError`] mapped at the caller.
///
/// Closure-style helper rather than a method so it composes cleanly with
/// `tokio::task::spawn_blocking`.
fn render_for_get(
    state: &McpState,
    entry: &PromptEntry,
    prompt_name: &str,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<String, TomeError> {
    // (2) Use the absolute body path cached on the PromptEntry at
    // registry build time. R-M4 (US1.d reviewer pass): pre-US1.d this
    // re-opened a read-only DB connection, re-ran `resolve_entry_body_path`
    // through a lossy `display().to_string()` round-trip, then dropped
    // the connection — for an absolute-path short-circuit the resolver
    // would do anyway. The registry-cached path is honest because the
    // resolver runs at startup and the catalog manifest is the source
    // of truth for the plugin dir (a catalog rebase between startup
    // and prompts/get would need a server restart per the contract).
    let body_path = entry.path.clone();

    // (3) Re-parse the entry's frontmatter from disk to recover
    // `declared_args` + the body content. The `description` cached on
    // `entry.description` is the truncated form for `prompts/list`;
    // the body is what the substitution layer renders.
    //
    // R-M2 + S-L1 (US1.d reviewer pass): map frontmatter parse failure
    // to the dedicated `SkillFrontmatterParseError` (exit 23) — was
    // being stuffed into `EntryNotFound.kind`, breaking the kind
    // discriminator contract (the `kind` field's domain is `"skill"`
    // or `"command"`) and leaking the raw path / `Debug` representation
    // into the error envelope (S-L1).
    let parsed = parse_skill_frontmatter(&body_path).map_err(|err| {
        TomeError::SkillFrontmatterParseError {
            file: body_path.clone(),
            message: err.to_string(),
        }
    })?;
    let declared_args = parsed.frontmatter.arguments.clone();
    let body = parsed.body;

    // (4) Map caller args → ArgumentValues.
    let args = map_caller_arguments(prompt_name, arguments, &declared_args)?;

    // (5) Build the SubstitutionContext.
    let context = build_get_context(state, entry, body_path, declared_args, args)?;

    // (6) Render.
    //
    // R-M1 (US1.d reviewer pass): split the data-dir creation failure
    // mapping by source dir class — plugin data dir → exit 9; workspace
    // data dir → exit 25. The `SubstitutionError` carrier was already
    // split per the substitution-engine contract; the boundary mapping
    // here had been collapsing both into one `TomeError` variant.
    substitution::render(&body, &context).map_err(|e| match e {
        SubstitutionError::PluginDataDirCreationFailed { path, source } => {
            TomeError::PluginDataDirWriteFailed { path, source }
        }
        SubstitutionError::WorkspaceDataDirCreationFailed { path, source } => {
            TomeError::WorkspaceDataDirWriteFailed { path, source }
        }
        SubstitutionError::InvalidArgumentFrontmatter { file, reason } => {
            TomeError::InvalidArgumentFrontmatter { file, reason }
        }
        SubstitutionError::PromptArgumentMismatch { expected, supplied } => {
            TomeError::PromptArgumentMismatch { expected, supplied }
        }
    })
}

/// Construct the [`SubstitutionContext`] for one `prompts/get` call.
///
/// `entry_path` carries the absolute path resolved upstream (saves the
/// caller from threading the path through twice). `entry_dir` is
/// `entry_path.parent()` with a defensive fallback to the plugin root.
fn build_get_context(
    state: &McpState,
    entry: &PromptEntry,
    entry_path: PathBuf,
    declared_args: Vec<String>,
    args: Option<ArgumentValues>,
) -> Result<SubstitutionContext, TomeError> {
    let workspace_name = state.scope.scope.name();
    // The plugin root dir is `entry_path` walked up to the directory
    // that hosts `.claude-plugin/`. For Phase 5 / US1.c we approximate
    // by using `entry_dir`'s grandparent (entries live under
    // `<plugin>/skills/<x>/SKILL.md` or `<plugin>/commands/<x>.md`).
    // Real production callers in US2 will replace this with a manifest-
    // walked plugin_root.
    let entry_dir = entry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry_path.clone());

    // The substitution context wants the plugin's root directory (parent
    // of skills/ or commands/). Walk up from entry_dir defensively.
    let plugin_root_dir = entry_dir
        .ancestors()
        .find(|p| p.join(".claude-plugin").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry_dir.clone());

    let plugin_data_dir = state
        .paths
        .plugin_data_dir_for(&entry.catalog, &entry.plugin);
    let workspace_data_dir =
        state
            .paths
            .workspace_data_dir_for(workspace_name, &entry.catalog, &entry.plugin);

    // `plugin_version` is cached on the PromptEntry at registry build
    // time (R-M3 / US1.d reviewer pass) — pre-US1.d this opened a
    // second read-only DB connection per request to fetch it.
    let plugin_version = entry.plugin_version.clone();

    // `clock`: honour the test-only override slot when set; otherwise
    // local time, falling back to UTC if the host has no local-offset
    // database (musl builds, locked-down sandboxes).
    let clock = current_clock();

    SubstitutionContext::builder()
        .catalog_name(entry.catalog.clone())
        .plugin_name(entry.plugin.clone())
        .plugin_version(plugin_version)
        .entry_name(entry.name.clone())
        .entry_path(entry_path)
        .entry_dir(entry_dir)
        .plugin_root_dir(plugin_root_dir)
        .plugin_data_dir(plugin_data_dir)
        .workspace_name(workspace_name.as_str().to_owned())
        .workspace_data_dir(workspace_data_dir)
        .clock(clock)
        .args(args)
        .declared_args(declared_args)
        .paths(state.paths.clone())
        .build()
        .map_err(|e| TomeError::SubstitutionFailed {
            reason: e.to_string(),
        })
}

/// Resolve the substitution clock — honours
/// `SUBSTITUTION_CLOCK_OVERRIDE` when set, else `now_utc()`. The
/// `time` crate's `now_local()` requires the `local-offset` feature
/// (not enabled in Tome's dep tree); the Phase 5 substitution
/// contract names the clock value as "wall-clock with the local
/// offset *when available*" and the substitution engine produces ISO
/// 8601 with offset, so UTC is a sound default that the test override
/// can replace for deterministic runs.
fn current_clock() -> time::OffsetDateTime {
    use std::sync::Mutex;

    let slot: &std::sync::OnceLock<Mutex<Option<time::OffsetDateTime>>> =
        &substitution::SUBSTITUTION_CLOCK_OVERRIDE;
    if let Some(mu) = slot.get() {
        // Mutex poison recovery per the F3 contract: tests that panic
        // mid-substitution shouldn't take the slot down for the rest of
        // the suite. (Same discipline as Phase 4 / P5 backend recovery.)
        let guard = mu.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = *guard {
            return t;
        }
    }
    time::OffsetDateTime::now_utc()
}

/// Map the rmcp `arguments` JSON object → [`ArgumentValues`] per
/// FR-041–FR-043. The contract distinguishes three caller shapes:
///
/// 1. Entry declares named arguments AND caller supplied object →
///    `ArgumentValues::Object { named, declared_order }`. Any key in
///    `named` that isn't in `declared_args` surfaces as
///    `PromptArgumentMismatch`. Any extra positional / count mismatch
///    surfaces likewise.
/// 2. Entry declares no arguments AND caller supplied object with key
///    `args` → `ArgumentValues::Single(s)` (the catch-all coercion per
///    FR-071).
/// 3. Entry declares no arguments AND caller supplied no args (or empty
///    args object) → `None`.
///
/// All other shapes are `PromptArgumentMismatch`.
fn map_caller_arguments(
    _prompt_name: &str,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
    declared_args: &[String],
) -> Result<Option<ArgumentValues>, TomeError> {
    // No args supplied → None (stage 3 is skipped).
    let Some(arguments) = arguments else {
        return Ok(None);
    };
    if arguments.is_empty() {
        return Ok(None);
    }

    if !declared_args.is_empty() {
        // Case 1: named args. Every key must match a declared name.
        let mut named: std::collections::HashMap<String, String> =
            std::collections::HashMap::with_capacity(declared_args.len());
        for (k, v) in &arguments {
            if !declared_args.iter().any(|d| d == k) {
                return Err(TomeError::PromptArgumentMismatch {
                    expected: declared_args.len(),
                    supplied: arguments.len(),
                });
            }
            named.insert(k.clone(), coerce_value_to_string(v));
        }
        Ok(Some(ArgumentValues::Object {
            named,
            declared_order: declared_args.to_vec(),
        }))
    } else {
        // Case 2: catch-all `args` key. ANY other key fails.
        if arguments.len() == 1
            && let Some(val) = arguments.get("args")
        {
            return Ok(Some(ArgumentValues::Single(coerce_value_to_string(val))));
        }
        Err(TomeError::PromptArgumentMismatch {
            expected: 0,
            supplied: arguments.len(),
        })
    }
}

/// Coerce a JSON value to a string for substitution. Strings pass
/// through; numbers / booleans / null stringify via Display; objects /
/// arrays serialise as compact JSON (uncommon for prompts/get but the
/// MCP spec allows them at the protocol level).
fn coerce_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => v.to_string(),
    }
}

/// Map a [`TomeError`] surfaced by the get pipeline to a
/// [`McpError`] envelope, applying the contract's `data.code` slug per
/// `contracts/mcp-prompts.md` § Error responses.
fn emit_tome_error_for_get(name: &str, started: Instant, err: TomeError) -> McpError {
    match err {
        TomeError::EntryNotFound { .. } => emit_get_error(
            name,
            started,
            "prompt_not_found",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("prompt `{name}`'s body file is missing on disk: {err}"),
                Some(json!({ "code": "prompt_not_found", "name": name })),
            ),
        ),
        TomeError::PromptArgumentMismatch { expected, supplied } => emit_get_error(
            name,
            started,
            "prompt_argument_mismatch",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "prompt `{name}` argument mismatch: expected {expected}, supplied {supplied}"
                ),
                Some(json!({
                    "code": "prompt_argument_mismatch",
                    "name": name,
                    "expected": expected,
                    "supplied": supplied,
                })),
            ),
        ),
        TomeError::WorkspaceDataDirWriteFailed { ref path, .. } => emit_get_error(
            name,
            started,
            "workspace_data_dir_write_failed",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "workspace data dir write failed at {}: {err}",
                    path.display()
                ),
                Some(json!({
                    "code": "workspace_data_dir_write_failed",
                    "name": name,
                    "path": path.display().to_string(),
                })),
            ),
        ),
        TomeError::PluginDataDirWriteFailed { ref path, .. } => emit_get_error(
            name,
            started,
            "plugin_data_dir_write_failed",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("plugin data dir write failed at {}: {err}", path.display()),
                Some(json!({
                    "code": "plugin_data_dir_write_failed",
                    "name": name,
                    "path": path.display().to_string(),
                })),
            ),
        ),
        TomeError::InvalidArgumentFrontmatter { ref file, .. } => emit_get_error(
            name,
            started,
            "invalid_argument_frontmatter",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("invalid argument frontmatter in {}: {err}", file.display()),
                Some(json!({
                    "code": "invalid_argument_frontmatter",
                    "name": name,
                    "file": file.display().to_string(),
                })),
            ),
        ),
        // R-M2 (US1.d reviewer pass): frontmatter parse failure during
        // prompts/get → INVALID_PARAMS / skill_frontmatter_parse_error.
        // Pre-US1.d this was stuffed into EntryNotFound.kind, breaking
        // the kind-discriminator contract.
        TomeError::SkillFrontmatterParseError { ref file, .. } => emit_get_error(
            name,
            started,
            "skill_frontmatter_parse_error",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "skill frontmatter parse failed in {}: {err}",
                    file.display()
                ),
                Some(json!({
                    "code": "skill_frontmatter_parse_error",
                    "name": name,
                    "file": file.display().to_string(),
                })),
            ),
        ),
        // Everything else (including SubstitutionFailed) maps to
        // INTERNAL_ERROR / substitution_failed per the contract's
        // catch-all row.
        other => internal_get_error(name, started, other.to_string()),
    }
}

/// Build an `internal_error` envelope tagged `substitution_failed`
/// (the contract's catch-all for unexpected pipeline failures).
fn internal_get_error(name: &str, started: Instant, msg: String) -> McpError {
    // FR-M-LOG-1 carry-over: scrub error chains in case a downstream
    // wrapped a reqwest / git error.
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::prompts",
        prompt = %name,
        error_code = "substitution_failed",
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "prompts/get error",
    );
    McpError::new(
        ErrorCode::INTERNAL_ERROR,
        msg,
        Some(json!({ "code": "substitution_failed", "name": name })),
    )
}

/// Log the error code, then return the caller's pre-built `McpError`
/// unchanged. Mirrors `get_skill`'s `emit_error` pattern.
fn emit_get_error(name: &str, started: Instant, code: &str, err: McpError) -> McpError {
    info!(
        target: "tome::mcp::prompts",
        prompt = %name,
        result = code,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "prompts/get",
    );
    err
}

/// Internal raw-row representation as pulled from SQLite. Kept private
/// to this module so the public types in [`PromptEntry`] / [`PromptRegistry`]
/// don't leak rusqlite shapes.
struct RawRow {
    catalog: String,
    plugin: String,
    name: String,
    kind_text: String,
    description: String,
    path: String,
    indexed_at: String,
    plugin_version: String,
}

/// Helper: build the rmcp [`PromptRouter`] from a [`PromptRegistry`]
/// and the shared [`McpState`]. Every entry gets a
/// [`PromptRoute::new_dyn`] handler that resolves the body, builds a
/// [`SubstitutionContext`], runs the substitution pipeline, and wraps
/// in a [`PromptGetResponse`].
///
/// The closure must satisfy rmcp's `new_dyn` bound:
///
/// ```text
/// for<'a> Fn(PromptContext<'a, S>)
///     -> Pin<Box<dyn Future<Output = ...> + Send + 'a>>
/// ```
///
/// Closures with a captured `String` don't pick up the HRTB binding via
/// inference (the inferred return type drops `'a` and produces
/// `'static`, which then fails the variance check because `Box<dyn
/// Trait + 'static>` is not a subtype of `Box<dyn Trait + 'a>`). We
/// route through a free async fn (`get_prompt_future`) bound to the
/// input lifetime so the HRTB infers cleanly.
pub fn build_router<S>(registry: &PromptRegistry, state: Arc<McpState>) -> PromptRouter<S>
where
    S: rmcp::service::MaybeSend + 'static,
{
    let mut router: PromptRouter<S> = PromptRouter::new();
    for descriptor in registry.descriptors() {
        let name_for_handler = descriptor.name.clone();
        let handler = make_get_handler::<S>(name_for_handler, state.clone());
        router.add_route(PromptRoute::new_dyn(descriptor, handler));
    }
    router
}

// --- Re-use a record in lookup ergonomics ---------------------------------

/// Convenience builder: take a [`SkillRecord`] (as read by the existing
/// `skills::find` helpers) and produce the registry-side [`EntryIdentity`]
/// + a partially-populated [`PromptEntry`] shell. The frontmatter-derived
/// fields (`arguments` / `argument_hint` / `prompt_name` override) still
/// need the disk-read pass — this helper just relocates the column-to-
/// field copy out of [`PromptRegistry::build_for_workspace`] for future
/// reuse.
///
/// Currently unused by the registry build path (which goes straight from
/// rusqlite rows to identities), but exposed for tests that want to
/// fabricate a registry from a hand-rolled `SkillRecord` set.
#[doc(hidden)]
pub fn _entry_identity_from_record(record: &SkillRecord) -> EntryIdentity {
    EntryIdentity {
        catalog: record.catalog.clone(),
        plugin: record.plugin.clone(),
        kind: record.kind,
        name: record.name.clone(),
        indexed_at: record.indexed_at.clone(),
        derived_name: derive_name(&record.plugin, &record.name, None),
    }
}
