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
use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::prompt::PromptContext;
use rmcp::handler::server::router::prompt::{PromptRoute, PromptRouter};
use rmcp::model::ErrorCode;
use rusqlite::Connection;
use serde_json::json;
use tracing::{error, info, warn};

use crate::error::{ErrorCategory, TomeError};
use crate::index::skills::{
    agent_name_clash_set, enabled_agents_for_workspace, resolve_entry_body_path,
};
use crate::index::workspaces::resolve_id_required;
use crate::mcp::prompt_collision::{CollisionRecord, EntryIdentity, resolve_collisions};
use crate::mcp::prompt_name::{derive_name, derive_suffixed_name};
use crate::mcp::state::McpState;
use crate::mcp::tools::common::error_data_with_code;
use crate::paths::Paths;
use crate::plugin::frontmatter::{ArgumentSpec, parse_skill_frontmatter};
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
/// append U+2026 (`…`) when truncated. Mirrors the bounded-walk shape
/// US4.d C-2 + Security HIGH fix landed in
/// [`crate::mcp::tools::search_skills::truncate_description`]: walks at
/// most `max + 1` chars regardless of input size (no `chars().count()`
/// over the full input, no `take().collect()` allocation). Runs at
/// registry-build time per entry rather than per request, so the DoS
/// amplifier isn't the same shape as search_skills', but the pattern
/// drift was the real cost — keeping the two truncate sites structurally
/// identical prevents future copy-paste regressions.
fn truncate_description(s: &str) -> String {
    let max = DESCRIPTION_MAX_CHARS;
    if max == 0 {
        return String::new();
    }
    let mut iter = s.char_indices();
    // Walk past `max` chars; if we exhaust the iterator within those,
    // no truncation needed.
    for _ in 0..max {
        if iter.next().is_none() {
            return s.to_owned();
        }
    }
    // Reserve one slot for the ellipsis by truncating at `max - 1`
    // content chars and appending `…`. The contract for this site says
    // the post-truncation string is `DESCRIPTION_MAX_CHARS` chars total
    // (content + ellipsis), DIFFERING from search_skills (which is
    // `max + 1` total: `max` content + ellipsis). The walk above
    // measures the input; once we know it overflows, find the
    // `max - 1`-th char's byte offset for the slice.
    let mut iter = s.char_indices();
    for _ in 0..(max - 1) {
        iter.next();
    }
    let cut = iter.next().map(|(idx, _)| idx).unwrap_or(s.len());
    let mut out = String::with_capacity(cut + '\u{2026}'.len_utf8());
    out.push_str(&s[..cut]);
    out.push('\u{2026}');
    out
}

// --- The catch-all argument's default description --------------------------

/// Default description for the catch-all `args` argument when an entry
/// declares no named arguments and the frontmatter carries no
/// `argument-hint`. Per the contract's "Argument schema derivation —
/// Case B".
const CATCHALL_DEFAULT_DESCRIPTION: &str =
    "Optional free-form input, passed to the entry as a single positional argument named `args`.";

// --- Agent personas (Phase 6 / FR-060–FR-067) -----------------------------

/// The single global, unnamespaced, reserved persona-drop prompt name.
/// Exposed exactly once when `expose_agents_as_personas` is on. See
/// `contracts/agent-personas.md` § `drop-persona`.
pub const DROP_PERSONA_NAME: &str = "drop-persona";

/// The `drop-persona` prompt body, reproduced verbatim from PRD §2.4 /
/// `contracts/agent-personas.md`.
const DROP_PERSONA_BODY: &str =
    "Stop acting as any assumed persona and return to your default behaviour\nand personality.";

/// `description` for a `<name>-persona` prompt. Phrased to make the
/// advisory-state caveat (FR-065) explicit on the `prompts/list` surface:
/// a persona is conversational context, not the isolation a native
/// subagent provides.
fn persona_description(display_name: &str) -> String {
    format!(
        "Assume the `{display_name}` agent persona (advisory conversational context, not enforced configuration — the agent may drift or ignore it; not the isolation a native subagent provides)."
    )
}

