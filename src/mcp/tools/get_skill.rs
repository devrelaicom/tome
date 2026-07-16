//! `get_skill` MCP tool — input/output schemas + handler.
//!
//! Consolidated (issue #497): a single `get_skill` tool with a
//! `metadata_only: bool` flag replaces the historical `get_skill` +
//! `get_skill_info` pair.
//!
//! * `metadata_only: false` (default) — the full-body fetch: the SKILL.md /
//!   command body (frontmatter stripped, substitution-rendered unless `raw`),
//!   the flat list of sibling-resource paths, and (with
//!   `include_resource_bodies`) the byte-capped inlined resource contents.
//! * `metadata_only: true` — the middle-tier introspection: full description +
//!   `when_to_use` guidance + `plugin_version` + `user_invocable` + a capped
//!   resource enumeration, WITHOUT reading or rendering the body.
//!
//! Both modes share the same lookup: the `kind` disambiguator, the `*` wildcard
//! `name` resolution, and the `available: [...]` / `candidates: [...]` payloads
//! on the not-found / ambiguous-glob error envelopes (all inherited from the
//! former `get_skill_info`).
//!
//! Contract: [`mcp-tools.md` §get_skill](../../../specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::error::{ErrorCategory, TomeError};
use crate::index::skills;
use crate::mcp::state::McpState;
use crate::mcp::tools::common::{error_data, error_data_with_code};
use crate::plugin::frontmatter;
use crate::plugin::identity::EntryKind;
use crate::substitution::{self, SubstitutionContext, SubstitutionError};

/// Resource enumeration cap (metadata-only mode). Top-level files and each
/// subdirectory's listing are clipped to this many entries; the overflow is
/// collapsed into the sentinel `"and N more"` string appended to the array.
/// (Inherited from the former `get_skill_info`.)
const PER_DIRECTORY_CAP: usize = 5;

/// The tool description per `mcp-tools.md` §get_skill lives on the
/// `#[tool]`-decorated method in `mcp::server` as a doc comment.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// The catalog name (e.g. `midnight-expert`). Provide the full
    /// `catalog`+`plugin`+`name` triple, OR `uri` — not both.
    #[serde(default)]
    pub catalog: Option<String>,
    /// The plugin name within the catalog (e.g. `compact-core`). Part of the
    /// full triple; omit when using `uri`.
    #[serde(default)]
    pub plugin: Option<String>,
    /// The entry `name` as returned by `search_skills`. Part of the full
    /// triple; omit when using `uri`. (The `*` wildcard still applies on the
    /// triple path.)
    #[serde(default)]
    pub name: Option<String>,
    /// A loose identifier resolved to an indexed entry: an absolute/relative
    /// path to a `SKILL.md` (or its directory), a `<plugin>:<skill>` or
    /// `<catalog>:<plugin>:<skill>` name (delimiter `:`, `_`, or `__`), or a
    /// bare entry name. Ambiguous URIs return `matches` + `next_actions`
    /// instead of a body. Provide EITHER `uri` OR the full triple.
    #[serde(default)]
    pub uri: Option<String>,
    /// Disambiguator. On the triple path: `skill` (default) | `command` |
    /// `agent`. On the `uri` path: when set, filters matches to that kind;
    /// when omitted, both `skill` and `command` are eligible.
    #[serde(default)]
    pub kind: Option<EntryKind>,
    /// Return metadata ONLY — description, `when_to_use`, resource listing,
    /// kind, version, and `user_invocable` — without fetching or rendering the
    /// entry body. This is the cheap middle tier between `search_skills`
    /// (ranked discovery) and the default full-body fetch. Default false.
    ///
    /// The first line is a clean, complete summary because schemars lifts it
    /// into the generated JSON-schema `title` that `tools/list` shows to an
    /// agent.
    ///
    /// When `true` the body-mode inputs `raw` / `include_resource_bodies` are
    /// ignored (no body is read), and the response carries the metadata shape
    /// (`description` / `when_to_use` / `resources` enumeration) rather than
    /// `content`. `#[serde(default)]` keeps existing callers (who omit
    /// `metadata_only`) parsing to `false` under `deny_unknown_fields`.
    #[serde(default)]
    pub metadata_only: bool,
    /// Return the body WITHOUT running the substitution pipeline — literal
    /// `${TOME_*}` tokens are preserved. Use this when authoring/converting a
    /// skill (you want the source tokens, not the resolved values). Default
    /// false (substitutions are applied). Ignored when `metadata_only` is true.
    /// (#331)
    ///
    /// The first line is a clean, complete summary because schemars lifts it
    /// into the generated JSON-schema `title` that `tools/list` shows to an
    /// agent — the `#331` reference is deliberately kept off the opening line.
    ///
    /// `#[serde(default)]` keeps existing callers (who omit `raw`) parsing
    /// to `false` under `deny_unknown_fields`, preserving current behavior.
    #[serde(default)]
    pub raw: bool,
    /// Inline the contents of small text resource files alongside their paths,
    /// so the agent avoids an N+1 file read per resource (and works even when
    /// the host's file tool can't reach a path). Default false. Ignored when
    /// `metadata_only` is true.
    ///
    /// The first line is a clean, complete summary because schemars lifts it
    /// into the generated JSON-schema `title` that `tools/list` shows to an
    /// agent — the `#333` reference is deliberately kept off the opening line.
    ///
    /// When `true`, each enumerated resource that is valid UTF-8 text and fits
    /// the per-file + total byte budgets is returned in `resource_bodies` as
    /// `{ path, content }`. Binary, over-cap, or budget-exceeding resources are
    /// skipped (their paths still appear in `resources` for the agent to fetch
    /// itself), so a hostile catalog cannot blow up the response. Only skills
    /// have a resource directory — commands leave `resource_bodies` absent.
    /// `#[serde(default)]` keeps existing callers parsing under
    /// `deny_unknown_fields`. (#333)
    #[serde(default)]
    pub include_resource_bodies: bool,
}

/// A validated request: exactly one of the triple form or the uri form.
#[derive(Debug)]
pub enum Request {
    Triple {
        catalog: String,
        plugin: String,
        name: String,
        kind: EntryKind,
    },
    Uri {
        uri: String,
        kinds: Vec<EntryKind>,
    },
}

impl Input {
    /// Terse constructor for the full-triple form (tests + triple callers).
    pub fn triple(
        catalog: impl Into<String>,
        plugin: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            catalog: Some(catalog.into()),
            plugin: Some(plugin.into()),
            name: Some(name.into()),
            uri: None,
            kind: None,
            metadata_only: false,
            raw: false,
            include_resource_bodies: false,
        }
    }

    /// Terse constructor for the uri form.
    pub fn for_uri(uri: impl Into<String>) -> Self {
        Self {
            catalog: None,
            plugin: None,
            name: None,
            uri: Some(uri.into()),
            kind: None,
            metadata_only: false,
            raw: false,
            include_resource_bodies: false,
        }
    }

    /// Validate the input into a `Request`. Exactly one of { full triple } or
    /// { uri } must be present.
    pub fn into_request(&self) -> Result<Request, McpError> {
        let has_triple = self.catalog.is_some() || self.plugin.is_some() || self.name.is_some();
        let full_triple = self.catalog.is_some() && self.plugin.is_some() && self.name.is_some();
        let has_uri = self.uri.is_some();

        match (has_uri, has_triple) {
            (true, true) => Err(McpError::invalid_params(
                "provide EITHER `uri` OR the full `catalog`+`plugin`+`name` triple, not both",
                None,
            )),
            (false, false) => Err(McpError::invalid_params(
                "provide `uri`, or the full `catalog`+`plugin`+`name` triple",
                None,
            )),
            (true, false) => {
                let uri = self.uri.clone().unwrap_or_default();
                if uri.trim().is_empty() {
                    return Err(McpError::invalid_params("`uri` must be non-empty", None));
                }
                let kinds = match self.kind {
                    Some(k) => vec![k],
                    None => vec![EntryKind::Skill, EntryKind::Command],
                };
                Ok(Request::Uri { uri, kinds })
            }
            (false, true) => {
                if !full_triple {
                    return Err(McpError::invalid_params(
                        "the triple form needs all of `catalog`, `plugin`, and `name`",
                        None,
                    ));
                }
                let catalog = self.catalog.clone().unwrap();
                let plugin = self.plugin.clone().unwrap();
                let name = self.name.clone().unwrap();
                // Pre-Task-5 behaviour, preserved byte-for-byte: an empty (but
                // present) triple field is rejected with the same message the
                // old `handle()`-top guard used, before it is ever handed to
                // `lookup_entry` (which would otherwise surface a confusing
                // downstream `unknown_catalog` / DB-open error instead).
                if catalog.is_empty() || plugin.is_empty() || name.is_empty() {
                    return Err(McpError::invalid_params(
                        "catalog, plugin, and name must be non-empty",
                        None,
                    ));
                }
                Ok(Request::Triple {
                    catalog,
                    plugin,
                    name,
                    kind: self.kind.unwrap_or(EntryKind::Skill),
                })
            }
        }
    }
}

