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
use std::path::PathBuf;
use std::pin::Pin;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::prompt::PromptContext;
use rmcp::handler::server::router::prompt::{PromptRoute, PromptRouter};
use rmcp::model::ErrorCode;
use rusqlite::Connection;
use serde_json::json;
use tracing::warn;

use crate::catalog::manifest::read_catalog_manifest;
use crate::error::TomeError;
use crate::index::skills::SkillRecord;
use crate::index::workspace_catalogs;
use crate::index::workspaces::resolve_id_required;
use crate::mcp::prompt_collision::{CollisionRecord, EntryIdentity, resolve_collisions};
use crate::mcp::prompt_name::derive_name;
use crate::paths::Paths;
use crate::plugin::frontmatter::parse_skill_frontmatter;
use crate::plugin::identity::EntryKind;
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
    /// Absolute or stored path to the entry file on disk (as recorded in
    /// `skills.path`).
    pub path: PathBuf,
    /// Named arguments declared in the entry's frontmatter.
    pub arguments: Vec<String>,
    /// Frontmatter `argument-hint`, used as the `args` description in
    /// the catch-all case.
    pub argument_hint: Option<String>,
    /// `true` when the entry body contains `$ARGUMENTS` (US1.b
    /// heuristic; US3 will replace this with a real regex parse).
    pub body_uses_arguments: bool,
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

        let mut stmt = conn
            .prepare(
                "SELECT s.catalog, s.plugin, s.name, s.kind, s.description, s.path, s.indexed_at
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
                })
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("query prompt-registry rows: {e}"))
            })?;

        let mut identities: Vec<EntryIdentity> = Vec::new();
        let mut hydrated: HashMap<(String, String, EntryKind, String), PromptEntry> =
            HashMap::new();
        // Per-(catalog, plugin) plugin-dir cache — avoids re-reading
        // each `tome-catalog.toml` for every entry of the same plugin.
        let mut plugin_dirs: HashMap<(String, String), Option<PathBuf>> = HashMap::new();

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

            // Resolve the entry's absolute path: the `path` column
            // stores `<entry-rel-path>` relative to the plugin's
            // top-level dir; the plugin dir lives under
            // `paths.cache_dir_for(catalog.url).join(decl.source)` (per
            // `tome-catalog.toml`) or falls back to
            // `paths.cache_dir_for(catalog.url).join(plugin)` for
            // manifest-less catalogs (matches
            // `lifecycle::resolve_plugin_dir`).
            let plugin_dir = match plugin_dirs
                .entry((row.catalog.clone(), row.plugin.clone()))
                .or_insert_with(|| {
                    resolve_plugin_dir_for_row(
                        conn,
                        paths,
                        workspace_name.as_str(),
                        &row.catalog,
                        &row.plugin,
                    )
                }) {
                Some(p) => p.clone(),
                None => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %row.catalog,
                        plugin = %row.plugin,
                        name = %row.name,
                        "skipping entry: catalog/plugin dir not resolvable on disk",
                    );
                    continue;
                }
            };
            let stored_path = PathBuf::from(&row.path);
            let path = if stored_path.is_absolute() {
                stored_path
            } else {
                plugin_dir.join(&stored_path)
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

/// Resolve the on-disk plugin directory for a `(catalog, plugin)` pair
/// in the supplied workspace. Mirrors `lifecycle::resolve_plugin_dir`
/// but works without a `Config` — the URL comes from the central DB's
/// `workspace_catalogs` table. Returns `None` when the catalog is not
/// enrolled, the plugin can't be located in the catalog manifest, or
/// the plugin directory is missing on disk.
fn resolve_plugin_dir_for_row(
    conn: &Connection,
    paths: &Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Option<PathBuf> {
    let catalog_path =
        workspace_catalogs::resolve_catalog_path(conn, paths, workspace_name, catalog).ok()?;
    let plugin_dir = match read_catalog_manifest(&catalog_path) {
        Some(manifest) => manifest
            .plugins
            .iter()
            .find(|p| p.name == plugin)
            .map(|decl| catalog_path.join(&decl.source))
            .unwrap_or_else(|| catalog_path.join(plugin)),
        None => catalog_path.join(plugin),
    };
    if plugin_dir.is_dir() {
        Some(plugin_dir)
    } else {
        None
    }
}

/// Build a stub `prompts/get` handler closure with the HRTB shape
/// rmcp's [`PromptRoute::new_dyn`] requires. The factory takes the
/// per-route `name` (the captured state) and returns a closure that
/// — for any input lifetime `'a` — produces a
/// `BoxFuture<'a, Result<PromptGetResponse, McpError>>` resolving to a
/// `METHOD_NOT_FOUND` error. US1.c replaces this with the real
/// substitution-pipeline handler.
///
/// Implementation note: the closure's return type is forwarded through
/// a free fn (`stub_future`) whose return type is bound to the input
/// lifetime, so the HRTB infers cleanly. An inline closure with a
/// captured `String` and `Box::pin(async move { ... })` returns a
/// `'static` future that fails the variance check because `Box` is
/// invariant in its element type.
#[allow(clippy::type_complexity)]
fn make_stub_handler<S>(
    name: String,
) -> impl for<'a> Fn(
    PromptContext<'a, S>,
) -> Pin<Box<dyn Future<Output = Result<PromptGetResponse, McpError>> + Send + 'a>>
+ Send
+ Sync
+ 'static
where
    S: 'static,
{
    move |ctx| stub_future(ctx, name.clone())
}

fn stub_future<'a, S>(
    _ctx: PromptContext<'a, S>,
    name: String,
) -> Pin<Box<dyn Future<Output = Result<PromptGetResponse, McpError>> + Send + 'a>> {
    Box::pin(async move {
        Err(McpError::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("prompts/get is not yet implemented for `{name}`"),
            Some(json!({ "code": "method_not_found" })),
        ))
    })
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
}