/// `description` for the reserved `drop-persona` prompt.
const DROP_PERSONA_DESCRIPTION: &str =
    "Stop acting as any assumed agent persona and return to default behaviour.";

/// `prompts/list` description for a reserved meta-install prompt.
fn meta_install_description(skill_id: &str) -> String {
    format!(
        "Install Tome's `{skill_id}` meta skill into this harness (a native guide that \
         teaches the agent how to use Tome; it persists for future sessions), then follow it."
    )
}

/// `prompts/get` body for a reserved meta-install prompt: a fixed `User`-role
/// instruction to call the `meta` tool, then follow the now-installed skill.
/// The `prompt_name` maps back to the embedded skill that declares it.
fn meta_install_body(prompt_name: &str) -> String {
    let resolved = crate::authoring::meta::all()
        .iter()
        .find(|s| s.prompt_name == Some(prompt_name))
        .map(|s| s.id);
    // In production a `MetaInstall` entry is only registered for a skill that
    // declares this exact `prompt_name`, so the lookup always hits. Catch a
    // future prompt_name↔skill drift in CI rather than silently misdirecting.
    debug_assert!(
        resolved.is_some(),
        "meta-install prompt `{prompt_name}` maps to no embedded skill",
    );
    let skill_id = resolved.unwrap_or("convert-marketplace");
    format!(
        "Install Tome's `{skill_id}` meta skill into this harness, then follow it:\n\n\
         1. Call the `meta` tool with `{{ \"action\": \"install\", \"skill_id\": \"{skill_id}\" }}`.\n\
         2. After it reports success, follow the now-installed `{skill_id}` skill for the rest of \
         the task. It persists on disk, so it is available in future sessions too."
    )
}

/// The role an entry plays on the prompt surface. Phase 5 entries are
/// [`PersonaRole::None`]; Phase 6 adds the two persona shapes that the
/// `prompts/get` path resolves through the template-wrapping branch
/// rather than the command/skill body path (FR-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaRole {
    /// A Phase 5 command/skill prompt — rendered via the unchanged
    /// command/skill body path.
    None,
    /// An agent exposed as a `<name>-persona` prompt — body is
    /// frontmatter-stripped, template-wrapped, then substituted.
    Agent,
    /// The reserved global `drop-persona` prompt — fixed body, no
    /// on-disk file, no substitution.
    Drop,
    /// Phase 9 / US3: a reserved built-in prompt that installs an embedded
    /// meta skill (the `convert-marketplace` skill declares
    /// `add-tome-conversion-skill`). Fixed body that drives the `meta` tool;
    /// no on-disk file, no arguments, no substitution.
    MetaInstall,
}

/// Map an entry's [`PersonaRole`] to the telemetry [`PromptKind`] for the
/// `tome.prompt_invoked` event (FR-027). The two persona shapes ([`Agent`] +
/// the reserved [`Drop`]) are `Persona`; the reserved meta-install built-in is
/// `Builtin`; a Phase 5 command/skill entry-prompt ([`None`]) is `Command`.
///
/// [`Agent`]: PersonaRole::Agent
/// [`Drop`]: PersonaRole::Drop
/// [`None`]: PersonaRole::None
fn prompt_kind_for(role: PersonaRole) -> crate::telemetry::event::PromptKind {
    use crate::telemetry::event::PromptKind;
    match role {
        // A user-invocable command or skill exposed as a prompt. The anonymous
        // stream only distinguishes the three prompt SHAPES, not skill-vs-command
        // (that finer split is the catalog-attributed stream's concern), so both
        // collapse to `Command` here.
        PersonaRole::None => PromptKind::Command,
        PersonaRole::Agent | PersonaRole::Drop => PromptKind::Persona,
        PersonaRole::MetaInstall => PromptKind::Builtin,
    }
}

