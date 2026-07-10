//! The concrete lint rules (`data-model.md §9`), consumed by both `lint` (the
//! command) and `convert` (which folds the registry over the converted result).
//!
//! Each rule is a [`Rule`] with an `id`/`scope`/`autofixable` and the scope's
//! `check_*` method. The runner ([`super::run`]) calls every rule against every
//! node at its scope and never exits early, so one run reports ALL findings.
//! Rules may read the filesystem (via the IR's `source_path`/`provenance`) for
//! dir-structure checks (`name == dir`, unsupported-component dirs) and to
//! compute autofix bytes; `lint` is I/O-bound and takes no index lock.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{Rule, Scope};
use crate::authoring::ir::{CatalogIr, Diagnostic, EntryIr, Fix, Location, PluginIr};
use crate::authoring::rewrite::{self, RewriteOptions, rewrite_body, rewrite_known_vars};
use crate::catalog::manifest::looks_like_email;
use crate::plugin::identity::{EntryKind, is_kebab, validate_segment};
use crate::util::{ENTRY_BODY_MAX, bounded_read_to_string};

/// Stable lint rule ids (also referenced by [`super::parse`]).
pub mod rule {
    pub const MANIFEST_INVALID: &str = "lint/manifest-invalid";
    pub const ENTRY_INVALID: &str = "lint/entry-invalid";
    pub const NAME_MISSING: &str = "lint/name-missing";
    pub const NAME_INVALID: &str = "lint/name-invalid";
    pub const NAME_NOT_KEBAB: &str = "lint/name-not-kebab";
    pub const VERSION_INVALID: &str = "lint/version-invalid";
    pub const OWNER_MISSING: &str = "lint/owner-missing";
    pub const OWNER_EMAIL_INVALID: &str = "lint/owner-email-invalid";
    pub const DUP_PLUGIN: &str = "lint/duplicate-plugin";
    pub const CATALOG_NAME_MISMATCH: &str = "lint/catalog-name-mismatch";
    pub const CATALOG_PLUGIN_MISSING: &str = "lint/catalog-plugin-missing";
    pub const CATALOG_PLUGIN_INVALID: &str = "lint/catalog-plugin-source-invalid";
    pub const NAME_NOT_DIR: &str = "lint/name-not-dir";
    pub const DESCRIPTION_MISSING: &str = "lint/description-missing";
    pub const DESCRIPTION_TOO_LONG: &str = "lint/description-too-long";
    pub const UNSUPPORTED_COMPONENT: &str = "lint/unsupported-component";
    pub const RESIDUAL_HARNESS_ISM: &str = "lint/residual-harness-ism";
    /// A source path the lint parser refused to read (an escaping or symlinked
    /// component under the artifact root) — reported, never followed.
    pub const UNSAFE_PATH: &str = "lint/unsafe-path";
    /// `hooks/hooks.json` is present but not valid JSON (or unreadable);
    /// `harness sync` would fail on this plugin (exit 43).
    pub const HOOKS_SPEC: &str = "lint/hooks-spec";
    /// `.mcp.json` is present but not valid JSON, or not a JSON object at the
    /// top level; `harness sync` would fail on this plugin.
    pub const MCP_SPEC: &str = "lint/mcp-spec";
    /// An agent entry's frontmatter has a wrong-typed field (e.g. `tools: 7`
    /// instead of a list); `harness sync` would fail on this plugin (exit 45).
    pub const AGENT_SPEC: &str = "lint/agent-spec";
    /// Skill body is too large to fit the harness MCP-output budget that
    /// `get_skill` returns it inside (70% of the limit, after envelope).
    pub const BODY_TOO_LARGE: &str = "lint/body-too-large";
    /// A single text supporting file is too large for the agent to read in
    /// one call.
    pub const RESOURCE_TOO_LARGE: &str = "lint/resource-too-large";
}

/// Agent-Skills description length cap (§9).
const DESCRIPTION_MAX: usize = 1024;

/// Bytes-per-token estimate. ~4 bytes/token for English prose; code and
/// markdown are denser and multi-byte input inflates byte count, so this errs
/// slightly high (more tokens), biasing toward flagging rather than missing.
/// This is an ESTIMATE — there is no tokenizer in the lint path (the only one
/// is the heavyweight llama GGUF tokenizer in `summarise`).
const BYTES_PER_TOKEN_EST: usize = 4;

/// Rough token count for a byte length.
fn est_tokens(bytes: usize) -> usize {
    bytes / BYTES_PER_TOKEN_EST
}

/// Token budgets for the get_skill MCP response. The harness cap is on the
/// FULL response (JSON envelope + `path` + `resources[]` paths + `content`),
/// so we reserve 70% of the limit for the body.
#[derive(Debug, Clone, Copy)]
struct TokenBudgets {
    /// Effective hard limit: `MAX_MCP_OUTPUT_TOKENS` env override, else the
    /// 25_000 default. Shown in messages so a user sees their own number.
    hard_limit: usize,
    /// 70% of `SOFT_LIMIT`, clamped to never exceed `hard_tokens`.
    soft_tokens: usize,
    /// 70% of `hard_limit`.
    hard_tokens: usize,
}

impl TokenBudgets {
    /// Claude Code's MCP-output soft-warning point. Not env-configurable.
    const SOFT_LIMIT: usize = 10_000;
    /// Default `MAX_MCP_OUTPUT_TOKENS`.
    const DEFAULT_HARD_LIMIT: usize = 25_000;
    /// Envelope headroom: keep the body under 70% of the limit.
    const HEADROOM_NUM: usize = 7;
    const HEADROOM_DEN: usize = 10;