/// #333: one inlined resource — the absolute `path` (identical to the entry in
/// `Output.resources`) plus its UTF-8 `content`. Emitted only for the subset of
/// resources that fit the per-file + whole-response byte budgets when the caller
/// passes `include_resource_bodies: true`. An emit-only record (no
/// `deny_unknown_fields`; the boundary is inputs).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceBody {
    /// Absolute path of the resource file — matches an entry in `resources`.
    pub path: String,
    /// The resource's UTF-8 text content, verbatim (no substitution, no
    /// normalisation).
    pub content: String,
}

/// Per-entry resource enumeration for the metadata-only mode (inherited from
/// the former `get_skill_info`). `files` carries top-level files in the entry's
/// parent directory (excluding the entry body itself); `directories` carries
/// each immediate subdirectory keyed by name with the alphabetised list of
/// children. Both axes are capped at [`PER_DIRECTORY_CAP`] entries; overflow
/// collapses into the sentinel string `"and {N} more"` appended to the array.
///
/// The `directories` map uses [`BTreeMap`] so JSON serialisation produces
/// alphabetical key order — the contract pins this for byte-stability.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceEnumeration {
    pub files: Vec<String>,
    pub directories: BTreeMap<String, Vec<String>>,
}

/// The consolidated `get_skill` response.
///
/// The shape is mode-dependent, but every mode-specific field is
/// `skip_serializing_if`-gated so the two wire shapes stay clean:
///
/// * Full-body mode (`metadata_only: false`) — `content`,
///   `resources_paths` (serialised as `resources`), `substitutions_applied`,
///   and optionally `resource_bodies`. This is byte-identical to the pre-#497
///   `get_skill` output.
/// * Metadata-only mode (`metadata_only: true`) — `description`, `when_to_use`,
///   `plugin_version`, `user_invocable`, and (for skills) `resources`
///   enumeration. This is byte-identical to the pre-#497 `get_skill_info`
///   output modulo the fixed field ORDER (both surfaces now serialise through
///   this one struct).
///
/// The always-present fields (`catalog` / `plugin` / `name` / `kind` / `path` /
/// `prompt_name`) are shared by both modes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// The entry's catalog. Absent in the multi-match case (`matches` /
    /// `next_actions` carry per-candidate identity instead).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    /// The entry's plugin. Absent in the multi-match case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// The RESOLVED concrete entry name (equals the input `name` for an exact
    /// match; the matched entry's real name for a `*` wildcard). Absent in
    /// the multi-match case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The resolved entry kind (`skill` | `command`). Absent in the
    /// multi-match case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<EntryKind>,
    /// Absolute path to the entry body file (a skill's `SKILL.md` or a
    /// command's `<name>.md`). Absent in the multi-match case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    // ---- Full-body mode fields (metadata_only == false) --------------------
    /// SKILL.md / command body with YAML frontmatter stripped, then
    /// substitution-rendered (built-ins + env passthrough) — UNLESS the
    /// caller passed `raw: true`, in which case the literal `${TOME_*}`
    /// tokens are preserved and no substitution runs. Absent in metadata-only
    /// mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Absolute paths of every OTHER file in the skill's directory
    /// (recursive). The agent may load any of them via its own
    /// file-reading tools. Empty for command-kind entries. Serialised as
    /// `resources`. Absent in metadata-only mode (which uses the structured
    /// `resources` enumeration below instead).
    #[serde(rename = "resources", skip_serializing_if = "Option::is_none")]
    pub resources_paths: Option<Vec<String>>,
    /// #331: whether the substitution pipeline was applied to `content`.
    /// `false` only when the caller passed `raw: true`. Absent in
    /// metadata-only mode (no body was rendered).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitutions_applied: Option<bool>,
    /// #333: the inlined subset of `resources`, present only when the caller
    /// passed `include_resource_bodies: true` AND at least one resource fit the
    /// byte budgets. Absent otherwise (and always in metadata-only mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_bodies: Option<Vec<ResourceBody>>,

    // ---- Metadata-only mode fields (metadata_only == true) -----------------
    /// Full frontmatter `description` (NOT truncated — that's `search_skills`'
    /// job). Present only in metadata-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional `when_to_use` guidance text. Present only in metadata-only
    /// mode (serialised even when `null` there, matching the former
    /// `get_skill_info` wire shape).
    #[serde(skip_serializing_if = "MetaWhenToUse::is_absent")]
    pub when_to_use: MetaWhenToUse,
    /// The entry's `plugin_version`. Present only in metadata-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_version: Option<String>,
    /// The resolved `user_invocable` flag. Present only in metadata-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_invocable: Option<bool>,
    /// Structured resource enumeration. Present only in metadata-only mode for
    /// skill-kind entries (omitted for commands and in full-body mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceEnumeration>,

    // ---- Shared trailing field ---------------------------------------------
    /// The MCP prompt name this entry is reachable under via `prompts/list` /
    /// `prompts/get` (`<plugin>__<entry>` form, post-override and
    /// post-collision-suffix). Present for any user-invocable entry; absent
    /// otherwise. Resolved from the live `PromptRegistry` (the SSOT). Appended
    /// LAST.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_name: Option<String>,

    // ---- Multi-match `uri` mode fields --------------------------------------
    /// Multi-match previews (present only when a `uri` matched >1 entry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<MatchItem>>,
    /// Exact disambiguating `get_skill` calls, aligned index-for-index with
    /// `matches`. Present only in the multi-match case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_actions: Option<Vec<NextAction>>,
}

/// One entry in a multi-match `uri` response: identity + full description, no
/// body. Serialize-only (the `deny_unknown_fields` guard is inputs-only).
#[derive(Debug, Serialize, JsonSchema)]
pub struct MatchItem {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    pub path: String,
    pub description: String,
}

/// A ready-to-issue `get_skill` call that disambiguates one match.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NextAction {
    pub tool: String,
    pub arguments: NextActionArgs,
}

/// The exact triple + kind for a `NextAction`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NextActionArgs {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
}

/// Tri-state for the metadata-only `when_to_use` field, so it can serialise as
/// `null` (present, no guidance) in metadata mode yet be entirely OMITTED in
/// full-body mode. A plain `Option<String>` cannot distinguish "absent" from
/// "present-but-null"; this three-way enum does, matching the former
/// `get_skill_info` wire shape (which emitted `"when_to_use":null`).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum MetaWhenToUse {
    /// The field is entirely absent (full-body mode) — `skip_serializing_if`
    /// elides it.
    Absent,
    /// Metadata mode with guidance text (`"when_to_use":"..."`).
    Present(String),
    /// Metadata mode without guidance (`"when_to_use":null`).
    Null,
}