/// Wrap an agent's (already substitution-applied) body in the
/// role-assumption template, reproduced verbatim from PRD §2.4 /
/// `contracts/agent-personas.md` § "Persona prompt body".
///
/// `display_name` is the agent's frontmatter `name` (else filename stem,
/// read before stripping); `persona_name` is the derived persona slug
/// (`<name>-persona` or `<plugin>-<name>-persona`) used for the wrapping
/// tag. `$ARGUMENTS` is left in place for the Phase 5 substitution
/// pipeline to resolve (Stage 3 + the `ARGUMENTS:` append fallback) —
/// the template is the *only* thing the persona path adds; substitution
/// itself is the shared pipeline (NFR-007, no parallel path).
fn wrap_persona_body(display_name: &str, persona_name: &str, body: &str) -> String {
    format!(
        "Assume the following {display_name} persona until instructed otherwise.\n\n\
         <{persona_name}>\n\
         {body}\n\
         </{persona_name}>\n\n\
         While acting as the {display_name} persona, you must: $ARGUMENTS"
    )
}

/// Does `body` reference the bare `$ARGUMENTS` placeholder? Used only to
/// decide whether the catch-all `args` argument is surfaced in
/// `prompts/list` (per `contracts/mcp-prompts.md § Argument schema
/// derivation` — case B).
///
/// US3.d (R-M1): delegates to `substitution::body_has_bare_arguments`
/// which uses the production regex dispatcher to avoid false positives
/// on `$ARGUMENTS_HELP`, `$ARGUMENTS_SUFFIX`, etc.
fn body_references_arguments(body: &str) -> bool {
    crate::substitution::body_has_bare_arguments(body)
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
    /// Named arguments declared in the entry's frontmatter, each carrying an
    /// optional per-argument description (issue #312).
    pub arguments: Vec<ArgumentSpec>,
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
    /// Phase 6: the persona role of this entry. [`PersonaRole::None`] for
    /// Phase 5 command/skill prompts; the two persona variants take the
    /// template-wrapping `prompts/get` branch (FR-064).
    pub persona: PersonaRole,
    /// Phase 6: the agent's display name (`<Name>` in the persona
    /// template) — frontmatter `name`, else filename stem. Empty for
    /// non-persona entries. For the `Drop` role this is unused.
    pub display_name: String,
}

impl PromptEntry {
    /// Build the rmcp [`PromptDescriptor`] including argument-schema
    /// derivation per FR-070 / FR-071 / FR-072.
    ///
    /// Polish m-4 (Phase 5): parameter named `prompt_name` (not `name`)
    /// to disambiguate from `self.name` (the entry's frontmatter name).
    /// The two diverge: `self.name` is `"fix-issue"`, the parameter is
    /// the registry HashMap key e.g. `"my-plugin__fix-issue2"` — the
    /// post-collision final name advertised to the host.
    pub fn descriptor(&self, prompt_name: String) -> PromptDescriptor {
        // Phase 6 persona paths derive their argument schema directly —
        // they bypass the Phase 5 named/catch-all derivation below.
        match self.persona {
            PersonaRole::Agent => {
                // Case B (catch-all `args`, optional) — the persona
                // template always carries `$ARGUMENTS`.
                let args = vec![
                    PromptArgument::new("args")
                        .with_description(CATCHALL_DEFAULT_DESCRIPTION)
                        .with_required(false),
                ];
                return PromptDescriptor::new(
                    prompt_name,
                    Some(self.description.clone()),
                    Some(args),
                );
            }
            PersonaRole::Drop => {
                // The reserved drop-persona prompt takes no arguments.
                return PromptDescriptor::new(prompt_name, Some(self.description.clone()), None);
            }
            PersonaRole::MetaInstall => {
                // The reserved meta-install prompt takes no arguments.
                return PromptDescriptor::new(prompt_name, Some(self.description.clone()), None);
            }
            PersonaRole::None => {}
        }

        let arguments = if !self.arguments.is_empty() {
            // Case A — named arguments. All required strings per FR-070,
            // declaration order preserved. A per-argument `description`
            // (issue #312) is threaded through when present; when absent the
            // `description` field is omitted from the wire (rmcp's
            // `skip_serializing_if`), keeping name-only args byte-identical.
            Some(
                self.arguments
                    .iter()
                    .map(|spec| {
                        let arg = PromptArgument::new(spec.name.clone()).with_required(true);
                        match &spec.description {
                            Some(desc) => arg.with_description(desc.clone()),
                            None => arg,
                        }
                    })
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

        PromptDescriptor::new(prompt_name, Some(self.description.clone()), arguments)
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
        expose_personas: bool,
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
                    persona: PersonaRole::None,
                    display_name: String::new(),
                },
            );
        }