    /// Resolve from the environment: 70% of `MAX_MCP_OUTPUT_TOKENS` when set to
    /// a positive integer, else 70% of the 25_000 default.
    fn from_env() -> Self {
        let max = std::env::var("MAX_MCP_OUTPUT_TOKENS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(Self::DEFAULT_HARD_LIMIT);
        Self::from_max(max)
    }

    /// Build budgets from an explicit hard limit (the test seam — no env).
    fn from_max(max: usize) -> Self {
        let hard_tokens = max * Self::HEADROOM_NUM / Self::HEADROOM_DEN;
        let soft_tokens =
            (Self::SOFT_LIMIT * Self::HEADROOM_NUM / Self::HEADROOM_DEN).min(hard_tokens);
        Self {
            hard_limit: max,
            soft_tokens,
            hard_tokens,
        }
    }
}

/// The full rule registry — used by `lint`, where the IR's `source_path` IS the
/// native Tome artifact being validated.
pub fn all() -> Vec<Box<dyn Rule>> {
    let budgets = TokenBudgets::from_env();
    vec![
        Box::new(CatalogManifest),
        Box::new(PluginManifest),
        Box::new(HooksSpec),
        Box::new(McpSpec),
        Box::new(AgentSpec),
        Box::new(UnsupportedComponents),
        Box::new(EntryName),
        Box::new(EntryDescription),
        Box::new(EntryHarnessIsms),
        Box::new(EntryBodyBudget { budgets }),
        Box::new(EntryResourceBudget { budgets }),
    ]
}

/// The registry `convert` runs over a freshly-imported IR. It OMITS the
/// filesystem-structural rules (`UnsupportedComponents`, `EntryName`) because on
/// the convert path the IR's `source_path` still points at the FOREIGN source
/// tree, not the Tome output: `UnsupportedComponents` would re-flag the same
/// `monitors/`/`themes/` dirs the importer already reported as
/// `convert/unsupported-component` (a double-finding under two rule ids), and
/// `EntryName`'s `expected`/autofix path would be source-relative and meaningless
/// (convert already enforces `name == dir` in the emitter). `lint` keeps
/// [`all`]; only the convert pre-emit pass uses this subset.
pub fn for_convert() -> Vec<Box<dyn Rule>> {
    let budgets = TokenBudgets::from_env();
    vec![
        Box::new(CatalogManifest),
        Box::new(PluginManifest),
        Box::new(HooksSpec),
        Box::new(McpSpec),
        Box::new(AgentSpec),
        Box::new(EntryDescription),
        Box::new(EntryHarnessIsms),
        Box::new(EntryBodyBudget { budgets }),
    ]
}

// --- catalog ----------------------------------------------------------------

struct CatalogManifest;
impl Rule for CatalogManifest {
    fn id(&self) -> &'static str {
        "lint/catalog-manifest"
    }
    fn scope(&self) -> Scope {
        Scope::Catalog
    }
    fn check_catalog(&self, c: &CatalogIr) -> Vec<Diagnostic> {
        let mut d = Vec::new();
        check_name(&c.name, "catalog", &mut d);
        check_version(&c.version, &mut d);
        if c.owner.name.trim().is_empty() && c.owner.email.trim().is_empty() {
            d.push(Diagnostic::warning(
                rule::OWNER_MISSING,
                "catalog `owner` has no name or email",
            ));
        }
        if !c.owner.email.trim().is_empty() && !looks_like_email(&c.owner.email) {
            d.push(Diagnostic::warning(
                rule::OWNER_EMAIL_INVALID,
                format!(
                    "catalog owner email `{}` is not a valid address",
                    c.owner.email
                ),
            ));
        }
        let mut seen = HashSet::new();
        for p in &c.plugins {
            if !seen.insert(p.name.as_str()) {
                d.push(Diagnostic::warning(
                    rule::DUP_PLUGIN,
                    format!("duplicate plugin name `{}` in the catalog", p.name),
                ));
            }
        }
        d
    }
}

// --- plugin -----------------------------------------------------------------

struct PluginManifest;
impl Rule for PluginManifest {
    fn id(&self) -> &'static str {
        "lint/plugin-manifest"
    }
    fn scope(&self) -> Scope {
        Scope::Plugin
    }
    fn check_plugin(&self, p: &PluginIr) -> Vec<Diagnostic> {
        let mut d = Vec::new();
        check_name(&p.name, "plugin", &mut d);
        check_version(&p.version, &mut d);
        d
    }
}

/// `hooks/hooks.json`, when present, must be valid JSON — otherwise
/// `harness sync` fails on this plugin at exit 43 with no earlier signal.
/// Content arrives on the IR (`hooks_json`), so the rule is provenance-safe in
/// both registries (it never reads the source tree).
struct HooksSpec;
impl Rule for HooksSpec {
    fn id(&self) -> &'static str {
        rule::HOOKS_SPEC
    }
    fn scope(&self) -> Scope {
        Scope::Plugin
    }
    fn check_plugin(&self, p: &PluginIr) -> Vec<Diagnostic> {
        let Some(json) = &p.hooks_json else {
            return Vec::new();
        };
        let value = match serde_json::from_str::<serde_json::Value>(json) {
            Ok(v) => v,
            Err(e) => {
                return vec![Diagnostic::warning(
                    rule::HOOKS_SPEC,
                    format!(
                        "hooks/hooks.json is not valid JSON ({e}); `harness sync` will fail on this plugin (exit 43)"
                    ),
                )];
            }
        };
        // After `convert` the importer normalises the wrapped form
        // (`{"hooks":{...}}` → the inner event-map), so the on-disk IR always
        // carries the event-map.  A native plugin whose author wrote the wrapped
        // form directly (without going through `convert`) will still have
        // `hooks_json` set to the raw wrapped text.  Flag it so the author knows
        // `harness sync` would fail (exit 43) on the un-normalised file.
        if value
            .as_object()
            .is_some_and(|obj| obj.get("hooks").is_some_and(|v| v.is_object()))
        {
            return vec![Diagnostic::warning(
                rule::HOOKS_SPEC,
                "hooks/hooks.json uses the wrapped form (top-level \"hooks\" key); \
                 `harness sync` requires the event-map form \
                 ({\"PreToolUse\": [...], ...}). \
                 Run `tome catalog convert` to fix."
                    .to_owned(),
            )];
        }
        // After checking the wrapped form, normalise the object (simple inline
        // unwrap: we already know the "hooks" discriminator is absent or
        // non-object) so we can validate the event-map values. `harness sync`
        // deserialises with `HashMap<String, Vec<HookEntry>>` — a non-array
        // value for any event key causes a parse failure there.
        let event_map = match value.as_object() {
            Some(obj) => obj,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for (key, val) in event_map {
            if !val.is_array() {
                out.push(Diagnostic::warning(
                    rule::HOOKS_SPEC,
                    format!(
                        "hooks event map value for '{}' must be an array (sync would fail with exit 43)",
                        key
                    ),
                ));
            }
        }
        out
    }
}