impl MetaWhenToUse {
    fn is_absent(&self) -> bool {
        matches!(self, MetaWhenToUse::Absent)
    }
}

/// #333: per-file cap for an inlined resource (64 KiB). A resource whose
/// on-disk size exceeds this is NOT inlined (its path stays in `resources`);
/// bounds the memory a single hostile file can force into the response.
const MAX_INLINE_RESOURCE_BYTES: u64 = 64 * 1024;

/// #333: whole-response cap across ALL inlined resources (1 MiB). Inlining stops
/// once the running total of inlined `content` bytes would exceed this; the
/// remaining resource paths stay in `resources`. Sized so a resource-heavy skill
/// still inlines a useful slice while a hostile catalog (many/large files)
/// cannot blow up the response — the total is hard-bounded regardless of the
/// number of resources.
const MAX_INLINE_TOTAL_BYTES: u64 = 1024 * 1024;

/// Pipeline:
///
/// 1. Validate non-empty `catalog` / `plugin` / `name`.
/// 2. Resolve `(catalog, plugin, kind, name)` — exact or `*` wildcard — to an
///    enabled entry. A miss is classified via the shared
///    [`common::classify_not_found`] SSOT into `unknown_catalog` /
///    `unknown_plugin` / `unknown_skill`; an `unknown_skill` (or zero-match
///    wildcard) carries the enabled `available` list; a multi-match wildcard is
///    an `ambiguous_name` error listing the candidates.
/// 3. `metadata_only: true` → build the metadata response (description +
///    `when_to_use` + resource enumeration; no body read).
///    `metadata_only: false` → read + strip + optionally render the body, walk
///    sibling resources, and optionally inline them.
pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    let hit = match input.into_request()? {
        Request::Triple {
            catalog,
            plugin,
            name,
            kind,
        } => {
            let paths = state.paths.clone();
            let scope = state.scope.scope.clone();
            let lookup_catalog = catalog.clone();
            let lookup_plugin = plugin.clone();
            let lookup_name = name.clone();

            let lookup = tokio::task::spawn_blocking(move || {
                lookup_entry(
                    &paths,
                    &scope,
                    &lookup_catalog,
                    &lookup_plugin,
                    kind,
                    &lookup_name,
                )
            })
            .await
            .map_err(|e| {
                internal(
                    &input,
                    started,
                    format!("lookup join: {e}"),
                    ErrorCategory::Internal,
                )
            })?
            .map_err(|e| {
                // C-L1: best-effort MCP-surface `tome.error` (closed category only),
                // with this session's `calling_harness`. Never alters the returned
                // `McpError`.
                crate::mcp::enqueue_tool_error(&state, e.category());
                internal(&input, started, e.to_string(), e.category())
            })?;

            match lookup {
                LookupOutcome::Found(hit) => hit,
                LookupOutcome::NotFound {
                    which: crate::mcp::tools::common::NotFound::UnknownCatalog,
                    ..
                } => {
                    return Err(emit_error(
                        &input,
                        started,
                        "unknown_catalog",
                        McpError::invalid_params(
                            format!("catalog `{catalog}` is not enabled in the resolved scope"),
                            Some(error_data_with_code(
                                "unknown_catalog",
                                ErrorCategory::EntryNotFound,
                                &[("catalog", json!(catalog))],
                            )),
                        ),
                    ));
                }
                LookupOutcome::NotFound {
                    which: crate::mcp::tools::common::NotFound::UnknownPlugin,
                    ..
                } => {
                    return Err(emit_error(
                        &input,
                        started,
                        "unknown_plugin",
                        McpError::invalid_params(
                            format!(
                                "plugin `{catalog}/{plugin}` is not enabled in the resolved scope"
                            ),
                            Some(error_data_with_code(
                                "unknown_plugin",
                                ErrorCategory::EntryNotFound,
                                &[("catalog", json!(catalog)), ("plugin", json!(plugin))],
                            )),
                        ),
                    ));
                }
                LookupOutcome::NotFound {
                    which: crate::mcp::tools::common::NotFound::UnknownSkill,
                    available,
                } => {
                    // #333: catalog + plugin resolved but the entry name did not (a
                    // mistyped exact name OR a glob that matched zero). ENRICH `data`
                    // with the enabled `(name, kind)` list for `(catalog, plugin)`.
                    return Err(emit_error(
                        &input,
                        started,
                        "unknown_skill",
                        McpError::invalid_params(
                            format!(
                                "skill `{catalog}/{plugin}/{name}` is not enabled in the resolved scope",
                            ),
                            Some(error_data_with_code(
                                "unknown_skill",
                                ErrorCategory::EntryNotFound,
                                &[
                                    ("catalog", json!(catalog)),
                                    ("plugin", json!(plugin)),
                                    ("name", json!(name)),
                                    ("available", available_json(&available)),
                                ],
                            )),
                        ),
                    ));
                }
                LookupOutcome::AmbiguousGlob { candidates } => {
                    return Err(emit_error(
                        &input,
                        started,
                        "ambiguous_name",
                        McpError::invalid_params(
                            format!(
                                "name pattern `{name}` matched {} entries in `{catalog}/{plugin}`; pick one",
                                candidates.len(),
                            ),
                            Some(error_data_with_code(
                                "ambiguous_name",
                                ErrorCategory::EntryNotFound,
                                &[
                                    ("catalog", json!(catalog)),
                                    ("plugin", json!(plugin)),
                                    ("name", json!(name)),
                                    ("candidates", available_json(&candidates)),
                                ],
                            )),
                        ),
                    ));
                }
            }
        }
        Request::Uri { uri, kinds } => {
            let paths = state.paths.clone();
            let scope_name = state.scope.scope.name().as_str().to_owned();
            let uri_for_task = uri.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                crate::mcp::tools::uri_resolver::resolve(&paths, &scope_name, &uri_for_task, &kinds)
            })
            .await
            .map_err(|e| {
                internal(
                    &input,
                    started,
                    format!("uri resolve join: {e}"),
                    ErrorCategory::Internal,
                )
            })?
            .map_err(|e| {
                crate::mcp::enqueue_tool_error(&state, e.category());
                internal(&input, started, e.to_string(), e.category())
            })?;

            match outcome {
                crate::mcp::tools::uri_resolver::ResolveOutcome::One(entry) => LookupHit {
                    catalog: entry.record.catalog.clone(),
                    plugin: entry.record.plugin.clone(),
                    body_path: entry.body_path,
                    kind: entry.record.kind,
                    name: entry.record.name.clone(),
                    description: entry.record.description,
                    when_to_use: entry.record.when_to_use,
                    plugin_version: entry.record.plugin_version,
                    user_invocable: entry.record.user_invocable,
                },
                crate::mcp::tools::uri_resolver::ResolveOutcome::Many(matches) => {
                    return handle_multi_match(input, started, matches).await;
                }
                crate::mcp::tools::uri_resolver::ResolveOutcome::NoMatch { available } => {
                    let available: Vec<AvailableEntry> = available
                        .into_iter()
                        .map(|r| AvailableEntry {
                            name: r.name,
                            kind: r.kind,
                        })
                        .collect();
                    return Err(emit_error(
                        &input,
                        started,
                        "unknown_skill",
                        McpError::invalid_params(
                            format!("uri `{uri}` did not resolve to an enabled entry"),
                            Some(error_data_with_code(
                                "unknown_skill",
                                ErrorCategory::EntryNotFound,
                                &[
                                    ("uri", json!(uri)),
                                    ("available", available_json(&available)),
                                ],
                            )),
                        ),
                    ));
                }
            }
        }
    };

    if input.metadata_only {
        handle_metadata(state, input, started, hit).await
    } else {
        handle_body(state, input, started, hit).await
    }
}