        // Phase 6 (FR-060–FR-067): when `expose_agents_as_personas` is on
        // (resolved against the server startup scope), append one
        // `<name>-persona` identity per enabled agent plus the reserved
        // `drop-persona`, folding them into the SINGLE Phase 5 collision
        // namespace (FR-066) below. The persona path is parallel to the
        // command/skill query above (agents are `user_invocable = 0`, so
        // they are NOT in that query) — see `contracts/agent-personas.md`
        // § Emission path.
        if expose_personas {
            Self::collect_persona_identities(
                workspace_name,
                paths,
                conn,
                &mut identities,
                &mut hydrated,
            )?;
        }

        // Phase 9 / US3 (FR-027/FR-028): reserved built-in prompts for every
        // embedded meta skill that declares a `prompt_name`. ALWAYS exposed
        // (not gated on personas). Each is seeded with empty
        // `(catalog, plugin, indexed_at)` so it sorts first in its collision
        // bucket and WINS the base name — a colliding plugin entry is
        // counter-suffixed, never the built-in. Same mechanism as
        // `drop-persona`; no on-disk file, fixed `prompts/get` body.
        for skill in crate::authoring::meta::all() {
            let Some(prompt_name) = skill.prompt_name else {
                continue;
            };
            let key = (
                String::new(),
                String::new(),
                EntryKind::Skill,
                prompt_name.to_owned(),
            );
            identities.push(EntryIdentity {
                catalog: String::new(),
                plugin: String::new(),
                kind: EntryKind::Skill,
                name: prompt_name.to_owned(),
                indexed_at: String::new(),
                derived_name: prompt_name.to_owned(),
            });
            hydrated.insert(
                key,
                PromptEntry {
                    catalog: String::new(),
                    plugin: String::new(),
                    name: prompt_name.to_owned(),
                    kind: EntryKind::Skill,
                    description: truncate_description(&meta_install_description(skill.id)),
                    path: PathBuf::new(),
                    arguments: Vec::new(),
                    argument_hint: None,
                    body_uses_arguments: false,
                    plugin_version: String::new(),
                    persona: PersonaRole::MetaInstall,
                    display_name: String::new(),
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
                // FR-004: `resolve_collisions` assigns every final name
                // against one global taken-set, so each name is unique and
                // this insert never overwrites a prior user-invocable entry.
                // The assert guards that invariant against future drift — a
                // silent overwrite here would drop an entry from prompts/list
                // and make it unresolvable on prompts/get.
                debug_assert!(
                    !by_name.contains_key(&final_name),
                    "duplicate final prompt name `{final_name}` — \
                     resolve_collisions must guarantee global uniqueness",
                );
                by_name.insert(final_name, entry);
            }
        }

        Ok(Self {
            by_name,
            collisions,
        })
    }