/// `.mcp.json`, when present, must be valid JSON and a top-level object —
/// otherwise `harness sync` would fail. Content arrives on the IR (`mcp_json`),
/// so the rule is provenance-safe in both registries (it never reads the source
/// tree).
struct McpSpec;
impl Rule for McpSpec {
    fn id(&self) -> &'static str {
        rule::MCP_SPEC
    }
    fn scope(&self) -> Scope {
        Scope::Plugin
    }
    fn check_plugin(&self, p: &PluginIr) -> Vec<Diagnostic> {
        let Some(json) = &p.mcp_json else {
            return Vec::new();
        };
        let value = match serde_json::from_str::<serde_json::Value>(json) {
            Ok(v) => v,
            Err(e) => {
                return vec![Diagnostic::warning(
                    rule::MCP_SPEC,
                    format!(".mcp.json is not valid JSON: {e}"),
                )];
            }
        };
        if !value.is_object() {
            return vec![Diagnostic::warning(
                rule::MCP_SPEC,
                ".mcp.json must be a JSON object at the top level",
            )];
        }
        Vec::new()
    }
}

/// Agent entries' frontmatter must have well-typed fields — otherwise
/// `harness sync` fails with exit 45 when it tries to deserialise
/// `AgentFrontmatter`. Re-reads the source file so it works in both `lint`
/// (source IS the artifact) and `convert` (IR carries the source path).
struct AgentSpec;
impl Rule for AgentSpec {
    fn id(&self) -> &'static str {
        rule::AGENT_SPEC
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        if e.kind != EntryKind::Agent {
            return Vec::new();
        }
        // Re-read the source file for frontmatter validation.
        let raw = match bounded_read_to_string(&e.source_path, ENTRY_BODY_MAX) {
            Ok(s) => s,
            Err(_) => return Vec::new(), // can't validate; parse already warned
        };
        let yaml_block = match extract_frontmatter_yaml(&raw) {
            Some(s) => s,
            None => return Vec::new(), // no frontmatter — nothing to check
        };

        // Deserialise only the agent-specific fields that carry type
        // constraints leniently (no deny_unknown_fields — frontmatter is
        // third-party input). We only validate the fields whose wrong type
        // would cause harness sync to fail; others are ignored.
        #[derive(serde::Deserialize)]
        struct AgentFrontmatterCheck {
            tools: Option<serde_json::Value>,
            #[serde(default, rename = "disallowedTools", alias = "disallowed_tools")]
            disallowed_tools: Option<serde_json::Value>,
        }

        let fm = match serde_yaml::from_str::<AgentFrontmatterCheck>(yaml_block) {
            Ok(f) => f,
            Err(parse_err) => {
                return vec![Diagnostic::warning(
                    rule::AGENT_SPEC,
                    format!(
                        "agent frontmatter parse error (sync would fail with exit 45): {parse_err}"
                    ),
                )
                .at(Location::file(e.source_path.clone()))];
            }
        };

        let mut out = Vec::new();
        if let Some(v) = &fm.tools
            && !v.is_array()
            && !v.is_null()
        {
            out.push(
                Diagnostic::warning(
                    rule::AGENT_SPEC,
                    "agent 'tools' field must be a list of strings, not a scalar (sync would fail with exit 45)",
                )
                .at(Location::file(e.source_path.clone())),
            );
        }
        if let Some(v) = &fm.disallowed_tools
            && !v.is_array()
            && !v.is_null()
        {
            out.push(
                Diagnostic::warning(
                    rule::AGENT_SPEC,
                    "agent 'disallowedTools' field must be a list of strings, not a scalar (sync would fail with exit 45)",
                )
                .at(Location::file(e.source_path.clone())),
            );
        }
        out
    }
}

/// Extract the YAML block from a frontmatter-fenced markdown file.
/// Returns `None` when the standard `---\n…\n---` fences are not present.
fn extract_frontmatter_yaml(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix("---\n")?;
    let end = rest.find("\n---\n").or_else(|| rest.find("\n---"))?;
    Some(&rest[..end])
}

struct UnsupportedComponents;
impl Rule for UnsupportedComponents {
    fn id(&self) -> &'static str {
        "lint/unsupported-components"
    }
    fn scope(&self) -> Scope {
        Scope::Plugin
    }
    fn check_plugin(&self, p: &PluginIr) -> Vec<Diagnostic> {
        let dir = &p.provenance.source_path;
        let mut d = Vec::new();
        // NB: `hooks/` is intentionally NOT flagged — Tome supports
        // `command`-type hooks, so a `hooks/` dir in a native Tome plugin is
        // not unsupported (the convert importer's CC-`hooks/` warning is a
        // separate, source-specific concern). (CON-3)
        for comp in [
            "monitors",
            "themes",
            "lsp",
            "output-styles",
            "channels",
            "bin",
        ] {
            if dir.join(comp).is_dir() {
                d.push(Diagnostic::warning(
                    rule::UNSUPPORTED_COMPONENT,
                    format!("plugin contains an unsupported `{comp}/` directory"),
                ));
            }
        }
        if dir.join("settings.json").is_file() {
            d.push(Diagnostic::warning(
                rule::UNSUPPORTED_COMPONENT,
                "plugin contains an unsupported `settings.json`",
            ));
        }
        d
    }
}