/// Metadata-only path (former `get_skill_info` behaviour): return description +
/// `when_to_use` + resource enumeration, without reading the body.
async fn handle_metadata(
    state: Arc<McpState>,
    input: Input,
    started: Instant,
    hit: LookupHit,
) -> Result<Output, McpError> {
    let LookupHit {
        catalog: hit_catalog,
        plugin: hit_plugin,
        body_path,
        kind: resolved_kind,
        name: resolved_name,
        description,
        when_to_use,
        plugin_version,
        user_invocable,
    } = hit;

    // Per FR-083 the resource enumeration is skill-only.
    let resources = if matches!(resolved_kind, EntryKind::Skill) {
        let body_path_for_walk = body_path.clone();
        let walked = tokio::task::spawn_blocking(move || walk_resources(&body_path_for_walk))
            .await
            .map_err(|e| {
                internal(
                    &input,
                    started,
                    format!("walk join: {e}"),
                    ErrorCategory::Internal,
                )
            })?;
        match walked {
            Ok(r) => Some(r),
            Err(err) => {
                return Err(emit_error(
                    &input,
                    started,
                    "resource_enum_failed",
                    McpError::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("resource enumeration failed: {err}"),
                        Some(error_data_with_code(
                            "resource_enum_failed",
                            ErrorCategory::Io,
                            &[("path", json!(body_path.display().to_string()))],
                        )),
                    ),
                ));
            }
        }
    } else {
        None
    };

    info!(
        target: "tome::mcp::tools::get_skill",
        catalog = hit_catalog,
        plugin = hit_plugin,
        name = resolved_name,
        kind = resolved_kind.as_str(),
        metadata_only = true,
        result = "ok",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    // FR-027/FR-028: `tome.entry_info` for the middle-tier lookup.
    crate::telemetry::emit(crate::telemetry::event::EntryInfo {
        rank: crate::mcp::rank_for(&state, &resolved_name),
        calling_harness: crate::mcp::calling_harness(&state),
    });

    let prompt_name = state
        .prompt_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .prompt_name_for(&hit_catalog, &hit_plugin, resolved_kind, &resolved_name)
        .map(str::to_owned);

    let when_to_use = match when_to_use {
        Some(text) => MetaWhenToUse::Present(text),
        None => MetaWhenToUse::Null,
    };

    Ok(Output {
        catalog: Some(hit_catalog),
        plugin: Some(hit_plugin),
        name: Some(resolved_name),
        kind: Some(resolved_kind),
        path: Some(body_path.display().to_string()),
        // Full-body-mode fields are absent in metadata mode.
        content: None,
        resources_paths: None,
        substitutions_applied: None,
        resource_bodies: None,
        // Metadata-mode fields.
        description: Some(description),
        when_to_use,
        plugin_version: Some(plugin_version),
        user_invocable: Some(user_invocable),
        resources,
        prompt_name,
        matches: None,
        next_actions: None,
    })
}

/// Full-body path (historical `get_skill` behaviour): read + strip + optionally
/// render the body, walk the sibling resources, and optionally inline them.
async fn handle_body(
    state: Arc<McpState>,
    input: Input,
    started: Instant,
    hit: LookupHit,
) -> Result<Output, McpError> {
    let LookupHit {
        catalog: hit_catalog,
        plugin: hit_plugin,
        body_path,
        kind: resolved_kind,
        name: resolved_name,
        plugin_version,
        ..
    } = hit;
    let skill_path = body_path;
    // Capture the version before it is moved into the substitution closure
    // below — the catalog-attributed emit (further down) needs it, and it is a
    // PUBLISHED manifest value (the FR-059 carve-out), not a secret.
    let attributed_plugin_version = plugin_version.clone();

    // #289: only skill-kind entries enumerate sibling resources.
    let read_input = input.clone_for_log();
    let read_path = skill_path.clone();
    let walk_resources = matches!(resolved_kind, EntryKind::Skill);
    let read_result =
        tokio::task::spawn_blocking(move || read_skill_and_resources(&read_path, walk_resources))
            .await
            .map_err(|e| {
                internal(
                    &read_input,
                    started,
                    format!("read join: {e}"),
                    ErrorCategory::Internal,
                )
            })?;

    let body_and_resources = match read_result {
        Ok(v) => v,
        Err(e) => {
            let (category, err) = match e {
                ReadError::SkillFileMissing(p) => (
                    crate::error::ErrorCategory::Io,
                    emit_error(
                        &read_input,
                        started,
                        "skill_file_missing",
                        McpError::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("skill file is missing: {}", p.display()),
                            Some(error_data_with_code(
                                "skill_file_missing",
                                ErrorCategory::Io,
                                &[("path", json!(p.display().to_string()))],
                            )),
                        ),
                    ),
                ),
                ReadError::FrontmatterStripFailed(detail) => (
                    crate::error::ErrorCategory::SkillFrontmatterParseError,
                    emit_error(
                        &read_input,
                        started,
                        "frontmatter_strip_failed",
                        McpError::new(
                            ErrorCode::INTERNAL_ERROR,
                            format!("frontmatter parse failed: {detail}"),
                            Some(error_data_with_code(
                                "frontmatter_strip_failed",
                                ErrorCategory::SkillFrontmatterParseError,
                                &[],
                            )),
                        ),
                    ),
                ),
                ReadError::Io(io) => (
                    crate::error::ErrorCategory::Io,
                    internal(&read_input, started, io.to_string(), ErrorCategory::Io),
                ),
            };
            emit_post_resolution_error_telemetry(
                &state,
                &hit_catalog,
                &hit_plugin,
                &resolved_name,
                attributed_plugin_version.clone(),
                category,
            )
            .await;
            return Err(err);
        }
    };

    let (raw_content, resources) = body_and_resources;

    // #331: `raw: true` returns the frontmatter-stripped body verbatim,
    // skipping the substitution pipeline entirely.
    let (content, substitutions_applied) = if input.raw {
        (raw_content, false)
    } else {
        let ctx_state = state.clone();
        let ctx_catalog = hit_catalog.clone();
        let ctx_plugin = hit_plugin.clone();
        let ctx_skill_path = skill_path.clone();
        let ctx_plugin_version = plugin_version;
        let ctx_resolved_name = resolved_name.clone();
        let rendered_result = tokio::task::spawn_blocking(move || {
            let ctx = build_substitution_context(
                &ctx_state,
                &ctx_catalog,
                &ctx_plugin,
                &ctx_resolved_name,
                &ctx_skill_path,
                ctx_plugin_version,
            )?;
            substitution::render(&raw_content, &ctx).map_err(map_substitution_error)
        })
        .await
        .map_err(|e| {
            internal(
                &input,
                started,
                format!("render join: {e}"),
                ErrorCategory::Internal,
            )
        })?;

        match rendered_result {
            Ok(s) => (s, true),
            Err((code, err)) => {
                let category = substitution_code_to_category(code);
                let err = emit_error(&input, started, code, err);
                emit_post_resolution_error_telemetry(
                    &state,
                    &hit_catalog,
                    &hit_plugin,
                    &resolved_name,
                    attributed_plugin_version.clone(),
                    category,
                )
                .await;
                return Err(err);
            }
        }
    };

    info!(
        target: "tome::mcp::tools::get_skill",
        catalog = hit_catalog.as_str(),
        plugin = hit_plugin.as_str(),
        name = resolved_name,
        kind = resolved_kind.as_str(),
        metadata_only = false,
        result = "ok",
        body_bytes = content.len(),
        resource_count = resources.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    // FR-027/FR-028: `tome.entry_invoked` once the entry body is fetched.
    crate::telemetry::emit(crate::telemetry::event::EntryInvoked {
        entry_kind: resolved_kind.into(),
        rank: crate::mcp::rank_for(&state, &resolved_name),
        calling_harness: crate::mcp::calling_harness(&state),
    });

    // FR-052: attributed `catalog.<id>.entry_invoked`, folded into ONE blocking
    // task (the sync SQLite open+query must not run on the reactor).
    let attribution_scope = state.scope.clone();
    let attribution_catalog = hit_catalog.clone();
    let attribution_entry_name = resolved_name.clone();
    let attribution_plugin_name = hit_plugin.clone();
    let attribution_harness = crate::mcp::calling_harness(&state);
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(catalog_id) =
            crate::telemetry::resolve_attribution(&attribution_scope, &attribution_catalog)
        {
            crate::telemetry::emit(crate::telemetry::event::AttributedEntryInvoked {
                catalog: catalog_id,
                entry_name: attribution_entry_name,
                entry_kind: resolved_kind.into(),
                plugin_name: attribution_plugin_name,
                plugin_version: attributed_plugin_version,
                calling_harness: attribution_harness,
            });
        }
    })
    .await;

    let prompt_name = state
        .prompt_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .prompt_name_for(&hit_catalog, &hit_plugin, resolved_kind, &resolved_name)
        .map(str::to_owned);

    // #333: when the caller opted in AND this is a skill, inline the byte-capped
    // subset of `resources`.
    let resource_bodies =
        if input.include_resource_bodies && matches!(resolved_kind, EntryKind::Skill) {
            let resources_for_inline = resources.clone();
            let bodies =
                tokio::task::spawn_blocking(move || inline_resource_bodies(&resources_for_inline))
                    .await
                    .map_err(|e| {
                        internal(
                            &input,
                            started,
                            format!("inline join: {e}"),
                            ErrorCategory::Internal,
                        )
                    })?;
            Some(bodies)
        } else {
            None
        };

    Ok(Output {
        catalog: Some(hit_catalog),
        plugin: Some(hit_plugin),
        name: Some(resolved_name),
        kind: Some(resolved_kind),
        path: Some(skill_path.display().to_string()),
        content: Some(content),
        resources_paths: Some(resources),
        substitutions_applied: Some(substitutions_applied),
        resource_bodies,
        // Metadata-mode fields are absent in full-body mode.
        description: None,
        when_to_use: MetaWhenToUse::Absent,
        plugin_version: None,
        user_invocable: None,
        resources: None,
        prompt_name,
        matches: None,
        next_actions: None,
    })
}