/// Helper: build the rmcp [`PromptRouter`] from a [`PromptRegistry`].
///
/// Every entry gets a [`PromptRoute::new_dyn`] handler that — for US1.b
/// — returns `METHOD_NOT_FOUND`. US1.c replaces this stub with the
/// substitution-pipeline-driven body render.
pub fn build_router<S>(registry: &PromptRegistry) -> PromptRouter<S>
where
    S: rmcp::service::MaybeSend + 'static,
{
    let mut router: PromptRouter<S> = PromptRouter::new();
    for descriptor in registry.descriptors() {
        let name_for_handler = descriptor.name.clone();
        // The closure must satisfy rmcp's `new_dyn` bound:
        //   for<'a> Fn(PromptContext<'a, S>)
        //       -> MaybeBoxFuture<'a, Result<GetPromptResult, ErrorData>>
        // `MaybeBoxFuture` is a private alias for `BoxFuture` (the
        // non-`local` variant rmcp ships by default). Without taking a
        // direct dep on `futures` we hand-construct the equivalent
        // `Pin<Box<dyn Future + Send>>` ourselves.
        // Build the per-route handler.
        //
        // rmcp's `new_dyn` requires a HRTB closure:
        //   for<'a> Fn(PromptContext<'a, S>)
        //       -> Pin<Box<dyn Future<Output = ...> + Send + 'a>>
        //
        // Closures with a captured `String` don't pick up the HRTB
        // binding via inference (the inferred return type drops `'a`
        // and produces `'static`, which then fails the variance check
        // because `Box<dyn Trait + 'static>` is not a subtype of
        // `Box<dyn Trait + 'a>`). The canonical work-around is to
        // pre-build the boxed future per route, then move it into a
        // fresh `Arc` and have the closure clone the `Arc` and rebox.
        //
        // Cleaner approach: wrap in an `Arc<dyn Fn(...) -> ...>`
        // pattern. We use a small `fn` item that takes the captured
        // state by reference + an erased `&PromptContext` (the route
        // never reads it) to keep the HRTB satisfaction trivial.
        let handler = make_stub_handler::<S>(name_for_handler);
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