// --- entry ------------------------------------------------------------------

struct EntryName;
impl Rule for EntryName {
    fn id(&self) -> &'static str {
        rule::NAME_NOT_DIR
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn autofixable(&self) -> bool {
        true
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        let expected = expected_entry_name(e);
        if expected.is_empty() || e.name == expected {
            return Vec::new();
        }
        let mut diag = Diagnostic::error(
            rule::NAME_NOT_DIR,
            format!(
                "entry `name` (`{}`) must match its {} (`{}`)",
                e.name,
                dir_kind(e.kind),
                expected
            ),
        )
        .at(Location::file(e.source_path.clone()));
        if let Ok(content) = bounded_read_to_string(&e.source_path, ENTRY_BODY_MAX)
            && let Some(fixed) = set_frontmatter_name(&content, &expected)
        {
            diag = diag.with_fix(Fix {
                path: e.source_path.clone(),
                replacement: fixed,
            });
        }
        vec![diag]
    }
}

struct EntryDescription;
impl Rule for EntryDescription {
    fn id(&self) -> &'static str {
        rule::DESCRIPTION_MISSING
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        match &e.description {
            None => vec![
                Diagnostic::warning(
                    rule::DESCRIPTION_MISSING,
                    format!("entry `{}` has no description", e.name),
                )
                .at(Location::file(e.source_path.clone())),
            ],
            Some(desc) if desc.chars().count() > DESCRIPTION_MAX => vec![
                Diagnostic::warning(
                    rule::DESCRIPTION_TOO_LONG,
                    format!(
                        "entry `{}` description is {} characters (max {DESCRIPTION_MAX})",
                        e.name,
                        desc.chars().count()
                    ),
                )
                .at(Location::file(e.source_path.clone())),
            ],
            _ => Vec::new(),
        }
    }
}

struct EntryHarnessIsms;
impl Rule for EntryHarnessIsms {
    fn id(&self) -> &'static str {
        rule::RESIDUAL_HARNESS_ISM
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn autofixable(&self) -> bool {
        true
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        // Detect via the shared rewriter (lint mode: legacy positionals are
        // ambiguous, so they are flagged, never rewritten).
        let outcome = rewrite_body(&e.body, RewriteOptions::default());
        if outcome.diagnostics.is_empty() {
            return Vec::new();
        }
        // One whole-file Fix for the rewritable `${CLAUDE_*}` subset.
        let fix = bounded_read_to_string(&e.source_path, ENTRY_BODY_MAX)
            .ok()
            .and_then(|content| {
                let fixed = rewrite_known_vars(&content);
                (fixed != content).then_some(Fix {
                    path: e.source_path.clone(),
                    replacement: fixed,
                })
            });

        let mut out = Vec::new();
        for diag in outcome.diagnostics {
            let mut ld = Diagnostic::warning(rule::RESIDUAL_HARNESS_ISM, diag.message.clone())
                .at(Location::file(e.source_path.clone()));
            if is_rewritable(diag.rule_id)
                && let Some(f) = &fix
            {
                ld = ld.with_fix(f.clone());
            }
            out.push(ld);
        }
        out
    }
}

struct EntryBodyBudget {
    budgets: TokenBudgets,
}
impl Rule for EntryBodyBudget {
    fn id(&self) -> &'static str {
        rule::BODY_TOO_LARGE
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        if e.kind != EntryKind::Skill {
            return Vec::new();
        }
        let bytes = e.body.len();
        let est = est_tokens(bytes);
        let kb = bytes / 1024;
        let loc = Location::file(e.source_path.clone());
        if est >= self.budgets.hard_tokens {
            vec![
                Diagnostic::warning(
                    rule::BODY_TOO_LARGE,
                    format!(
                        "skill body is ~{est} tokens (≈{kb} KB), over the {hard}-token \
                     budget (70% of the {limit}-token MCP-output limit) — get_skill \
                     returns the body inline, so its response will be truncated; split \
                     long material into references/ files",
                        hard = self.budgets.hard_tokens,
                        limit = self.budgets.hard_limit,
                    ),
                )
                .at(loc),
            ]
        } else if est >= self.budgets.soft_tokens {
            vec![
                Diagnostic::warning(
                    rule::BODY_TOO_LARGE,
                    format!(
                        "skill body is ~{est} tokens (≈{kb} KB), past the {soft}-token \
                     budget (70% of the {softlimit}-token MCP-output soft limit, leaving \
                     room for the get_skill response envelope) — move long material into \
                     references/ files the agent loads on demand",
                        soft = self.budgets.soft_tokens,
                        softlimit = TokenBudgets::SOFT_LIMIT,
                    ),
                )
                .at(loc),
            ]
        } else {
            Vec::new()
        }
    }
}

struct EntryResourceBudget {
    budgets: TokenBudgets,
}
impl Rule for EntryResourceBudget {
    fn id(&self) -> &'static str {
        rule::RESOURCE_TOO_LARGE
    }
    fn scope(&self) -> Scope {
        Scope::Entry
    }
    fn check_entry(&self, e: &EntryIr) -> Vec<Diagnostic> {
        if e.kind != EntryKind::Skill {
            return Vec::new();
        }
        let Some(dir) = e.source_path.parent() else {
            return Vec::new();
        };
        let mut resources = Vec::new();
        walk_text_resources(dir, &e.source_path, &mut resources);
        // Deterministic order — read_dir order is OS-dependent.
        resources.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = Vec::new();
        for (path, bytes) in resources {
            let est = est_tokens(usize::try_from(bytes).unwrap_or(usize::MAX));
            if est >= self.budgets.hard_tokens {
                let name = path.strip_prefix(dir).unwrap_or(&path).display();
                out.push(
                    Diagnostic::warning(
                        rule::RESOURCE_TOO_LARGE,
                        format!(
                            "supporting file `{name}` is ~{est} tokens (≈{kb} KB), over \
                             the {hard}-token budget — the agent cannot read it in one \
                             call; split it into smaller files",
                            kb = bytes / 1024,
                            hard = self.budgets.hard_tokens,
                        ),
                    )
                    .at(Location::file(path.clone())),
                );
            }
        }
        out
    }
}