/// Assemble the multi-match `uri` response: previews + aligned next_actions.
/// No body is read; the body-mode flags (`raw` / `include_resource_bodies`) and
/// `metadata_only` are ignored — a multi-match short-circuits before either
/// mode's tail runs.
async fn handle_multi_match(
    input: Input,
    started: Instant,
    matches: Vec<crate::mcp::tools::uri_resolver::ResolvedEntry>,
) -> Result<Output, McpError> {
    let items: Vec<MatchItem> = matches
        .iter()
        .map(|e| MatchItem {
            catalog: e.record.catalog.clone(),
            plugin: e.record.plugin.clone(),
            name: e.record.name.clone(),
            kind: e.record.kind,
            path: e.body_path.display().to_string(),
            description: e.record.description.clone(),
        })
        .collect();
    let next_actions: Vec<NextAction> = matches
        .iter()
        .map(|e| NextAction {
            tool: "get_skill".to_owned(),
            arguments: NextActionArgs {
                catalog: e.record.catalog.clone(),
                plugin: e.record.plugin.clone(),
                name: e.record.name.clone(),
                kind: e.record.kind,
            },
        })
        .collect();

    info!(
        target: "tome::mcp::tools::get_skill",
        uri = input.uri.as_deref().unwrap_or(""),
        result = "multi_match",
        match_count = items.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(Output {
        catalog: None,
        plugin: None,
        name: None,
        kind: None,
        path: None,
        content: None,
        resources_paths: None,
        substitutions_applied: None,
        resource_bodies: None,
        description: None,
        when_to_use: MetaWhenToUse::Absent,
        plugin_version: None,
        user_invocable: None,
        resources: None,
        prompt_name: None,
        matches: Some(items),
        next_actions: Some(next_actions),
    })
}

/// Serialise an available-entries / candidate list to the error-`data` JSON
/// array `[ { "name": ..., "kind": ... }, ... ]`.
fn available_json(entries: &[AvailableEntry]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|e| json!({ "name": e.name, "kind": e.kind.as_str() }))
            .collect(),
    )
}

/// A resolved entry hit carrying every field both modes need.
struct LookupHit {
    /// The entry's catalog. Sourced from the lookup, not `input` — correct
    /// for both the triple path (equal to `input.catalog`) and the future
    /// `uri` path (where `input.catalog` is absent).
    catalog: String,
    /// The entry's plugin. Same rationale as `catalog`.
    plugin: String,
    body_path: PathBuf,
    kind: EntryKind,
    /// The resolved concrete name. Equals `input.name` for the exact-match
    /// path; for a glob it is the single matched candidate's name.
    name: String,
    description: String,
    when_to_use: Option<String>,
    plugin_version: String,
    user_invocable: bool,
}

/// One `(name, kind)` pair for the available-entries / candidate lists.
///
/// Intentional kind-scope asymmetry (do NOT unify the two uses): the
/// `unknown_skill` `available` list is deliberately kind-AGNOSTIC — it lists
/// EVERY enabled entry the caller could ask for. The `AmbiguousGlob`
/// `candidates` list is deliberately kind-SCOPED — only entries of the
/// requested `kind` that the glob matched.
struct AvailableEntry {
    name: String,
    kind: EntryKind,
}

enum LookupOutcome {
    Found(LookupHit),
    NotFound {
        which: crate::mcp::tools::common::NotFound,
        available: Vec<AvailableEntry>,
    },
    AmbiguousGlob {
        candidates: Vec<AvailableEntry>,
    },
}