    /// Append persona identities (one `<name>-persona` per enabled agent
    /// plus the reserved `drop-persona`) into the shared `identities` /
    /// `hydrated` collections so they participate in the SINGLE Phase 5
    /// collision pass over the union namespace (FR-066). Agents are
    /// `user_invocable = 0`, so they are absent from the command/skill
    /// query — this is the parallel persona path (FR-064).
    ///
    /// Name derivation (FR-061): `<name>-persona` normally,
    /// `<plugin>-<name>-persona` only for agents whose `<name>` clashes
    /// across two or more enabled plugins (the FR-072 clash set, reused
    /// via [`crate::index::skills::agent_name_clash_set`]). The clash
    /// prefix is applied HERE, before the collision pass; the Phase 5
    /// counter-suffix backstop fires only on residual collisions.
    ///
    /// R4-1: each agent persona carries the agent's REAL `indexed_at`, so
    /// a `<name>-persona` colliding with a command/skill tie-breaks by
    /// `indexed_at ASC` exactly like any other entry (FR-062). The empty
    /// `indexed_at` seed is reserved for `drop-persona` ONLY.
    ///
    /// `drop-persona` is seeded with an empty `indexed_at` + empty
    /// `(catalog, plugin)` so it sorts first in any collision bucket
    /// (the resolver tie-breaks on `indexed_at ASC` then the identity
    /// tuple). That guarantees the reservation: if a command/skill/
    /// persona derives to `drop-persona`, the OTHER entry is
    /// counter-suffixed and `drop-persona` keeps the base name.
    fn collect_persona_identities(
        workspace_name: &WorkspaceName,
        paths: &Paths,
        conn: &Connection,
        identities: &mut Vec<EntryIdentity>,
        hydrated: &mut HashMap<(String, String, EntryKind, String), PromptEntry>,
    ) -> Result<(), TomeError> {
        let clash_set = agent_name_clash_set(conn, workspace_name.as_str())?;
        let agents = enabled_agents_for_workspace(conn, workspace_name.as_str())?;

        for agent in agents {
            let path = match resolve_entry_body_path(
                conn,
                paths,
                workspace_name.as_str(),
                &agent.catalog,
                &agent.plugin,
                &agent.path,
            ) {
                Ok(p) => p,
                Err(err) => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %agent.catalog,
                        plugin = %agent.plugin,
                        name = %agent.name,
                        reason = %err,
                        "skipping persona: agent dir not resolvable on disk",
                    );
                    continue;
                }
            };

            // Re-parse to recover the display name (frontmatter `name`,
            // read BEFORE stripping; else the row name = filename stem).
            let parsed = match parse_skill_frontmatter(&path) {
                Ok(p) => p,
                Err(err) => {
                    warn!(
                        target: "tome::mcp::prompts",
                        catalog = %agent.catalog,
                        plugin = %agent.plugin,
                        name = %agent.name,
                        path = %path.display(),
                        reason = %err,
                        "skipping persona: agent frontmatter unreadable on disk",
                    );
                    continue;
                }
            };
            let (display_name, _) = parsed.resolved_name(&agent.name);

            // FR-061: clash prefix applied here, before the collision
            // pass. `<plugin>-<name>-persona` for clashing agents,
            // `<name>-persona` otherwise. C4-2: derive through
            // `derive_suffixed_name` so the `-persona` suffix is preserved
            // even when the base is long (the prior whole-override
            // truncation amputated it).
            let base = if clash_set.contains(&agent.name) {
                format!("{}-{}", agent.plugin, agent.name)
            } else {
                agent.name.clone()
            };
            let derived = derive_suffixed_name(&base, "persona");

            // R4-1: carry the agent's REAL `indexed_at` so a colliding
            // `<name>-persona` tie-breaks by `indexed_at ASC` like every
            // other entry (FR-062) — NOT the over-broad empty seed that
            // belongs only to `drop-persona`.
            identities.push(EntryIdentity {
                catalog: agent.catalog.clone(),
                plugin: agent.plugin.clone(),
                kind: EntryKind::Agent,
                name: agent.name.clone(),
                indexed_at: agent.indexed_at.clone(),
                derived_name: derived,
            });