// --- helpers ----------------------------------------------------------------

fn check_name(name: &str, what: &str, d: &mut Vec<Diagnostic>) {
    if name.trim().is_empty() {
        d.push(Diagnostic::error(
            rule::NAME_MISSING,
            format!("{what} `name` is missing or empty"),
        ));
    } else if validate_segment(name).is_err() {
        d.push(Diagnostic::error(
            rule::NAME_INVALID,
            format!("{what} name `{name}` is not a safe path segment"),
        ));
    } else if !is_kebab(name) {
        d.push(Diagnostic::warning(
            rule::NAME_NOT_KEBAB,
            format!("{what} name `{name}` is not kebab-case"),
        ));
    }
}

fn check_version(version: &str, d: &mut Vec<Diagnostic>) {
    let v = version.trim();
    if v.is_empty() {
        d.push(Diagnostic::error(
            rule::VERSION_INVALID,
            "`version` is missing",
        ));
    } else if semver::Version::parse(v).is_err() {
        d.push(Diagnostic::error(
            rule::VERSION_INVALID,
            format!("`version` `{version}` is not valid semver"),
        ));
    }
}

fn dir_kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Skill => "directory",
        EntryKind::Command | EntryKind::Agent => "file name",
    }
}

/// The name an entry MUST carry: a skill's directory name, or a command/agent's
/// file stem.
fn expected_entry_name(e: &EntryIr) -> String {
    match e.kind {
        EntryKind::Skill => e
            .source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned(),
        EntryKind::Command | EntryKind::Agent => e
            .source_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned(),
    }
}

fn is_rewritable(rule_id: &str) -> bool {
    matches!(
        rule_id,
        rewrite::rule::PLUGIN_ROOT
            | rewrite::rule::PLUGIN_DATA
            | rewrite::rule::SKILL_DIR
            | rewrite::rule::PROJECT_DIR
    )
}

/// Rewrite (or insert) the frontmatter `name:` to `new_name`, preserving the
/// rest of the file verbatim. Returns `None` when the file has no parseable
/// LF-delimited frontmatter block (the diagnostic is still emitted, just without
/// an autofix).
fn set_frontmatter_name(content: &str, new_name: &str) -> Option<String> {
    let stripped = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let after_open = stripped.strip_prefix("---\n")?;
    let close_at = after_open.find("\n---")?;
    let yaml = &after_open[..close_at];
    let rest = &after_open[close_at..]; // starts with "\n---…"

    let mut lines: Vec<String> = yaml.lines().map(str::to_owned).collect();
    let mut replaced = false;
    for line in &mut lines {
        if line.trim_start().starts_with("name:") {
            *line = format!("name: {new_name}");
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.insert(0, format!("name: {new_name}"));
    }
    Some(format!("---\n{}{rest}", lines.join("\n")))
}

/// Whether a supporting file is text the agent would read via a get_skill
/// resource path. Extension-less files (LICENSE, NOTES) count as text;
/// known binary assets (images, archives) do not.
fn is_text_like(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdx" | "txt" | "rst"
        ),
        None => true,
    }
}