/// Build the enabled `(name, kind)` list for `(catalog, plugin)` — every
/// enabled entry of any kind, in `list_for_plugin`'s `(kind, name)` order.
fn available_entries(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<Vec<AvailableEntry>, TomeError> {
    Ok(
        skills::list_for_plugin(conn, workspace_name, catalog, plugin)?
            .into_iter()
            .filter(|row| row.enabled)
            .map(|row| AvailableEntry {
                name: row.name,
                kind: row.kind,
            })
            .collect(),
    )
}

/// Resolve `(catalog, plugin, kind, name)` to an enabled entry.
///
/// * `get_skill` (full-body) historically tried `Skill` then `Command` when no
///   `kind` was passed; the consolidated tool keeps that fallback ONLY for an
///   exact name at the DEFAULT `kind` (Skill), so a caller that hands it a
///   command name returned by `search_skills` still gets the command body. When
///   the caller passes an explicit non-default `kind`, or uses a wildcard, only
///   that `kind` is considered (the former `get_skill_info` semantics).
fn lookup_entry(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::Scope,
    catalog: &str,
    plugin: &str,
    kind: EntryKind,
    name: &str,
) -> Result<LookupOutcome, TomeError> {
    let conn = crate::index::db::open_read_only(&paths.index_db)?;
    let workspace_name = scope.name().as_str();

    // A `name` containing `*` is a glob — resolve it against the enabled
    // entries of the requested `kind` for `(catalog, plugin)`.
    if crate::plugin::selector::is_glob(name) {
        let mut matches: Vec<skills::SkillRecord> =
            skills::list_for_plugin(&conn, workspace_name, catalog, plugin)?
                .into_iter()
                .filter(|row| row.enabled && row.kind == kind)
                .filter(|row| crate::plugin::selector::glob_match(name, &row.name))
                .collect();
        match matches.len() {
            1 => {
                let row = matches.remove(0);
                return Ok(LookupOutcome::Found(hit_from_row(
                    &conn,
                    paths,
                    workspace_name,
                    catalog,
                    plugin,
                    row,
                )?));
            }
            0 => {
                // Fall through to the shared not-found classification below.
            }
            _ => {
                let candidates = matches
                    .into_iter()
                    .map(|row| AvailableEntry {
                        name: row.name,
                        kind: row.kind,
                    })
                    .collect();
                return Ok(LookupOutcome::AmbiguousGlob { candidates });
            }
        }
        let which =
            crate::mcp::tools::common::classify_not_found(&conn, workspace_name, catalog, plugin)?;
        let available = if which == crate::mcp::tools::common::NotFound::UnknownSkill {
            available_entries(&conn, workspace_name, catalog, plugin)?
        } else {
            Vec::new()
        };
        return Ok(LookupOutcome::NotFound { which, available });
    }

    // Exact name. Try the requested kind first; when it is the DEFAULT (Skill),
    // fall back to Command so a command name resolves through the full-body path
    // (the pre-#497 `get_skill` behaviour). An explicit non-default kind is not
    // widened (the former `get_skill_info` exactness).
    let kinds: &[EntryKind] = if kind == EntryKind::Skill {
        &[EntryKind::Skill, EntryKind::Command]
    } else {
        std::slice::from_ref(&kind)
    };
    for &try_kind in kinds {
        if let Some(row) = skills::find(&conn, workspace_name, catalog, plugin, try_kind, name)? {
            if !row.enabled {
                // A disabled row of this kind: keep looking (a sibling kind may
                // be enabled) before collapsing to `unknown_skill` below.
                continue;
            }
            return Ok(LookupOutcome::Found(hit_from_row(
                &conn,
                paths,
                workspace_name,
                catalog,
                plugin,
                row,
            )?));
        }
    }

    let which =
        crate::mcp::tools::common::classify_not_found(&conn, workspace_name, catalog, plugin)?;
    let available = if which == crate::mcp::tools::common::NotFound::UnknownSkill {
        available_entries(&conn, workspace_name, catalog, plugin)?
    } else {
        Vec::new()
    };
    Ok(LookupOutcome::NotFound { which, available })
}

/// Build a [`LookupHit`] from a resolved `SkillRecord`, resolving its stored
/// relative path to an absolute body path.
fn hit_from_row(
    conn: &rusqlite::Connection,
    paths: &crate::paths::Paths,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
    row: skills::SkillRecord,
) -> Result<LookupHit, TomeError> {
    let body_path =
        skills::resolve_entry_body_path(conn, paths, workspace_name, catalog, plugin, &row.path)?;
    Ok(LookupHit {
        catalog: catalog.to_owned(),
        plugin: plugin.to_owned(),
        body_path,
        kind: row.kind,
        name: row.name,
        description: row.description,
        when_to_use: row.when_to_use,
        plugin_version: row.plugin_version,
        user_invocable: row.user_invocable,
    })
}

enum ReadError {
    SkillFileMissing(PathBuf),
    FrontmatterStripFailed(String),
    Io(std::io::Error),
}

/// Read the entry body (frontmatter stripped) and, for skill-kind entries,
/// the sibling resource paths.
fn read_skill_and_resources(
    skill_path: &Path,
    walk_resources: bool,
) -> Result<(String, Vec<String>), ReadError> {
    if !skill_path.is_file() {
        return Err(ReadError::SkillFileMissing(skill_path.to_path_buf()));
    }

    let parsed = frontmatter::parse_skill_frontmatter(skill_path)
        .map_err(|e| ReadError::FrontmatterStripFailed(e.to_string()))?;

    let mut resources: Vec<String> = Vec::new();
    if walk_resources {
        let parent = skill_path
            .parent()
            .ok_or_else(|| ReadError::SkillFileMissing(skill_path.to_path_buf()))?;
        walk_dir(parent, skill_path, &mut resources).map_err(ReadError::Io)?;
        resources.sort();
    }

    Ok((parsed.body, resources))
}

/// FR-S-02: walk the skill's directory tree and collect every file path, but
/// **reject symlinks** outright (a hostile catalog author can commit a symlink
/// to a secret). Defence in depth via `entry.file_type()` (lstat, doesn't
/// follow symlinks) plus an explicit `is_symlink()` skip.
fn walk_dir(dir: &Path, exclude: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
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

/// Enumerate the entry's parent directory for the metadata-only mode (inherited
/// from the former `get_skill_info::walk_resources`).
///
/// - `files`: top-level files in the parent directory, excluding the entry body,
///   alphabetised by basename, capped at [`PER_DIRECTORY_CAP`] + `"and N more"`.
/// - `directories`: immediate subdirectories, each with its immediate children
///   (NOT recursed), same cap + sentinel per subdirectory. [`BTreeMap`]
///   guarantees alphabetical JSON key order.
/// - Symlinks (file or dir) are skipped at every level (FR-S-02).
fn walk_resources(body_path: &Path) -> std::io::Result<ResourceEnumeration> {
    let parent = body_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "entry path `{}` has no parent directory",
                body_path.display()
            ),
        )
    })?;

    let mut files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            subdirs.push(path);
        } else if ft.is_file() && path != body_path {
            files.push(path);
        }
    }

    files.sort_by(|a, b| basename_cmp(a, b));
    subdirs.sort_by(|a, b| basename_cmp(a, b));

    let files_out = clip_and_sentinel(files.iter().map(|p| p.display().to_string()).collect());

    let mut directories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sub in subdirs {
        let name = sub
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| sub.display().to_string());
        let children = list_dir_children(&sub)?;
        directories.insert(name, children);
    }

    Ok(ResourceEnumeration {
        files: files_out,
        directories,
    })
}

/// List one subdirectory's immediate children (files only — recursion is
/// intentionally NOT performed).
fn list_dir_children(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_file() {
            children.push(path);
        }
    }
    children.sort_by(|a, b| basename_cmp(a, b));
    Ok(clip_and_sentinel(
        children.iter().map(|p| p.display().to_string()).collect(),
    ))
}

/// Apply the `"and N more"` sentinel rule: if `items` fits inside
/// [`PER_DIRECTORY_CAP`], return it unchanged; otherwise truncate to the cap and
/// append `"and {N} more"`.
fn clip_and_sentinel(items: Vec<String>) -> Vec<String> {
    if items.len() <= PER_DIRECTORY_CAP {
        return items;
    }
    let omitted = items.len() - PER_DIRECTORY_CAP;
    let mut out: Vec<String> = items.into_iter().take(PER_DIRECTORY_CAP).collect();
    out.push(format!("and {omitted} more"));
    out
}

/// Compare two paths by basename so the sorts above produce the
/// alphabetical-by-name ordering the contract pins.
fn basename_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    let an = a.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let bn = b.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    an.cmp(&bn)
}

/// #333: inline the subset of `resources` whose contents fit the per-file and
/// whole-response byte budgets.
fn inline_resource_bodies(resources: &[String]) -> Vec<ResourceBody> {
    let mut out: Vec<ResourceBody> = Vec::new();
    let mut total: u64 = 0;
    for path in resources {
        let Ok(content) =
            crate::util::bounded_read_to_string(Path::new(path), MAX_INLINE_RESOURCE_BYTES)
        else {
            continue;
        };
        let len = content.len() as u64;
        if total.saturating_add(len) > MAX_INLINE_TOTAL_BYTES {
            continue;
        }
        total = total.saturating_add(len);
        out.push(ResourceBody {
            path: path.clone(),
            content,
        });
    }
    out
}

/// Build the [`SubstitutionContext`] for one full-body `get_skill` call. The
/// entry `name` passed here is the RESOLVED name (for a wildcard, the concrete
/// matched entry, not the pattern).
fn build_substitution_context(
    state: &McpState,
    catalog: &str,
    plugin: &str,
    resolved_name: &str,
    skill_path: &Path,
    plugin_version: String,
) -> Result<SubstitutionContext, (&'static str, McpError)> {
    crate::mcp::substitution_helpers::build_context_for_entry(
        catalog.to_owned(),
        plugin.to_owned(),
        plugin_version,
        resolved_name.to_owned(),
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
                Some(error_data(ErrorCategory::SubstitutionFailed)),
            ),
        )
    })
}