            hydrated.insert(
                (
                    agent.catalog.clone(),
                    agent.plugin.clone(),
                    EntryKind::Agent,
                    agent.name.clone(),
                ),
                PromptEntry {
                    catalog: agent.catalog,
                    plugin: agent.plugin,
                    name: agent.name,
                    kind: EntryKind::Agent,
                    description: truncate_description(&persona_description(&display_name)),
                    path,
                    arguments: Vec::new(),
                    argument_hint: None,
                    body_uses_arguments: true,
                    // C4-1: thread the agent's real plugin version so
                    // `${TOME_PLUGIN_VERSION}` resolves in the persona body.
                    plugin_version: agent.plugin_version,
                    persona: PersonaRole::Agent,
                    display_name,
                },
            );
        }

        // The reserved global `drop-persona` (FR-063), exposed exactly
        // once. Seeded to sort first in any collision bucket so it wins
        // the base name (the reservation). No on-disk file; fixed body.
        let drop_key = (
            String::new(),
            String::new(),
            EntryKind::Agent,
            DROP_PERSONA_NAME.to_owned(),
        );
        identities.push(EntryIdentity {
            catalog: String::new(),
            plugin: String::new(),
            kind: EntryKind::Agent,
            name: DROP_PERSONA_NAME.to_owned(),
            indexed_at: String::new(),
            derived_name: DROP_PERSONA_NAME.to_owned(),
        });
        hydrated.insert(
            drop_key,
            PromptEntry {
                catalog: String::new(),
                plugin: String::new(),
                name: DROP_PERSONA_NAME.to_owned(),
                kind: EntryKind::Agent,
                description: DROP_PERSONA_DESCRIPTION.to_owned(),
                path: PathBuf::new(),
                arguments: Vec::new(),
                argument_hint: None,
                body_uses_arguments: false,
                plugin_version: String::new(),
                persona: PersonaRole::Drop,
                display_name: String::new(),
            },
        );

        Ok(())
    }

    /// Look up an entry by its final prompt name.
    pub fn lookup(&self, name: &str) -> Option<&PromptEntry> {
        self.by_name.get(name)
    }

    /// Reverse lookup (#289): given an entry's identity, return the FINAL
    /// (post-collision-suffixing) prompt name it is registered under, or
    /// `None` when no prompt exists for it.
    ///
    /// This is the SSOT for "what prompt does `search_skills` / `get_skill` /
    /// `get_skill_info` point a caller at for this entry" — it resolves the
    /// override + collision suffix that `prompt_name::derive_name` alone cannot
    /// (the override lives only in frontmatter, the suffix only in this
    /// registry). `None` is the right answer for an entry that is searchable
    /// but NOT `user_invocable` (a command with `user_invocable: false`, or any
    /// skill that opted out of the prompt surface) — such an entry is absent
    /// from `by_name`, so no prompt can be invoked for it.
    ///
    /// Built entries are few (one per enabled user-invocable entry), so the
    /// linear scan is cheap and keeps `by_name` keyed on its load-bearing
    /// final-name → entry mapping rather than adding a second index.
    pub fn prompt_name_for(
        &self,
        catalog: &str,
        plugin: &str,
        kind: EntryKind,
        name: &str,
    ) -> Option<&str> {
        self.by_name
            .iter()
            .find(|(_, e)| {
                e.kind == kind && e.catalog == catalog && e.plugin == plugin && e.name == name
            })
            .map(|(final_name, _)| final_name.as_str())
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
/// 6. Run [`substitution::render`] (built-ins → env → arguments → tail).
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
    // Clone the Arc out of the RwLock so the lookup borrow is not tied
    // to a lock guard that would need to outlive this scope.
    let registry = state
        .prompt_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(entry) = registry.lookup(&name).cloned() else {
        return Err(emit_get_error(
            &name,
            started,
            "prompt_not_found",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("prompt `{name}` not found in this workspace"),
                Some(error_data_with_code(
                    "prompt_not_found",
                    ErrorCategory::EntryNotFound,
                    &[("name", json!(name))],
                )),
            ),
        ));
    };

    // (2-3-4-5-6) Body resolve + frontmatter re-parse + arg map + render
    // are all synchronous I/O / compute. Run on the blocking pool per the
    // sync-boundary discipline (rusqlite is sync, std::fs is sync, and the
    // env-passthrough stage may create_dir_all the plugin data dir).
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

    // FR-027: `tome.prompt_invoked` on a successful `prompts/get` render. The
    // `prompt_kind` is derived from the entry's persona role (the same
    // discriminator `render_for_get` branched on): the two persona shapes are
    // `Persona`, the reserved meta-install built-in is `Builtin`, and a Phase 5
    // command/skill entry-prompt is `Command`. Best-effort enqueue — a sub-ms
    // local append that never blocks the render or flushes.
    crate::telemetry::emit(crate::telemetry::event::PromptInvoked {
        prompt_kind: prompt_kind_for(entry.persona),
        calling_harness: crate::mcp::calling_harness(&state),
    });

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
    // Phase 6: persona paths diverge from the command/skill body path.
    match entry.persona {
        PersonaRole::Drop => {
            // Fixed body, no on-disk file, no substitution, no args.
            return Ok(DROP_PERSONA_BODY.to_owned());
        }
        PersonaRole::MetaInstall => {
            // Fixed body driving the `meta` tool; no on-disk file, no args.
            return Ok(meta_install_body(prompt_name));
        }
        PersonaRole::Agent => {
            return render_persona_for_get(state, entry, prompt_name, arguments);
        }
        PersonaRole::None => {}
    }

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
    // The get-pipeline only needs the argument NAMES (for caller-arg
    // matching + substitution); descriptions are a prompts/list concern.
    let declared_args = parsed.frontmatter.argument_names();
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