/// Recursively collect `(path, byte_len)` for text-like files under `dir`,
/// excluding `skill_file` and skipping symlinks — mirroring `get_skill`'s
/// `walk_dir`, which never serves symlinked resources. I/O errors short-circuit
/// the offending directory rather than failing the lint run.
fn walk_text_resources(dir: &Path, skill_file: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            walk_text_resources(&path, skill_file, out);
        } else if path != skill_file
            && is_text_like(&path)
            && let Ok(meta) = entry.metadata()
        {
            out.push((path, meta.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::ir::{Artifact, MappedFrontmatter, Provenance, Severity};
    use crate::authoring::lint::run;
    use crate::catalog::manifest::Owner;
    use std::fs;
    use std::path::PathBuf;

    fn entry(
        kind: EntryKind,
        name: &str,
        source: PathBuf,
        desc: Option<&str>,
        body: &str,
    ) -> EntryIr {
        EntryIr {
            kind,
            name: name.to_owned(),
            description: desc.map(str::to_owned),
            frontmatter: MappedFrontmatter::default(),
            body: body.to_owned(),
            supporting_files: Vec::new(),
            source_path: source,
            diagnostics: Vec::new(),
        }
    }

    fn plugin(name: &str, version: &str, dir: PathBuf, entries: Vec<EntryIr>) -> PluginIr {
        PluginIr {
            name: name.to_owned(),
            version: version.to_owned(),
            description: None,
            author: None,
            license: None,
            entries,
            mcp_servers: Vec::new(),
            hooks_files: Vec::new(),
            hooks_json: None,
            mcp_json: None,
            provenance: Provenance::local("tome", dir),
            diagnostics: Vec::new(),
        }
    }

    fn run_plugin(p: PluginIr) -> Vec<Diagnostic> {
        run(&Artifact::Plugin(p), &all()).diagnostics
    }

    fn has(d: &[Diagnostic], id: &str) -> bool {
        d.iter().any(|x| x.rule_id == id)
    }

    #[test]
    fn resource_budget_flags_large_text_file_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(dir.join("references")).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\ndescription: d\n---\nbody\n").unwrap();

        // from_max(1_000) -> hard_tokens = 700 -> ~2_800-byte threshold.
        let rule = EntryResourceBudget {
            budgets: TokenBudgets::from_max(1_000),
        };
        let e = entry(EntryKind::Skill, "foo", skill, Some("d"), "body\n");

        // Small text file: no finding.
        fs::write(dir.join("references/small.md"), "x".repeat(100)).unwrap();
        assert!(rule.check_entry(&e).is_empty(), "small file clean");

        // Large text file: one finding naming the file.
        fs::write(dir.join("references/big.md"), "x".repeat(3_000)).unwrap();
        let d = rule.check_entry(&e);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, rule::RESOURCE_TOO_LARGE);
        assert!(d[0].message.contains("big.md"), "{:?}", d[0].message);

        // Large BINARY file: skipped (not text-like) — still just big.md.
        fs::write(dir.join("references/big.png"), vec![0u8; 5_000]).unwrap();
        assert_eq!(rule.check_entry(&e).len(), 1, "png ignored");
    }

    #[test]
    fn resource_budget_walks_nested_dirs() {
        // get_skill's walk recurses, so a large file in references/sub/ must be
        // flagged too — locks the recursive-walk parity the feature relies on.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(dir.join("references/sub")).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\ndescription: d\n---\nbody\n").unwrap();
        fs::write(dir.join("references/sub/deep.md"), "x".repeat(3_000)).unwrap();

        let rule = EntryResourceBudget {
            budgets: TokenBudgets::from_max(1_000),
        };
        let e = entry(EntryKind::Skill, "foo", skill, Some("d"), "body\n");
        let d = rule.check_entry(&e);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("deep.md"), "{:?}", d[0].message);
    }

    #[test]
    #[cfg(unix)]
    fn resource_budget_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(dir.join("references")).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\ndescription: d\n---\nbody\n").unwrap();
        // A large real target outside the skill dir, symlinked in.
        let target = tmp.path().join("huge.md");
        fs::write(&target, "x".repeat(5_000)).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("references/link.md")).unwrap();

        let rule = EntryResourceBudget {
            budgets: TokenBudgets::from_max(1_000),
        };
        let e = entry(EntryKind::Skill, "foo", skill, Some("d"), "body\n");
        assert!(
            rule.check_entry(&e).is_empty(),
            "symlinked resource skipped"
        );
    }

    #[test]
    fn registries_wire_the_budget_rules() {
        let all_ids: Vec<&str> = all().iter().map(|r| r.id()).collect();
        assert!(
            all_ids.contains(&rule::BODY_TOO_LARGE),
            "body rule in all()"
        );
        assert!(
            all_ids.contains(&rule::RESOURCE_TOO_LARGE),
            "resource rule in all()"
        );

        let conv_ids: Vec<&str> = for_convert().iter().map(|r| r.id()).collect();
        assert!(
            conv_ids.contains(&rule::BODY_TOO_LARGE),
            "body rule in for_convert()"
        );
        assert!(
            !conv_ids.contains(&rule::RESOURCE_TOO_LARGE),
            "resource rule is FS-reading — all()-only, like UnsupportedComponents/EntryName"
        );
    }

    #[test]
    fn body_budget_fires_at_soft_and_hard_boundaries() {
        let rule = EntryBodyBudget {
            budgets: TokenBudgets::from_max(25_000), // hard=17_500, soft=7_000
        };
        let mk = |n: usize| {
            entry(
                EntryKind::Skill,
                "foo",
                PathBuf::from("SKILL.md"),
                Some("d"),
                &"x".repeat(n),
            )
        };

        // Soft boundary is 7_000 tokens = 28_000 bytes.
        assert!(
            rule.check_entry(&mk(28_000 - 1)).is_empty(),
            "just under soft"
        );
        let at_soft = rule.check_entry(&mk(28_000));
        assert_eq!(at_soft.len(), 1);
        assert_eq!(at_soft[0].rule_id, rule::BODY_TOO_LARGE);
        assert!(at_soft[0].message.contains("past the 7000-token budget"));

        // Hard boundary is 17_500 tokens = 70_000 bytes.
        let at_hard = rule.check_entry(&mk(70_000));
        assert_eq!(at_hard.len(), 1);
        assert!(at_hard[0].message.contains("over the 17500-token budget"));
        assert!(
            at_hard[0].message.contains("25000-token"),
            "shows effective limit"
        );

        // Empty body is clean.
        assert!(rule.check_entry(&mk(0)).is_empty());
    }

    #[test]
    fn budget_rules_ignore_non_skill_entries() {
        // A Command with a huge body + the same-dir siblings must NOT be flagged:
        // get_skill only ever serves Skills.
        let big = "x".repeat(80_000); // ~20k tokens, well over any budget
        let cmd = entry(
            EntryKind::Command,
            "do",
            PathBuf::from("commands/do.md"),
            Some("d"),
            &big,
        );
        let body_rule = EntryBodyBudget {
            budgets: TokenBudgets::from_max(25_000),
        };
        assert!(
            body_rule.check_entry(&cmd).is_empty(),
            "command body not flagged"
        );
        let res_rule = EntryResourceBudget {
            budgets: TokenBudgets::from_max(25_000),
        };
        assert!(
            res_rule.check_entry(&cmd).is_empty(),
            "command siblings not walked"
        );
    }

    #[test]
    fn token_budgets_apply_headroom_and_clamp() {
        let d = TokenBudgets::from_max(25_000);
        assert_eq!(d.hard_limit, 25_000);
        assert_eq!(d.hard_tokens, 17_500);
        assert_eq!(d.soft_tokens, 7_000);
        // Tiny configured cap: soft is clamped down to hard so soft <= hard.
        let t = TokenBudgets::from_max(8_000);
        assert_eq!(t.hard_tokens, 5_600);
        assert_eq!(t.soft_tokens, 5_600);
        // est_tokens is integer bytes/4.
        assert_eq!(est_tokens(0), 0);
        assert_eq!(est_tokens(4), 1);
        assert_eq!(est_tokens(28_000), 7_000);
    }

    #[test]
    fn flags_missing_version_and_non_kebab_name() {
        let tmp = tempfile::tempdir().unwrap();
        let p = plugin("Not_Kebab", "", tmp.path().to_path_buf(), vec![]);
        let d = run_plugin(p);
        assert!(has(&d, rule::VERSION_INVALID));
        assert!(has(&d, rule::NAME_NOT_KEBAB));
    }

    #[test]
    fn clean_plugin_has_no_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\ndescription: d\n---\nbody\n").unwrap();
        let e = entry(EntryKind::Skill, "foo", skill, Some("d"), "body\n");
        let d = run_plugin(plugin(
            "good-plugin",
            "1.0.0",
            tmp.path().to_path_buf(),
            vec![e],
        ));
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn flags_name_not_matching_dir_with_an_autofix() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/realdir");
        fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: wrongname\ndescription: d\n---\nbody\n").unwrap();
        // Entry name (from frontmatter) != dir name `realdir`.
        let e = entry(EntryKind::Skill, "wrongname", skill, Some("d"), "body\n");
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![e]));
        let finding = d
            .iter()
            .find(|x| x.rule_id == rule::NAME_NOT_DIR)
            .expect("name-not-dir");
        assert_eq!(finding.severity, Severity::Error);
        let fix = finding.autofix.as_ref().expect("autofix present");
        assert!(
            fix.replacement.contains("name: realdir"),
            "{}",
            fix.replacement
        );
    }

    #[test]
    fn flags_missing_description_and_residual_harness_ism() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\n---\nUse ${CLAUDE_PLUGIN_ROOT}/x\n").unwrap();
        let e = entry(
            EntryKind::Skill,
            "foo",
            skill,
            None,
            "Use ${CLAUDE_PLUGIN_ROOT}/x\n",
        );
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![e]));
        assert!(has(&d, rule::DESCRIPTION_MISSING));
        let hi = d
            .iter()
            .find(|x| x.rule_id == rule::RESIDUAL_HARNESS_ISM)
            .expect("harness-ism");
        let fix = hi
            .autofix
            .as_ref()
            .expect("rewritable harness-ism has a fix");
        assert!(fix.replacement.contains("${TOME_PLUGIN_DIR}/x"));
    }

    #[test]
    fn flags_unsupported_component_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("monitors")).unwrap();
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]));
        assert!(has(&d, rule::UNSUPPORTED_COMPONENT));
    }

    #[test]
    fn is_kebab_and_email_helpers() {
        assert!(is_kebab("my-plugin"));
        assert!(!is_kebab("My_Plugin"));
        assert!(!is_kebab("-x"));
        assert!(!is_kebab("a--b"));
        assert!(looks_like_email("a@b.io"));
        assert!(!looks_like_email("a@b"));
        assert!(!looks_like_email("a@@b.io"));
    }

    #[test]
    fn set_frontmatter_name_replaces_or_inserts() {
        let replaced =
            set_frontmatter_name("---\nname: old\ndescription: d\n---\nbody\n", "new").unwrap();
        assert!(replaced.contains("name: new"));
        assert!(replaced.contains("description: d"));
        assert!(replaced.ends_with("body\n"));
        let inserted = set_frontmatter_name("---\ndescription: d\n---\nbody\n", "new").unwrap();
        assert!(inserted.contains("name: new"));
    }

    #[test]
    fn set_frontmatter_name_returns_none_for_crlf_or_no_frontmatter() {
        // CRLF block isn't matched by the LF-anchored strip → no autofix.
        assert!(set_frontmatter_name("---\r\nname: old\r\n---\r\nbody\r\n", "new").is_none());
        // No frontmatter at all → no autofix (the diagnostic is still emitted).
        assert!(set_frontmatter_name("just a body\n", "new").is_none());
    }

    #[test]
    fn description_too_long_fires_at_the_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(&skill, "---\nname: foo\n---\nbody\n").unwrap();
        let long: String = "x".repeat(DESCRIPTION_MAX + 1);
        let e = entry(
            EntryKind::Skill,
            "foo",
            skill.clone(),
            Some(&long),
            "body\n",
        );
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![e]));
        assert!(has(&d, rule::DESCRIPTION_TOO_LONG));
        // Exactly DESCRIPTION_MAX does NOT fire (false-positive guard).
        let ok: String = "x".repeat(DESCRIPTION_MAX);
        let e = entry(EntryKind::Skill, "foo", skill, Some(&ok), "body\n");
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![e]));
        assert!(!has(&d, rule::DESCRIPTION_TOO_LONG));
    }

    #[test]
    fn command_name_must_match_its_file_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("commands");
        fs::create_dir_all(&dir).unwrap();
        let cmd = dir.join("do.md");
        fs::write(&cmd, "---\nname: wrong\ndescription: d\n---\nbody\n").unwrap();
        // Command file stem is `do`, frontmatter name is `wrong`.
        let e = entry(EntryKind::Command, "wrong", cmd, Some("d"), "body\n");
        let d = run_plugin(plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![e]));
        assert!(has(&d, rule::NAME_NOT_DIR));
    }

    fn run_catalog(c: CatalogIr) -> Vec<Diagnostic> {
        run(&Artifact::Catalog(c), &all()).diagnostics
    }

    #[test]
    fn catalog_owner_email_and_duplicate_plugin_fire() {
        let tmp = tempfile::tempdir().unwrap();
        let c = CatalogIr {
            name: "c".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            owner: Owner {
                name: "o".into(),
                email: "not-an-email".into(),
            },
            plugins: vec![
                plugin("dup", "1.0.0", tmp.path().to_path_buf(), vec![]),
                plugin("dup", "1.0.0", tmp.path().to_path_buf(), vec![]),
            ],
            provenance: Provenance::local("tome", tmp.path().to_path_buf()),
            diagnostics: Vec::new(),
        };
        let d = run_catalog(c);
        assert!(has(&d, rule::OWNER_EMAIL_INVALID));
        assert!(has(&d, rule::DUP_PLUGIN));
    }

    // --- mcp-spec rule -------------------------------------------------------

    #[test]
    fn mcp_spec_valid_object_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);
        p.mcp_json = Some(r#"{"mcpServers":{}}"#.to_owned());
        assert!(
            McpSpec.check_plugin(&p).is_empty(),
            "valid object must pass"
        );
    }

    #[test]
    fn mcp_spec_invalid_json_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);
        p.mcp_json = Some("{not json".to_owned());
        let d = McpSpec.check_plugin(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, rule::MCP_SPEC);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(d[0].message.contains(".mcp.json is not valid JSON"));
    }

    #[test]
    fn mcp_spec_non_object_top_level_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);
        // Arrays and null are not top-level objects.
        p.mcp_json = Some("[1, 2, 3]".to_owned());
        let d = McpSpec.check_plugin(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, rule::MCP_SPEC);
        assert!(d[0].message.contains("JSON object at the top level"));

        // A string is also not an object.
        p.mcp_json = Some("\"hello\"".to_owned());
        let d = McpSpec.check_plugin(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, rule::MCP_SPEC);
    }

    #[test]
    fn mcp_spec_absent_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);
        p.mcp_json = None;
        assert!(McpSpec.check_plugin(&p).is_empty(), "absent is silent");
    }

    // --- agent-spec rule -----------------------------------------------------

    #[test]
    fn agent_spec_wrong_tools_type_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents");
        fs::create_dir_all(&dir).unwrap();
        let agent_file = dir.join("helper.md");
        fs::write(
            &agent_file,
            "---\nname: helper\ndescription: d\ntools: 7\n---\nbody\n",
        )
        .unwrap();
        let e = entry(EntryKind::Agent, "helper", agent_file, Some("d"), "body\n");
        let d = AgentSpec.check_entry(&e);
        assert!(
            d.iter().any(|x| x.rule_id == rule::AGENT_SPEC),
            "scalar tools must be flagged: {d:?}"
        );
        assert!(
            d.iter().any(|x| x.message.contains("'tools'")),
            "message must mention the field: {d:?}"
        );
    }

    #[test]
    fn agent_spec_valid_tools_list_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents");
        fs::create_dir_all(&dir).unwrap();
        let agent_file = dir.join("helper.md");
        fs::write(
            &agent_file,
            "---\nname: helper\ndescription: d\ntools:\n  - Bash\n  - Read\n---\nbody\n",
        )
        .unwrap();
        let e = entry(EntryKind::Agent, "helper", agent_file, Some("d"), "body\n");
        assert!(AgentSpec.check_entry(&e).is_empty(), "list tools must pass");
    }

    #[test]
    fn agent_spec_wrong_disallowed_tools_type_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents");
        fs::create_dir_all(&dir).unwrap();
        let agent_file = dir.join("helper.md");
        fs::write(
            &agent_file,
            "---\nname: helper\ndescription: d\ndisallowedTools: NotAList\n---\nbody\n",
        )
        .unwrap();
        let e = entry(EntryKind::Agent, "helper", agent_file, Some("d"), "body\n");
        let d = AgentSpec.check_entry(&e);
        assert!(
            d.iter().any(|x| x.rule_id == rule::AGENT_SPEC),
            "scalar disallowedTools must be flagged: {d:?}"
        );
        assert!(
            d.iter().any(|x| x.message.contains("'disallowedTools'")),
            "message must mention the field: {d:?}"
        );
    }

    #[test]
    fn agent_spec_non_agent_entry_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills/foo");
        fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        fs::write(
            &skill,
            "---\nname: foo\ndescription: d\ntools: 7\n---\nbody\n",
        )
        .unwrap();
        let e = entry(EntryKind::Skill, "foo", skill, Some("d"), "body\n");
        assert!(
            AgentSpec.check_entry(&e).is_empty(),
            "non-agent must be silent even with bad fields"
        );
    }

    // --- hooks-spec non-array values -----------------------------------------

    #[test]
    fn hooks_spec_non_array_values_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);
        // A string value for a hooks event must be flagged.
        p.hooks_json = Some(r#"{"PreToolUse": "not-an-array"}"#.to_owned());
        let d = HooksSpec.check_plugin(&p);
        assert_eq!(d.len(), 1, "one non-array value must produce one warning");
        assert_eq!(d[0].rule_id, rule::HOOKS_SPEC);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(
            d[0].message.contains("PreToolUse"),
            "message must name the key: {}",
            d[0].message
        );
        assert!(
            d[0].message.contains("array"),
            "message must mention array: {}",
            d[0].message
        );
    }

    #[test]
    fn hooks_spec_flags_invalid_json_and_wrapped_form() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = plugin("p", "1.0.0", tmp.path().to_path_buf(), vec![]);

        // No hooks at all → silent.
        p.hooks_json = None;
        assert!(HooksSpec.check_plugin(&p).is_empty());

        // Event-map form (what harness sync expects) → silent.
        p.hooks_json = Some(r#"{"PreToolUse":[]}"#.to_owned());
        assert!(
            HooksSpec.check_plugin(&p).is_empty(),
            "event-map must be silent"
        );

        // Empty event-map → silent.
        p.hooks_json = Some("{}".to_owned());
        assert!(
            HooksSpec.check_plugin(&p).is_empty(),
            "empty event-map must be silent"
        );

        // Wrapped form: {"hooks": {...}} → warns because harness sync would fail.
        p.hooks_json = Some(r#"{"hooks":{}}"#.to_owned());
        let d = HooksSpec.check_plugin(&p);
        assert_eq!(d.len(), 1, "wrapped form must warn");
        assert_eq!(d[0].rule_id, rule::HOOKS_SPEC);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(
            d[0].message.contains("wrapped form"),
            "message: {}",
            d[0].message
        );

        // Wrapped form with description and non-empty event-map → same warning.
        p.hooks_json = Some(
            r#"{"description":"my hooks","hooks":{"PreToolUse":[{"type":"command"}]}}"#.to_owned(),
        );
        let d = HooksSpec.check_plugin(&p);
        assert_eq!(d.len(), 1, "full wrapped form must warn");
        assert_eq!(d[0].rule_id, rule::HOOKS_SPEC);

        // Invalid JSON → one warning naming the file and the consequence.
        p.hooks_json = Some("{not json".to_owned());
        let d = HooksSpec.check_plugin(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, rule::HOOKS_SPEC);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(d[0].message.contains("hooks/hooks.json"));
    }
}