/// Map a [`SubstitutionError`] surfaced by the render pipeline to a
/// (`code`, [`McpError`]) tuple.
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
                Some(error_data_with_code(
                    "plugin_data_dir_write_failed",
                    ErrorCategory::PluginDataDirWriteFailed,
                    &[("path", json!(path.display().to_string()))],
                )),
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
                Some(error_data_with_code(
                    "workspace_data_dir_write_failed",
                    ErrorCategory::WorkspaceDataDirWriteFailed,
                    &[("path", json!(path.display().to_string()))],
                )),
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
                Some(error_data_with_code(
                    "invalid_argument_frontmatter",
                    ErrorCategory::InvalidArgumentFrontmatter,
                    &[("file", json!(file.display().to_string()))],
                )),
            ),
        ),
        SubstitutionError::PromptArgumentMismatch { expected, supplied } => (
            "prompt_argument_mismatch",
            McpError::new(
                ErrorCode::INVALID_PARAMS,
                format!("prompt argument mismatch: expected {expected}, supplied {supplied}"),
                Some(error_data_with_code(
                    "prompt_argument_mismatch",
                    ErrorCategory::PromptArgumentMismatch,
                    &[("expected", json!(expected)), ("supplied", json!(supplied))],
                )),
            ),
        ),
    }
}

/// Map a substitution/context-build `code` slug back to its closed
/// [`ErrorCategory`], for the Co-M1 attributed-error telemetry.
fn substitution_code_to_category(code: &str) -> crate::error::ErrorCategory {
    use crate::error::ErrorCategory;
    match code {
        "plugin_data_dir_write_failed" => ErrorCategory::PluginDataDirWriteFailed,
        "workspace_data_dir_write_failed" => ErrorCategory::WorkspaceDataDirWriteFailed,
        "invalid_argument_frontmatter" => ErrorCategory::InvalidArgumentFrontmatter,
        "prompt_argument_mismatch" => ErrorCategory::PromptArgumentMismatch,
        _ => ErrorCategory::SubstitutionFailed,
    }
}

/// Build the `internal_error` envelope plus an error log event.
fn internal(input: &Input, started: Instant, msg: String, category: ErrorCategory) -> McpError {
    // FR-M-LOG-1: scrub error chains before logging.
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog.as_deref().unwrap_or(""),
        plugin = input.plugin.as_deref().unwrap_or(""),
        name = input.name.as_deref().unwrap_or(""),
        uri = input.uri.as_deref().unwrap_or(""),
        kind = input.kind.map(|k| k.as_str()).unwrap_or("skill"),
        error_code = category.as_str(),
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(error_data(category)))
}

/// Log the error variants the contract recognises, then return the caller's
/// pre-built `McpError` unchanged.
fn emit_error(input: &Input, started: Instant, code: &str, err: McpError) -> McpError {
    info!(
        target: "tome::mcp::tools::get_skill",
        catalog = input.catalog.as_deref().unwrap_or(""),
        plugin = input.plugin.as_deref().unwrap_or(""),
        name = input.name.as_deref().unwrap_or(""),
        uri = input.uri.as_deref().unwrap_or(""),
        kind = input.kind.map(|k| k.as_str()).unwrap_or("skill"),
        result = code,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );
    err
}

/// Co-M1 / FR-052: emit the POST-resolution telemetry for a full-body
/// `get_skill` failure that occurred AFTER the entry row resolved.
///
/// `catalog` / `plugin` / `entry_name` are the RESOLVED identity from the
/// `LookupHit` — not `input.catalog` / `input.plugin` / `input.name`, which
/// are `None` on the `uri` path (the success paths in `handle_body` already
/// attribute off the hit; this brings the error paths in line).
async fn emit_post_resolution_error_telemetry(
    state: &Arc<McpState>,
    catalog: &str,
    plugin: &str,
    entry_name: &str,
    plugin_version: String,
    category: crate::error::ErrorCategory,
) {
    crate::mcp::enqueue_tool_error(state, category);

    let scope = state.scope.clone();
    let catalog = catalog.to_owned();
    let plugin_name = plugin.to_owned();
    let entry_name = entry_name.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        if let Some(catalog_id) = crate::telemetry::resolve_attribution(&scope, &catalog) {
            crate::telemetry::emit(crate::telemetry::event::AttributedError {
                catalog: catalog_id,
                plugin_name,
                entry_name: Some(entry_name),
                error_class: category,
                plugin_version,
            });
        }
    })
    .await;
}

impl Input {
    fn clone_for_log(&self) -> Self {
        Self {
            catalog: self.catalog.clone(),
            plugin: self.plugin.clone(),
            name: self.name.clone(),
            uri: self.uri.clone(),
            kind: self.kind,
            metadata_only: self.metadata_only,
            raw: self.raw,
            include_resource_bodies: self.include_resource_bodies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_under_cap_returns_unchanged() {
        let items: Vec<String> = (0..PER_DIRECTORY_CAP)
            .map(|i| format!("item-{i}"))
            .collect();
        let clipped = clip_and_sentinel(items.clone());
        assert_eq!(clipped, items);
    }

    #[test]
    fn clip_at_cap_returns_unchanged() {
        let items: Vec<String> = (0..PER_DIRECTORY_CAP)
            .map(|i| format!("item-{i}"))
            .collect();
        assert_eq!(items.len(), PER_DIRECTORY_CAP);
        let clipped = clip_and_sentinel(items.clone());
        assert_eq!(clipped, items, "exactly-at-cap must NOT add sentinel");
    }

    #[test]
    fn clip_over_cap_truncates_and_appends_sentinel() {
        let total = PER_DIRECTORY_CAP + 3;
        let items: Vec<String> = (0..total).map(|i| format!("item-{i:02}")).collect();
        let clipped = clip_and_sentinel(items);
        assert_eq!(clipped.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(clipped[PER_DIRECTORY_CAP], "and 3 more");
        for (idx, val) in clipped.iter().take(PER_DIRECTORY_CAP).enumerate() {
            assert_eq!(val, &format!("item-{idx:02}"));
        }
    }

    #[test]
    fn basename_cmp_orders_by_filename_not_full_path() {
        let p1 = PathBuf::from("/tmp/b/aaa");
        let p2 = PathBuf::from("/tmp/a/zzz");
        assert_eq!(basename_cmp(&p1, &p2), std::cmp::Ordering::Less);
    }

    #[test]
    fn walk_resources_alphabetises_files_and_skips_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        std::fs::write(dir.join("zebra.txt"), "z").unwrap();
        std::fs::write(dir.join("apple.txt"), "a").unwrap();
        std::fs::write(dir.join("mango.txt"), "m").unwrap();

        let r = walk_resources(&entry).unwrap();
        assert_eq!(r.files.len(), 3);
        assert!(r.files[0].ends_with("apple.txt"));
        assert!(r.files[1].ends_with("mango.txt"));
        assert!(r.files[2].ends_with("zebra.txt"));
        assert!(r.directories.is_empty());
    }

    #[test]
    fn walk_resources_caps_top_level_files_with_sentinel() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        for i in 0..7 {
            std::fs::write(dir.join(format!("file-{i:02}.txt")), "x").unwrap();
        }
        let r = walk_resources(&entry).unwrap();
        assert_eq!(r.files.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(r.files[PER_DIRECTORY_CAP], "and 2 more");
    }

    #[test]
    fn walk_resources_enumerates_subdirs_alphabetically_with_per_dir_cap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();

        let examples = dir.join("examples");
        std::fs::create_dir(&examples).unwrap();
        std::fs::write(examples.join("basic.ts"), "x").unwrap();
        std::fs::write(examples.join("advanced.ts"), "x").unwrap();

        let scripts = dir.join("scripts");
        std::fs::create_dir(&scripts).unwrap();
        for i in 0..7 {
            std::fs::write(scripts.join(format!("step-{i:02}.sh")), "x").unwrap();
        }

        let r = walk_resources(&entry).unwrap();
        let keys: Vec<&str> = r.directories.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["examples", "scripts"]);

        let examples_out = r.directories.get("examples").unwrap();
        assert_eq!(examples_out.len(), 2);
        assert!(examples_out[0].ends_with("advanced.ts"));
        assert!(examples_out[1].ends_with("basic.ts"));

        let scripts_out = r.directories.get("scripts").unwrap();
        assert_eq!(scripts_out.len(), PER_DIRECTORY_CAP + 1);
        assert_eq!(scripts_out[PER_DIRECTORY_CAP], "and 2 more");
    }