/// Render an agent-persona `prompts/get` (FR-062). Strips the agent
/// frontmatter, wraps the body in the role-assumption template, then runs
/// the SAME Phase 5 [`substitution::render`] pipeline — there is NO
/// parallel substitution path (NFR-007). The template embeds
/// `$ARGUMENTS`, so Stage 3 of the pipeline resolves the single catch-all
/// `args`; when the caller supplies input the template doesn't consume,
/// the pipeline's documented `ARGUMENTS:` append fallback applies.
fn render_persona_for_get(
    state: &McpState,
    entry: &PromptEntry,
    prompt_name: &str,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<String, TomeError> {
    let body_path = entry.path.clone();

    // Strip the agent frontmatter — `parsed.body` is the post-strip body.
    let parsed = parse_skill_frontmatter(&body_path).map_err(|err| {
        TomeError::SkillFrontmatterParseError {
            file: body_path.clone(),
            message: err.to_string(),
        }
    })?;

    // Wrap in the role-assumption template (verbatim from the contract).
    // `$ARGUMENTS` is left for the shared pipeline to resolve.
    let wrapped = wrap_persona_body(&entry.display_name, prompt_name, &parsed.body);

    // The persona prompt schema is the single catch-all `args` (Case B),
    // so `declared_args` is empty and caller args map through the
    // catch-all path exactly as a Phase 5 catch-all prompt.
    let declared_args: Vec<String> = Vec::new();
    let args = map_caller_arguments(prompt_name, arguments, &declared_args)?;

    let context = build_get_context(state, entry, body_path, declared_args, args)?;

    substitution::render(&wrapped, &context).map_err(|e| match e {
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
/// Polish M-2 (Phase 5): delegates to the shared
/// [`super::substitution_helpers::build_context_for_entry`] helper.
/// Both `prompts/get` and `get_skill` build the same context shape
/// modulo args + plugin_version source; the helper consolidates the
/// duplication.
fn build_get_context(
    state: &McpState,
    entry: &PromptEntry,
    entry_path: PathBuf,
    declared_args: Vec<String>,
    args: Option<ArgumentValues>,
) -> Result<SubstitutionContext, TomeError> {
    super::substitution_helpers::build_context_for_entry(
        entry.catalog.clone(),
        entry.plugin.clone(),
        entry.plugin_version.clone(),
        entry.name.clone(),
        entry_path,
        state.scope.scope.name(),
        state.scope.project_root.clone(),
        state.paths.clone(),
        args,
        declared_args,
    )
    .map_err(|e| TomeError::SubstitutionFailed {
        reason: e.to_string(),
    })
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
                Some(error_data_with_code(
                    "prompt_not_found",
                    ErrorCategory::EntryNotFound,
                    &[("name", json!(name))],
                )),
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
                Some(error_data_with_code(
                    "prompt_argument_mismatch",
                    ErrorCategory::PromptArgumentMismatch,
                    &[
                        ("name", json!(name)),
                        ("expected", json!(expected)),
                        ("supplied", json!(supplied)),
                    ],
                )),
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
                Some(error_data_with_code(
                    "workspace_data_dir_write_failed",
                    ErrorCategory::WorkspaceDataDirWriteFailed,
                    &[
                        ("name", json!(name)),
                        ("path", json!(path.display().to_string())),
                    ],
                )),
            ),
        ),
        TomeError::PluginDataDirWriteFailed { ref path, .. } => emit_get_error(
            name,
            started,
            "plugin_data_dir_write_failed",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("plugin data dir write failed at {}: {err}", path.display()),
                Some(error_data_with_code(
                    "plugin_data_dir_write_failed",
                    ErrorCategory::PluginDataDirWriteFailed,
                    &[
                        ("name", json!(name)),
                        ("path", json!(path.display().to_string())),
                    ],
                )),
            ),
        ),
        TomeError::InvalidArgumentFrontmatter { ref file, .. } => emit_get_error(
            name,
            started,
            "invalid_argument_frontmatter",
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("invalid argument frontmatter in {}: {err}", file.display()),
                Some(error_data_with_code(
                    "invalid_argument_frontmatter",
                    ErrorCategory::InvalidArgumentFrontmatter,
                    &[
                        ("name", json!(name)),
                        ("file", json!(file.display().to_string())),
                    ],
                )),
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
                Some(error_data_with_code(
                    "skill_frontmatter_parse_error",
                    ErrorCategory::SkillFrontmatterParseError,
                    &[
                        ("name", json!(name)),
                        ("file", json!(file.display().to_string())),
                    ],
                )),
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
        Some(error_data_with_code(
            "substitution_failed",
            ErrorCategory::SubstitutionFailed,
            &[("name", json!(name))],
        )),
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

#[cfg(test)]
mod tests {
    use super::{PersonaRole, prompt_kind_for};
    use crate::telemetry::event::PromptKind;

    #[test]
    fn prompt_kind_for_maps_every_persona_role() {
        // The `prompts/get` telemetry discriminator: command/skill entry-prompts
        // are `Command`, both persona shapes are `Persona`, the reserved
        // meta-install built-in is `Builtin`. Exhaustive so a new `PersonaRole`
        // surfaces as a compile error here.
        assert_eq!(prompt_kind_for(PersonaRole::None), PromptKind::Command);
        assert_eq!(prompt_kind_for(PersonaRole::Agent), PromptKind::Persona);
        assert_eq!(prompt_kind_for(PersonaRole::Drop), PromptKind::Persona);
        assert_eq!(
            prompt_kind_for(PersonaRole::MetaInstall),
            PromptKind::Builtin
        );
    }
}