    #[cfg(unix)]
    #[test]
    fn walk_resources_skips_symlinks() {
        let entry_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let dir = entry_tmp.path();
        let entry = dir.join("SKILL.md");
        std::fs::write(&entry, "body").unwrap();
        std::fs::write(dir.join("real.txt"), "r").unwrap();

        let target = target_tmp.path().join("secret.txt");
        std::fs::write(&target, "secret").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("link.txt")).unwrap();

        let r = walk_resources(&entry).unwrap();
        assert_eq!(
            r.files.len(),
            1,
            "symlink must be skipped, got {:?}",
            r.files
        );
        assert!(r.files[0].ends_with("real.txt"));
    }

    /// Full-body Output serialises with `content` + `resources` + `kind` +
    /// `substitutions_applied`, and OMITS every metadata-only field — the wire
    /// shape stays byte-identical to the pre-#497 `get_skill` output.
    #[test]
    fn body_mode_output_omits_metadata_fields() {
        let out = Output {
            catalog: Some("c".into()),
            plugin: Some("p".into()),
            name: Some("s".into()),
            kind: Some(EntryKind::Skill),
            path: Some("/abs/SKILL.md".into()),
            content: Some("body".into()),
            resources_paths: Some(vec!["/abs/a.txt".into()]),
            substitutions_applied: Some(true),
            resource_bodies: None,
            description: None,
            when_to_use: MetaWhenToUse::Absent,
            plugin_version: None,
            user_invocable: None,
            resources: None,
            prompt_name: None,
            matches: None,
            next_actions: None,
        };
        let json = serde_json::to_string(&out).unwrap();
        // Body-mode fields present.
        assert!(json.contains("\"content\":\"body\""), "{json}");
        assert!(json.contains("\"resources\":[\"/abs/a.txt\"]"), "{json}");
        assert!(json.contains("\"substitutions_applied\":true"), "{json}");
        assert!(json.contains("\"kind\":\"skill\""), "{json}");
        // Metadata-mode fields absent.
        for absent in [
            "description",
            "when_to_use",
            "plugin_version",
            "user_invocable",
            "resource_bodies",
            "prompt_name",
        ] {
            assert!(
                !json.contains(&format!("\"{absent}\"")),
                "body-mode JSON must omit `{absent}`: {json}",
            );
        }
    }

    /// Metadata-only Output serialises with `description` + `when_to_use`
    /// (nullable) + `plugin_version` + `user_invocable` + `resources`
    /// enumeration, and OMITS every body-mode field.
    #[test]
    fn metadata_mode_output_omits_body_fields() {
        let out = Output {
            catalog: Some("c".into()),
            plugin: Some("p".into()),
            name: Some("s".into()),
            kind: Some(EntryKind::Skill),
            path: Some("/abs/SKILL.md".into()),
            content: None,
            resources_paths: None,
            substitutions_applied: None,
            resource_bodies: None,
            description: Some("desc".into()),
            when_to_use: MetaWhenToUse::Null,
            plugin_version: Some("1.0.0".into()),
            user_invocable: Some(false),
            resources: Some(ResourceEnumeration {
                files: vec!["/abs/x.txt".into()],
                directories: BTreeMap::new(),
            }),
            prompt_name: None,
            matches: None,
            next_actions: None,
        };
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"description\":\"desc\""), "{json}");
        // `when_to_use` serialises as JSON null (present-but-none).
        assert!(json.contains("\"when_to_use\":null"), "{json}");
        assert!(json.contains("\"plugin_version\":\"1.0.0\""), "{json}");
        assert!(json.contains("\"user_invocable\":false"), "{json}");
        assert!(json.contains("\"resources\":{"), "{json}");
        // Body-mode fields absent.
        assert!(!json.contains("\"content\""), "{json}");
        assert!(!json.contains("\"substitutions_applied\""), "{json}");
        assert!(!json.contains("\"resource_bodies\""), "{json}");
    }

    #[test]
    fn multi_match_output_omits_identity_and_body_fields() {
        let out = Output {
            catalog: None,
            plugin: None,
            name: None,
            kind: None,
            path: None,
            content: None,
            resources_paths: None,
            substitutions_applied: None,
            resource_bodies: None,
            description: None,
            when_to_use: MetaWhenToUse::Absent,
            plugin_version: None,
            user_invocable: None,
            resources: None,
            prompt_name: None,
            matches: Some(vec![MatchItem {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "foo".into(),
                kind: EntryKind::Skill,
                path: "/abs/SKILL.md".into(),
                description: "d".into(),
            }]),
            next_actions: Some(vec![NextAction {
                tool: "get_skill".into(),
                arguments: NextActionArgs {
                    catalog: "acme".into(),
                    plugin: "plug".into(),
                    name: "foo".into(),
                    kind: EntryKind::Skill,
                },
            }]),
        };
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains(r#""matches":[{"catalog":"acme","plugin":"plug","name":"foo","kind":"skill","path":"/abs/SKILL.md","description":"d"}]"#), "{json}");
        assert!(json.contains(r#""next_actions":[{"tool":"get_skill","arguments":{"catalog":"acme","plugin":"plug","name":"foo","kind":"skill"}}]"#), "{json}");
        // Identity + body fields omitted in multi-match.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("catalog"));
        assert!(!obj.contains_key("path"));
        assert!(!obj.contains_key("content"));
    }

    #[test]
    fn into_request_accepts_full_triple() {
        let input = Input::triple("cat", "plug", "skill");
        match input.into_request().unwrap() {
            Request::Triple {
                catalog,
                plugin,
                name,
                kind,
            } => {
                assert_eq!(
                    (catalog.as_str(), plugin.as_str(), name.as_str()),
                    ("cat", "plug", "skill")
                );
                assert_eq!(kind, EntryKind::Skill);
            }
            _ => panic!("expected Triple"),
        }
    }

    #[test]
    fn into_request_accepts_uri_with_both_kinds_by_default() {
        let input = Input::for_uri("plug:skill");
        match input.into_request().unwrap() {
            Request::Uri { uri, kinds } => {
                assert_eq!(uri, "plug:skill");
                assert_eq!(kinds, vec![EntryKind::Skill, EntryKind::Command]);
            }
            _ => panic!("expected Uri"),
        }
    }

    #[test]
    fn into_request_rejects_both_uri_and_triple() {
        let mut input = Input::triple("cat", "plug", "skill");
        input.uri = Some("plug:skill".into());
        assert!(input.into_request().is_err());
    }

    #[test]
    fn into_request_rejects_partial_triple() {
        let mut input = Input::for_uri("x"); // start empty-ish
        input.uri = None;
        input.plugin = Some("plug".into());
        input.name = Some("skill".into()); // catalog missing
        assert!(input.into_request().is_err());
    }

    #[test]
    fn into_request_rejects_neither() {
        let input = Input {
            catalog: None,
            plugin: None,
            name: None,
            uri: None,
            kind: None,
            metadata_only: false,
            raw: false,
            include_resource_bodies: false,
        };
        assert!(input.into_request().is_err());
    }
}
