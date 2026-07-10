//! Claude Code → Tome IR importer (Tier 1, FR-010/FR-012/FR-013).
//!
//! Reads a Claude Code plugin directory — `.claude-plugin/plugin.json` plus the
//! conventional `skills/`, `commands/`, `agents/` trees, an optional `.mcp.json`,
//! and the `hooks/` subtree — through the [`UntrustedRoot`] guard and produces a
//! [`PluginIr`]. Honest by construction:
//!
//! * manifest fields map 1:1 where Tome models them; dropped fields surface as
//!   `Info`, exotic fields (`userConfig`/`dependencies`) as `Warning`
//!   (`data-model.md §1`);
//! * unsupported component directories (`monitors/`, `themes/`, `lsp/`, …) and
//!   plugin `settings.json` surface as `Warning`s (FR-012, §8);
//! * the `hooks/` subtree is copied **verbatim** (byte-identical, no
//!   harness-ism rewriting) so that `harness::hooks::read_rewritten_entries`
//!   can apply the `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}` rewrite at
//!   sync time with the tokens intact;
//! * entry frontmatter maps the Tome-modelled set, dropping the rest with an
//!   `Info`, tool-restriction fields with a `Warning` (silently broadening
//!   capability), and treating agent conversion as lossy (FR-013, §6);
//! * entry bodies are harness-ism-rewritten ([`rewrite_body`]); command bodies
//!   additionally get the legacy 1-based `$1..$9` → 0-based rewrite.
//!
//! Third-party JSON is parsed leniently (`serde_json::Value`) — an unknown
//! field is a diagnostic, never a parse abort (the strictness boundary).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::authoring::ir::{
    AgentMeta, CatalogIr, Diagnostic, EntryIr, McpServerIr, McpTransport, PluginIr, Provenance,
    SupportingFile,
};
use crate::authoring::rewrite::{RewriteOptions, rewrite_body};
use crate::authoring::untrusted::UntrustedRoot;
use crate::catalog::git::{Git, scrub_to_string};
use crate::catalog::manifest::Owner;
use crate::error::TomeError;
use crate::plugin::frontmatter::{frontmatter_keys, parse_skill_frontmatter_str};
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomeAuthor;
use crate::util::{HARNESS_MCP_MAX, PLUGIN_MANIFEST_MAX};

// The diagnostic rule ids this importer emits live in the shared
// `super::rule` SSOT (promoted when Codex became the second consumer).
use super::{FetchContext, rule};

/// Frontmatter keys Tome models 1:1 (kebab-case, matching `SkillFrontmatter`'s
/// `rename_all`; the two snake exceptions carry explicit renames). Anything not
/// in this set is dropped with a diagnostic.
const MODELLED_FRONTMATTER: &[&str] = &[
    "name",
    "description",
    "when_to_use",
    "arguments",
    "argument-hint",
    "disable-model-invocation",
    "user-invocable",
    "prompt_name",
];

/// Agent-specific frontmatter keys preserved through the pipeline into
/// [`AgentMeta`] (G4). These are NOT in [`MODELLED_FRONTMATTER`] (they are
/// not `SkillFrontmatter` fields) but are preserved for agent entries so
/// `harness sync` can translate them.
const AGENT_META_KEYS: &[&str] = &[
    "model",
    "tools",
    "allowed-tools",
    "disallowed-tools",
    "permissionMode",
    "permission_mode",
];

/// Frontmatter keys whose loss silently broadens capability — always a Warning
/// when the entry is NOT an agent (for agents we preserve them via AgentMeta).
const TOOL_RESTRICTION_KEYS: &[&str] = &["allowed-tools", "disallowed-tools"];

/// When an entry has no frontmatter `description`, fall back to this many
/// characters of the (already-rewritten) body.
const DESCRIPTION_FALLBACK_CHARS: usize = 500;

/// Resolve an entry's description: the trimmed frontmatter value if non-empty,
/// else a prefix of the **rewritten** body — so a fallback reflects the
/// rewritten harness-isms (e.g. `$0`, not the source's `$1`).
fn resolved_description(fm: &crate::plugin::frontmatter::SkillFrontmatter, body: &str) -> String {
    match fm.description.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_owned(),
        _ => body.chars().take(DESCRIPTION_FALLBACK_CHARS).collect(),
    }
}

/// Serde target for agent-specific frontmatter fields. Parsed leniently from
/// the raw source YAML (third-party input). Mirrors the `AgentFrontmatter`
/// struct in `harness::agents` but scoped to the fields Tome preserves through
/// the convert pipeline (G4). Unknown keys are silently tolerated per the
/// strictness boundary (principle IV).
#[derive(Default, serde::Deserialize)]
struct AgentSourceFrontmatter {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    /// `allowed-tools` is the kebab-case CC spelling; also accept snake_case.
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    allowed_tools: Option<Vec<String>>,
    /// `disallowedTools` is the CC camelCase spelling; also accept kebab/snake.
    #[serde(
        default,
        rename = "disallowedTools",
        alias = "disallowed-tools",
        alias = "disallowed_tools"
    )]
    disallowed_tools: Option<Vec<String>>,
    /// `permissionMode` is the CC camelCase spelling; also accept snake_case.
    #[serde(default, rename = "permissionMode", alias = "permission_mode")]
    permission_mode: Option<String>,
}

/// Extract the [`AgentMeta`] from the raw source YAML for an agent entry (G4).
///
/// Parses leniently — unknown keys are silently ignored per the strictness
/// boundary. Returns `None` when the content has no parseable YAML block.
fn parse_agent_meta(content: &str) -> Option<AgentMeta> {
    // Reuse the same delimiter split logic as `frontmatter_keys`.
    let stripped = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    // Find the first `---` line (opening delimiter).
    let after_open = stripped
        .strip_prefix("---\n")
        .or_else(|| stripped.strip_prefix("---\r\n"))?;
    // Find the closing `---` line.
    let close_pos = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))?;
    let yaml_block = &after_open[..close_pos];

    let Ok(parsed) = serde_yaml::from_str::<AgentSourceFrontmatter>(yaml_block) else {
        return None;
    };

    // `tools` from Claude Code means "allowed tools"; merge with the explicit
    // `allowed-tools` key (Claude Code uses both conventions).
    let tools = parsed.tools.or(parsed.allowed_tools);

    let meta = AgentMeta {
        model: parsed.model,
        tools,
        disallowed_tools: parsed.disallowed_tools,
        permission_mode: parsed.permission_mode,
    };

    if meta.is_empty() { None } else { Some(meta) }
}

/// Token strings that are substituted at MCP-serve time but NOT when harness
/// sync writes native agent bodies. When an agent body contains these tokens
/// after the harness-ism rewrite, the author should be warned (G8).
const UNRESOLVED_AGENT_TOKENS: &[&str] = &[
    "${TOME_PLUGIN_DIR}",
    "${TOME_PLUGIN_DATA}",
    "${TOME_SKILL_DIR}",
    "${TOME_PROJECT_DIR}",
];

/// Unsupported component directories/files (FR-012, §8): present ⇒ Warning.
const UNSUPPORTED_COMPONENTS: &[(&str, &str)] = &[
    ("monitors", "monitors"),
    ("themes", "themes"),
    ("lsp", "LSP servers"),
    ("output-styles", "output styles"),
    ("channels", "channels"),
    ("bin", "`bin/` executables"),
];

/// Defensive bounds against a hostile source tree.
const MAX_SUPPORTING_DEPTH: usize = 16;
const MAX_SUPPORTING_FILES: usize = 4096;
/// Cap total directories enumerated so a shallow-but-very-wide tree cannot turn
/// the walk into unbounded work even when it stays under the file cap.
const MAX_SUPPORTING_DIRS: usize = 4096;
/// VCS metadata / OS junk never copied into a converted artifact. Relevant for
/// bare native-skill sources whose root may be a git checkout.
const SKIP_SUPPORTING_NAMES: &[&str] = &[".git", ".hg", ".svn", ".DS_Store"];

/// Import a Claude Code plugin directory into a [`PluginIr`]. `default_name` is
/// used when the manifest omits `name`; `source_path` is recorded for the
/// report.
pub fn import_plugin(
    root: &UntrustedRoot,
    default_name: &str,
    source_path: &Path,
) -> Result<PluginIr, TomeError> {
    let mut diagnostics = Vec::new();

    // --- manifest -----------------------------------------------------------
    let manifest_json =
        root.read_text(Path::new(".claude-plugin/plugin.json"), PLUGIN_MANIFEST_MAX)?;
    let value: serde_json::Value = serde_json::from_str(&manifest_json).map_err(|e| {
        TomeError::Usage(format!(
            "source .claude-plugin/plugin.json is not valid JSON: {e}"
        ))
    })?;

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_name.to_owned());
    let version = match value.get("version").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_owned(),
        _ => {
            diagnostics.push(Diagnostic::warning(
                rule::MISSING_VERSION,
                "plugin.json has no `version`; defaulting to `0.0.0`",
            ));
            "0.0.0".to_owned()
        }
    };
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let license = value
        .get("license")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let author = parse_author(value.get("author"));

    for field in [
        "displayName",
        "homepage",
        "repository",
        "keywords",
        "$schema",
    ] {
        if value.get(field).is_some() {
            diagnostics.push(Diagnostic::info(
                rule::DROPPED_MANIFEST_FIELD,
                format!("plugin.json `{field}` is not modelled by Tome; dropping it"),
            ));
        }
    }
    for field in ["userConfig", "dependencies"] {
        if value.get(field).is_some() {
            diagnostics.push(Diagnostic::warning(
                rule::UNSUPPORTED_MANIFEST_FIELD,
                format!("plugin.json `{field}` has no Tome equivalent; it is dropped from the conversion"),
            ));
        }
    }
    // Component-path overrides: Tome uses the conventional dirs, so a custom
    // path is dropped (info).
    for field in ["commands", "agents", "skills", "hooks", "mcpServers"] {
        if value.get(field).is_some() {
            diagnostics.push(Diagnostic::info(
                rule::DROPPED_MANIFEST_FIELD,
                format!(
                    "plugin.json `{field}` path override is ignored; Tome reads the conventional `{field}` location"
                ),
            ));
        }
    }

    // --- entries ------------------------------------------------------------
    let mut entries = Vec::new();
    import_skill_dir(root, &mut entries, &mut diagnostics)?;
    import_md_dir(
        root,
        "commands",
        EntryKind::Command,
        &mut entries,
        &mut diagnostics,
    )?;
    import_md_dir(
        root,
        "agents",
        EntryKind::Agent,
        &mut entries,
        &mut diagnostics,
    )?;

    // --- unsupported components (FR-012) -----------------------------------
    for (dir, label) in UNSUPPORTED_COMPONENTS {
        if root.is_dir(Path::new(dir)) {
            diagnostics.push(Diagnostic::warning(
                rule::UNSUPPORTED_COMPONENT,
                format!(
                    "`{dir}/` ({label}) is not representable in Tome; dropped from the conversion"
                ),
            ));
        }
    }
    if root.is_file(Path::new("settings.json")) {
        diagnostics.push(Diagnostic::warning(
            rule::UNSUPPORTED_COMPONENT,
            "plugin `settings.json` is not representable in Tome; dropped from the conversion",
        ));
    }

    // --- unrecognised top-level dirs/files (closes #523) -------------------
    // After the known-component and UNSUPPORTED_COMPONENTS passes, enumerate
    // every top-level entry so that unrecognised directories like `scripts/`
    // or `lib/` produce an actionable warning instead of silently vanishing.
    // This matters because hooks or commands often shell out to
    // `${CLAUDE_PLUGIN_ROOT}/scripts/…` — if the directory isn't imported the
    // reference breaks silently at runtime.
    warn_unrecognised_plugin_root_entries(root, &mut diagnostics)?;

    // --- hooks/ verbatim pass-through --------------------------------------
    let (hooks_files, hooks_json) = collect_hooks(root, &mut diagnostics)?;

    // --- MCP servers --------------------------------------------------------
    let mcp_servers = import_mcp(root, &mut diagnostics)?;

    Ok(PluginIr {
        name,
        version,
        description,
        author,
        license,
        entries,
        mcp_servers,
        hooks_files,
        hooks_json,
        mcp_json: None,
        provenance: Provenance {
            source_harness: "claude-code".to_owned(),
            source_path: source_path.to_path_buf(),
        },
        diagnostics,
    })
}

/// Import a Claude Code marketplace (`.claude-plugin/marketplace.json` + the
/// vendored plugin subdirectories it lists) into a [`CatalogIr`].
///
/// Relative-path plugins are imported and vendored inline; a failure converting
/// **any** relative-path plugin aborts the whole conversion (the error is
/// propagated, so `build_ir` returns before `emit` and nothing lands —
/// all-or-nothing, FR-014a). Remote-source plugins (`github`/`git`/`url`) are
/// shallow-cloned and vendored by default; `fetch.enabled = false` (i.e.
/// `--no-fetch`) restores the hermetic warn-and-skip path. A fetch failure
/// skips that plugin only (forward-progress) but is strict-blocking so
/// `--strict` hard-fails on any failure. Unfetchable kinds (`npm`, etc.) are
/// always warned-and-skipped. A fetched plugin.json name that disagrees with
/// the marketplace entry is resolved in favor of the entry and is not
/// strict-blocking.
pub fn import_marketplace(
    root: &UntrustedRoot,
    source_path: &Path,
    fetch: &mut FetchContext,
) -> Result<CatalogIr, TomeError> {
    let mut diagnostics = Vec::new();

    let manifest_json = root.read_text(
        Path::new(".claude-plugin/marketplace.json"),
        PLUGIN_MANIFEST_MAX,
    )?;
    let value: serde_json::Value = serde_json::from_str(&manifest_json).map_err(|e| {
        TomeError::Usage(format!(
            "source .claude-plugin/marketplace.json is not valid JSON: {e}"
        ))
    })?;

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            source_path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("catalog")
                .to_owned()
        });
    let description = match value.get("description").and_then(|v| v.as_str()) {
        Some(d) if !d.trim().is_empty() => d.trim().to_owned(),
        _ => {
            diagnostics.push(Diagnostic::info(
                rule::CATALOG_SYNTHESIZED_FIELD,
                "marketplace has no `description`; synthesizing one",
            ));
            format!("Converted from the {name} Claude Code marketplace")
        }
    };
    let version = match value.get("version").and_then(|v| v.as_str()).or_else(|| {
        value
            .get("metadata")
            .and_then(|m| m.get("version"))
            .and_then(|v| v.as_str())
    }) {
        Some(v) if !v.trim().is_empty() => v.trim().to_owned(),
        _ => {
            diagnostics.push(Diagnostic::warning(
                rule::MISSING_VERSION,
                "marketplace has no `version`; defaulting to `0.0.0`",
            ));
            "0.0.0".to_owned()
        }
    };
    let owner = parse_owner(value.get("owner"), &mut diagnostics);

    let mut plugins = Vec::new();
    if let Some(arr) = value.get("plugins").and_then(|v| v.as_array()) {
        for entry in arr {
            let pname = entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let label = pname.as_deref().unwrap_or("<unnamed>");
            match classify_plugin_source(entry.get("source")) {
                PluginSource::Relative(rel) => {
                    // Validate the source path is in-root + symlink-safe, then
                    // import the vendored plugin under its own sub-root.
                    let plugin_abs = root.resolve(Path::new(&rel))?;
                    let plugin_root = UntrustedRoot::open(&plugin_abs)?;
                    let default = pname.clone().unwrap_or_else(|| rel.clone());
                    // ALL-OR-NOTHING: propagate any single-plugin import failure.
                    let plugin = import_plugin(&plugin_root, &default, &plugin_abs)?;
                    // The vendored plugin's own `name` (from its plugin.json)
                    // becomes its emitted directory under the catalog, so it
                    // MUST be a safe path segment — reject a `../…`/absolute
                    // name before it reaches the emitter (SEC-1, defence-in-depth
                    // alongside the emit-sink containment check).
                    UntrustedRoot::validate_name(&plugin.name)?;
                    plugins.push(plugin);
                }
                PluginSource::RemoteGit {
                    kind,
                    url,
                    reference,
                } => {
                    let display_url = scrub_to_string(url.as_bytes());
                    if !fetch.enabled {
                        diagnostics.push(Diagnostic::warning(
                            rule::REMOTE_PLUGIN_SKIPPED,
                            format!(
                                "plugin `{label}` has a remote source ({kind}); skipped under --no-fetch"
                            ),
                        ));
                        continue;
                    }
                    match fetch_remote_plugin(&url, reference.as_deref(), pname.as_deref(), fetch) {
                        Ok(mut plugin) => {
                            // The marketplace entry `name` is the catalog identity; a
                            // differing fetched plugin.json name is surfaced + overridden.
                            // A fetched plugin.json name that disagrees with the marketplace
                            // entry is resolved in favor of the entry and is not strict-blocking.
                            let fetched_info = if let Some(entry_name) = &pname
                                && *entry_name != plugin.name
                            {
                                let old = std::mem::replace(&mut plugin.name, entry_name.clone());
                                format!(
                                    "plugin `{label}` fetched from {display_url} and vendored \
                                     (its plugin.json self-names `{old}`; the marketplace entry name wins)"
                                )
                            } else {
                                format!("plugin `{label}` fetched from {display_url} and vendored")
                            };
                            // Same SEC-1 defence as the relative path: the name becomes
                            // the emitted directory. Per-plugin forward-progress: an unsafe
                            // name skips THIS plugin (strict-blocking), never the whole catalog.
                            if let Err(e) = UntrustedRoot::validate_name(&plugin.name) {
                                diagnostics.push(Diagnostic::warning(
                                    rule::REMOTE_PLUGIN_FETCH_FAILED,
                                    format!("plugin `{label}` fetched but has an unsafe name: {e}"),
                                ));
                                continue;
                            }
                            diagnostics
                                .push(Diagnostic::info(rule::REMOTE_PLUGIN_FETCHED, fetched_info));
                            plugins.push(plugin);
                        }
                        // Forward-progress: a fetch/import failure skips THIS plugin
                        // only; the warning is strict-blocking.
                        Err(e) => diagnostics.push(Diagnostic::warning(
                            rule::REMOTE_PLUGIN_FETCH_FAILED,
                            format!("plugin `{label}` ({display_url}) could not be fetched: {e}"),
                        )),
                    }
                }
                PluginSource::RemoteUnfetchable(kind) => {
                    diagnostics.push(Diagnostic::warning(
                        rule::REMOTE_PLUGIN_SKIPPED,
                        format!(
                            "plugin `{label}` has a remote source ({kind}) Tome cannot fetch; it is skipped"
                        ),
                    ));
                }
                PluginSource::Malformed => diagnostics.push(Diagnostic::warning(
                    rule::REMOTE_PLUGIN_SKIPPED,
                    format!("plugin `{label}` has an unrecognized `source`; skipping it"),
                )),
            }
        }
    }

    Ok(CatalogIr {
        name,
        version,
        description,
        owner,
        plugins,
        provenance: Provenance {
            source_harness: "claude-code".to_owned(),
            source_path: source_path.to_path_buf(),
        },
        diagnostics,
    })
}

/// Classification of a marketplace `plugins[].source`.
enum PluginSource {
    /// A relative path within the marketplace repo (vendored inline).
    Relative(String),
    /// A git-fetchable remote: `github` (clone URL synthesized from `repo`),
    /// `git`/`url` (the URL as given — the dominant real-world shape is
    /// `{"source":"url","url":"https://github.com/….git"}`). `reference` is
    /// an optional `ref` pin passed to the shallow clone.
    RemoteGit {
        kind: String,
        url: String,
        reference: Option<String>,
    },
    /// A remote kind Tome cannot git-fetch (`npm`, unknown) — warned-and-skipped.
    RemoteUnfetchable(String),
    /// An unrecognized/absent source.
    Malformed,
}

/// A string `source` is a relative path; an object `source` is classified by
/// its `source` type field (`local`/`relative` → vendor; `github`/`git`/`url`
/// → git-fetchable; anything else → unfetchable remote).
fn classify_plugin_source(source: Option<&serde_json::Value>) -> PluginSource {
    match source {
        Some(serde_json::Value::String(s)) => PluginSource::Relative(s.clone()),
        Some(serde_json::Value::Object(o)) => {
            let reference = o.get("ref").and_then(|v| v.as_str()).map(str::to_owned);
            match o.get("source").and_then(|v| v.as_str()) {
                Some("local") | Some("relative") => match o.get("path").and_then(|v| v.as_str()) {
                    Some(p) => PluginSource::Relative(p.to_owned()),
                    None => PluginSource::Malformed,
                },
                Some("github") => match o.get("repo").and_then(|v| v.as_str()) {
                    Some(repo) => PluginSource::RemoteGit {
                        kind: "github".to_owned(),
                        url: format!("https://github.com/{repo}.git"),
                        reference,
                    },
                    None => PluginSource::Malformed,
                },
                Some(kind @ ("git" | "url")) => match o.get("url").and_then(|v| v.as_str()) {
                    Some(url) => PluginSource::RemoteGit {
                        kind: kind.to_owned(),
                        url: url.to_owned(),
                        reference,
                    },
                    None => PluginSource::Malformed,
                },
                Some(kind) => PluginSource::RemoteUnfetchable(kind.to_owned()),
                None => PluginSource::Malformed,
            }
        }
        _ => PluginSource::Malformed,
    }
}

/// URL schemes a marketplace remote may use — the conventional git set.
/// `file://` is deliberately ABSENT: a hostile marketplace could otherwise
/// vendor the operator's local private repos into the converted output
/// (read-disclosure). The hermetic tests opt back in via the
/// `#[doc(hidden)]` override below.
///
/// A URL outside this set (including anything starting with `-`) is refused
/// BEFORE it reaches the spawned git — the scheme check plus
/// `clone_shallow`'s `--` end-of-options marker together close git argument
/// injection.
const FETCH_URL_SCHEMES: &[&str] = &["https://", "http://", "git://", "ssh://", "git@"];

/// Test-only opt-in for `file://` remotes (integration tests can't see
/// `#[cfg(test)]`). Never set in production code paths.
#[doc(hidden)]
pub static ALLOW_FILE_URLS_FOR_TESTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn scheme_allowed(url: &str) -> bool {
    if FETCH_URL_SCHEMES.iter().any(|s| url.starts_with(s)) {
        return true;
    }
    url.starts_with("file://")
        && ALLOW_FILE_URLS_FOR_TESTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Shallow-clone a remote plugin source and import it as a Claude Code plugin.
/// The clone's TempDir is pushed onto the keepalive ONLY on success — a failed
/// fetch/import drops (cleans up) the clone immediately. Errors carry only
/// scrubbed URLs. `reference` rides `git clone --branch`, which accepts
/// branch/tag names only — a commit-SHA pin fails the clone and degrades to
/// the fetch-failed warning.
fn fetch_remote_plugin(
    url: &str,
    reference: Option<&str>,
    entry_name: Option<&str>,
    fetch: &mut FetchContext,
) -> Result<PluginIr, TomeError> {
    if !scheme_allowed(url) {
        return Err(TomeError::Usage(format!(
            "unsupported remote URL scheme (expected one of {})",
            FETCH_URL_SCHEMES.join(", ")
        )));
    }
    let tempdir = tempfile::Builder::new()
        .prefix("tome-fetch-")
        .tempdir()
        .map_err(TomeError::Io)?;
    let dest = tempdir.path().join("repo");
    let git = Git::new(scrub_to_string(url.as_bytes()));
    git.clone_shallow(url, &dest, reference)?;

    let plugin_root = UntrustedRoot::open(&dest)?;
    // The fetched repo must BE a plugin. A self-marketplace repo carrying both
    // manifests imports as a plugin here — the catalog context already decided
    // the level (no recursive marketplace expansion).
    if !plugin_root.is_file(Path::new(".claude-plugin/plugin.json")) {
        return Err(TomeError::Usage(
            "fetched repository has no .claude-plugin/plugin.json (not a Claude Code plugin)"
                .to_owned(),
        ));
    }
    let default = entry_name.unwrap_or("plugin");
    let plugin = import_plugin(&plugin_root, default, &dest)?;
    fetch.keepalive.push(tempdir);
    Ok(plugin)
}

/// Parse a CC marketplace `owner` (`{name, email}` object) into the required
/// [`Owner`], synthesizing missing fields with a diagnostic (the Tome catalog
/// manifest requires both).
fn parse_owner(value: Option<&serde_json::Value>, diagnostics: &mut Vec<Diagnostic>) -> Owner {
    let placeholder_email = "unknown@example.invalid";
    if let Some(obj) = value.and_then(|v| v.as_object()) {
        let name = obj
            .get("name")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let email = obj
            .get("email")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if let (Some(name), Some(email)) = (name.clone(), email.clone()) {
            return Owner { name, email };
        }
        diagnostics.push(Diagnostic::info(
            rule::CATALOG_SYNTHESIZED_FIELD,
            "marketplace `owner` is incomplete; synthesizing the missing field(s)",
        ));
        return Owner {
            name: name.unwrap_or_else(|| "unknown".to_owned()),
            email: email.unwrap_or_else(|| placeholder_email.to_owned()),
        };
    }
    diagnostics.push(Diagnostic::info(
        rule::CATALOG_SYNTHESIZED_FIELD,
        "marketplace has no `owner`; synthesizing one",
    ));
    Owner {
        name: "unknown".to_owned(),
        email: placeholder_email.to_owned(),
    }
}

/// Import each `skills/<name>/SKILL.md` directory into a skill [`EntryIr`].
/// A single malformed skill is skipped with a warning, never aborting the
/// plugin (`first_error` forward-progress).
fn import_skill_dir(
    root: &UntrustedRoot,
    entries: &mut Vec<EntryIr>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), TomeError> {
    if !root.is_dir(Path::new("skills")) {
        return Ok(());
    }
    for child in root.list_dir(Path::new("skills"))? {
        if !child.is_dir {
            continue;
        }
        match import_skill(root, &child.rel, &child.name) {
            Ok(entry) => entries.push(entry),
            Err(e) => diagnostics.push(Diagnostic::warning(
                rule::SKIPPED_ENTRY,
                format!("skipped skill `{}`: {e}", child.name),
            )),
        }
    }
    Ok(())
}

/// Import each `<dir>/<name>.md` file into a command/agent [`EntryIr`].
fn import_md_dir(
    root: &UntrustedRoot,
    dir: &str,
    kind: EntryKind,
    entries: &mut Vec<EntryIr>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), TomeError> {
    if !root.is_dir(Path::new(dir)) {
        return Ok(());
    }
    for child in root.list_dir(Path::new(dir))? {
        if child.is_dir || !child.name.ends_with(".md") {
            continue;
        }
        match import_md_entry(root, &child.rel, &child.name, kind) {
            Ok(entry) => entries.push(entry),
            Err(e) => diagnostics.push(Diagnostic::warning(
                rule::SKIPPED_ENTRY,
                format!("skipped {} `{}`: {e}", kind.as_str(), child.name),
            )),
        }
    }
    Ok(())
}

/// Build a skill entry from `<rel_dir>/SKILL.md`, validating the emitted name,
/// rewriting harness-isms, classifying dropped frontmatter, and collecting the
/// directory's other files as supporting files.
///
/// `pub(crate)` because the native-`SKILL.md` importers (Cursor/OpenCode/Cline/
/// generic Agent Skills) reuse it for a *bare* skill source (`rel_dir = ""`),
/// then apply any harness-specific supporting-path remap.
pub(crate) fn import_skill(
    root: &UntrustedRoot,
    rel_dir: &Path,
    dir_name: &str,
) -> Result<EntryIr, TomeError> {
    let skill_md = rel_dir.join("SKILL.md");
    let content = root.read_body(&skill_md)?;
    let parsed = parse_skill_frontmatter_str(&skill_md, &content)
        .map_err(|e| TomeError::Usage(e.to_string()))?;

    // The emitted skill directory == the resolved name (preserving name==dir);
    // it must be a safe path segment.
    let (name, _used_dir) = parsed.resolved_name(dir_name);
    UntrustedRoot::validate_name(&name)?;

    let mut diagnostics = Vec::new();
    classify_dropped_frontmatter(&content, EntryKind::Skill, &mut diagnostics);
    let rewritten = rewrite_body(&parsed.body, RewriteOptions::default());
    diagnostics.extend(rewritten.diagnostics);
    let description = resolved_description(&parsed.frontmatter, &rewritten.text);

    let supporting_files = collect_supporting(root, rel_dir, Some("SKILL.md"))?;

    Ok(EntryIr {
        kind: EntryKind::Skill,
        name,
        description: Some(description),
        frontmatter: parsed.frontmatter,
        agent_meta: None,
        body: rewritten.text,
        supporting_files,
        source_path: root.resolve(&skill_md)?,
        diagnostics,
    })
}

/// Build a command/agent entry from a single `<rel_file>` markdown file.
fn import_md_entry(
    root: &UntrustedRoot,
    rel_file: &Path,
    file_name: &str,
    kind: EntryKind,
) -> Result<EntryIr, TomeError> {
    let stem = file_name
        .strip_suffix(".md")
        .unwrap_or(file_name)
        .to_owned();
    UntrustedRoot::validate_name(&stem)?;

    let content = root.read_body(rel_file)?;
    let parsed = parse_skill_frontmatter_str(rel_file, &content)
        .map_err(|e| TomeError::Usage(e.to_string()))?;

    let (name, _used) = parsed.resolved_name(&stem);
    UntrustedRoot::validate_name(&name)?;

    let mut diagnostics = Vec::new();
    classify_dropped_frontmatter(&content, kind, &mut diagnostics);
    let rewritten = rewrite_body(
        &parsed.body,
        RewriteOptions {
            // Only Claude Code *commands* use legacy 1-based positionals.
            legacy_command_args: kind == EntryKind::Command,
        },
    );
    diagnostics.extend(rewritten.diagnostics);
    let description = resolved_description(&parsed.frontmatter, &rewritten.text);

    // G4: For agents, preserve the agent-specific frontmatter fields
    // (`model`, `tools`, `disallowedTools`, `permissionMode`) in `AgentMeta`
    // so that `harness sync` can translate them into per-harness native agent
    // files. Non-agent entries never carry this field.
    let agent_meta = if kind == EntryKind::Agent {
        parse_agent_meta(&content)
    } else {
        None
    };

    // G8: Warn when an agent body contains TOME_* substitution tokens that
    // the native-agent writer copies verbatim — the substitution layer only
    // fires on the MCP-served path. Non-agent entries are not affected because
    // their bodies are MCP-served (where substitution fires).
    if kind == EntryKind::Agent {
        let body = &rewritten.text;
        for token in UNRESOLVED_AGENT_TOKENS {
            if body.contains(token) {
                diagnostics.push(Diagnostic::warning(
                    rule::AGENT_UNRESOLVED_TOKEN,
                    format!(
                        "`{token}` in agent body will not be substituted by harness sync — \
                         this token is only resolved on the MCP-served path. Remove the token \
                         or replace it with a static path before syncing to a native agent harness."
                    ),
                ));
            }
        }
    }

    Ok(EntryIr {
        kind,
        name,
        description: Some(description),
        frontmatter: parsed.frontmatter,
        agent_meta,
        body: rewritten.text,
        supporting_files: Vec::new(),
        source_path: root.resolve(rel_file)?,
        diagnostics,
    })
}

/// Emit `Info`/`Warning` diagnostics for every source frontmatter key Tome does
/// not model (`data-model.md §6`).
///
/// For agent entries, the agent-specific keys (`model`, `tools`,
/// `allowed-tools`, `disallowed-tools`, `permissionMode`) are now preserved
/// via [`AgentMeta`] (G4), so they are classified as `Info` rather than
/// `Warning`. Tool restriction keys are also downgraded to `Info` for agents
/// since the data is preserved.
fn classify_dropped_frontmatter(content: &str, kind: EntryKind, diagnostics: &mut Vec<Diagnostic>) {
    for key in frontmatter_keys(content) {
        if MODELLED_FRONTMATTER.contains(&key.as_str()) {
            continue;
        }
        // Agent-specific keys are preserved via AgentMeta — emit Info only.
        if kind == EntryKind::Agent && AGENT_META_KEYS.contains(&key.as_str()) {
            diagnostics.push(Diagnostic::info(
                rule::AGENT_LOSSY,
                format!(
                    "agent frontmatter `{key}` is preserved in the converted agent file for harness sync"
                ),
            ));
        } else if TOOL_RESTRICTION_KEYS.contains(&key.as_str()) {
            // For non-agent entries (skills, commands) tool restrictions are
            // still truly dropped — they are `Warning`.
            diagnostics.push(Diagnostic::warning(
                rule::TOOL_RESTRICTION_DROPPED,
                format!(
                    "frontmatter `{key}` (a tool restriction) is dropped — Tome does not constrain tools, so dropping it silently broadens capability"
                ),
            ));
        } else if kind == EntryKind::Agent {
            diagnostics.push(Diagnostic::info(
                rule::AGENT_LOSSY,
                format!(
                    "agent frontmatter `{key}` is not modelled by Tome; dropping it (agent conversion is lossy)"
                ),
            ));
        } else {
            diagnostics.push(Diagnostic::info(
                rule::DROPPED_FRONTMATTER,
                format!("frontmatter `{key}` is not modelled by Tome; dropping it"),
            ));
        }
    }
}

/// Collect a skill directory's non-`SKILL.md` files as supporting files
/// (preserving `scripts/`/`references/`/`assets/` substructure), guard-validated
/// for containment + symlink refusal, with defensive depth/count bounds.
///
/// `exclude` names a single depth-0 file to skip (e.g. `Some("SKILL.md")` for
/// skill entries). Pass `None` when every depth-0 file should be collected
/// (e.g. the `hooks/` verbatim walk).
fn collect_supporting(
    root: &UntrustedRoot,
    rel_dir: &Path,
    exclude: Option<&str>,
) -> Result<Vec<SupportingFile>, TomeError> {
    let mut out = Vec::new();
    let mut dirs_visited = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(rel_dir.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_SUPPORTING_DEPTH {
            return Err(TomeError::Usage(format!(
                "source skill tree under {} exceeds the maximum supported depth ({MAX_SUPPORTING_DEPTH})",
                rel_dir.display()
            )));
        }
        dirs_visited += 1;
        if dirs_visited > MAX_SUPPORTING_DIRS {
            return Err(TomeError::Usage(format!(
                "source skill {} has more than {MAX_SUPPORTING_DIRS} directories",
                rel_dir.display()
            )));
        }
        for child in root.list_dir(&dir)? {
            // Skip the entry's own primary file at depth 0 (e.g. SKILL.md is
            // rendered, not copied).
            if depth == 0 && Some(child.name.as_str()) == exclude {
                continue;
            }
            // Never copy VCS metadata / OS junk into the converted artifact.
            if SKIP_SUPPORTING_NAMES.contains(&child.name.as_str()) {
                continue;
            }
            if child.is_dir {
                stack.push((child.rel, depth + 1));
                continue;
            }
            if out.len() >= MAX_SUPPORTING_FILES {
                return Err(TomeError::Usage(format!(
                    "source skill {} has more than {MAX_SUPPORTING_FILES} supporting files",
                    rel_dir.display()
                )));
            }
            let relative = child
                .rel
                .strip_prefix(rel_dir)
                .unwrap_or(&child.rel)
                .to_path_buf();
            out.push(SupportingFile {
                relative,
                source: root.resolve(&child.rel)?,
            });
        }
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

/// Top-level plugin-root names that this importer explicitly handles. Entries
/// that map to `UNSUPPORTED_COMPONENTS` or `settings.json` are NOT listed here
/// — instead `warn_unrecognised_plugin_root_entries` checks against
/// `UNSUPPORTED_COMPONENTS` structurally so that a future addition to that
/// array is automatically silent here without a manual mirror.
const KNOWN_PLUGIN_ROOT_NAMES: &[&str] = &[
    // Handled importer entry-point directories.
    "skills",
    "commands",
    "agents",
    "hooks",
    // Handled files/dirs.
    ".claude-plugin",
    ".mcp.json",
    // Already warned via the settings.json explicit check above.
    "settings.json",
];

/// Root-level documentation, VCS metadata, and build files that CC plugins
/// commonly ship and that are never referenced via `${CLAUDE_PLUGIN_ROOT}` —
/// skip silently so the unrecognised-entry warning stays signal, not noise.
const SKIP_PLUGIN_ROOT_FILES: &[&str] = &[
    "README.md",
    "README",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "CHANGELOG.md",
    "CHANGELOG",
    "NOTICE",
    ".gitignore",
    ".gitattributes",
    "Makefile",
    ".editorconfig",
];

/// Scan the plugin root for top-level entries that are not in the known set
/// (neither a handled directory/file, nor an `UNSUPPORTED_COMPONENTS` entry,
/// nor a common documentation/metadata file) and emit an actionable
/// [`Diagnostic::warning`] for each one.
///
/// This surfaces unrecognised support directories (e.g. `scripts/`, `lib/`)
/// that hooks or commands commonly reference via
/// `${CLAUDE_PLUGIN_ROOT}/scripts/…` — without this warning they vanish
/// silently from the conversion and the reference breaks at runtime.
fn warn_unrecognised_plugin_root_entries(
    root: &UntrustedRoot,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), TomeError> {
    // list_dir on the root itself (Path::new("")) enumerates only depth-0 entries,
    // sorted, with symlink refusal already enforced.
    let children = root.list_dir(Path::new(""))?;
    for child in children {
        if KNOWN_PLUGIN_ROOT_NAMES.contains(&child.name.as_str()) {
            continue;
        }
        // Skip VCS metadata and OS junk (same guard as collect_supporting).
        if SKIP_SUPPORTING_NAMES.contains(&child.name.as_str()) {
            continue;
        }
        // Already handled by the UNSUPPORTED_COMPONENTS pass — skip to avoid
        // double-warning. This check is structural so a new UNSUPPORTED_COMPONENTS
        // entry is automatically silent here without mirroring it into
        // KNOWN_PLUGIN_ROOT_NAMES.
        if UNSUPPORTED_COMPONENTS
            .iter()
            .any(|(name, _)| *name == child.name.as_str())
        {
            continue;
        }
        // Skip common documentation/build files that are never
        // ${CLAUDE_PLUGIN_ROOT}-referenced and would produce a false-positive
        // "will break at runtime" warning if left to fall through.
        let lower = child.name.to_lowercase();
        if SKIP_PLUGIN_ROOT_FILES.contains(&child.name.as_str())
            || lower.starts_with("readme")
            || lower.starts_with("license")
            || lower.starts_with("changelog")
        {
            continue;
        }
        let kind_label = if child.is_dir { "directory" } else { "file" };
        let display = if child.is_dir {
            format!("{}/", child.name)
        } else {
            child.name.clone()
        };
        diagnostics.push(Diagnostic::warning(
            rule::UNRECOGNISED_PLUGIN_DIR,
            format!(
                "{kind_label} '{display}' was not imported; commands or hooks referencing \
                 ${{CLAUDE_PLUGIN_ROOT}}/{display} will break at runtime after conversion"
            ),
        ));
    }
    Ok(())
}

/// Collect the `hooks/` subtree for verbatim pass-through. Reuses the
/// supporting-file walk (bounded, symlink-refusing, VCS-junk-skipping); rel
/// paths are re-prefixed `hooks/` so emit plans them at the plugin root.
/// `hooks/hooks.json`'s text is also carried (when readable) for the lint
/// hooks-spec rule.
fn collect_hooks(
    root: &UntrustedRoot,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Vec<SupportingFile>, Option<String>), TomeError> {
    let hooks_dir = Path::new("hooks");
    if !root.is_dir(hooks_dir) {
        return Ok((Vec::new(), None));
    }
    let mut files = collect_supporting(root, hooks_dir, None)?;
    for f in &mut files {
        f.relative = hooks_dir.join(&f.relative);
    }
    let json = if root.is_file(Path::new("hooks/hooks.json")) {
        // hooks.json shares the harness-config read cap (1 MiB) — same semantic class as .mcp.json.
        match root.read_text(Path::new("hooks/hooks.json"), HARNESS_MCP_MAX) {
            Ok(s) => Some(normalize_hooks_json(s)),
            Err(e) => {
                // Copied verbatim regardless, but Tome cannot validate it —
                // surfaced as strict-blocking honesty.
                diagnostics.push(Diagnostic::warning(
                    rule::HOOKS_UNREADABLE,
                    format!(
                        "hooks/hooks.json could not be read as UTF-8 text ({e}); it is copied verbatim but not validated"
                    ),
                ));
                None
            }
        }
    } else {
        None
    };
    Ok((files, json))
}

/// Detect and unwrap the Claude Code wrapped hooks.json form.
///
/// Claude Code marketplace material uses two top-level shapes for `hooks.json`:
///
/// - **Event-map** (what `harness sync` requires):
///   `{"PreToolUse": [...], "PostToolUse": [...]}`
/// - **Wrapped** (also seen in CC material):
///   `{"description": "...", "hooks": {"PreToolUse": [...]}}`
///
/// When the text parses as JSON and the top-level object has exactly a `"hooks"`
/// key whose value is itself an object, the content inside that key IS the
/// event-map — unwrap it and re-serialise. This ensures the emitted
/// `hooks/hooks.json` is always in the event-map form that `harness sync`
/// expects, so a converted plugin doesn't fail at exit 43.
///
/// Any other shape (valid or invalid JSON, plain event-map, non-object `"hooks"`
/// value) is returned verbatim — the lint rule will signal the bad shapes.
fn normalize_hooks_json(raw: String) -> String {
    // Only attempt parse when the string looks like an object; non-JSON is
    // returned verbatim so the HooksSpec lint rule catches it.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return raw;
    };
    let Some(obj) = value.as_object() else {
        return raw;
    };
    // The wrapped form is an object that has a "hooks" key whose value is an
    // object (the event-map). We must not accidentally unwrap a native
    // event-map that happens to have a hook named "hooks" — but the CC spec
    // names hooks after events ("PreToolUse", "PostToolUse", …) so a top-level
    // "hooks" key is the wrapper discriminator.
    if let Some(inner) = obj.get("hooks").filter(|v| v.is_object()) {
        // Re-serialise the inner event-map as the canonical form.
        if let Ok(normalised) = serde_json::to_string_pretty(inner) {
            return normalised;
        }
    }
    raw
}

/// Synthesize the plugin's MCP servers from `.mcp.json` (CC format), inferring
/// transport from `command` (stdio) vs `url` (http). Lenient parse.
fn import_mcp(
    root: &UntrustedRoot,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<McpServerIr>, TomeError> {
    if !root.is_file(Path::new(".mcp.json")) {
        return Ok(Vec::new());
    }
    let content = root.read_text(Path::new(".mcp.json"), HARNESS_MCP_MAX)?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| TomeError::Usage(format!("source .mcp.json is not valid JSON: {e}")))?;
    let Some(servers) = value.get("mcpServers").and_then(|v| v.as_object()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (name, cfg) in servers {
        if let Some(command) = cfg.get("command").and_then(|v| v.as_str()) {
            let args = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let env = cfg
                .get("env")
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            out.push(McpServerIr {
                name: name.clone(),
                transport: McpTransport::Stdio {
                    command: command.to_owned(),
                    args,
                    env,
                },
            });
        } else if let Some(url) = cfg.get("url").and_then(|v| v.as_str()) {
            out.push(McpServerIr {
                name: name.clone(),
                transport: McpTransport::Http {
                    url: url.to_owned(),
                },
            });
        } else {
            diagnostics.push(Diagnostic::warning(
                rule::MALFORMED_MCP,
                format!("MCP server `{name}` has neither `command` nor `url`; skipping it"),
            ));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Parse a CC `author` value (a `"Name <email>"` string or a `{name, email}`
/// object) into a [`TomeAuthor`]. Returns `None` when nothing usable is present.
fn parse_author(value: Option<&serde_json::Value>) -> Option<TomeAuthor> {
    let value = value?;
    if let Some(s) = value.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        // "Name <email>" — split on the angle brackets if present.
        if let (Some(lt), Some(gt)) = (s.find('<'), s.rfind('>'))
            && lt < gt
        {
            let name = s[..lt].trim();
            let email = s[lt + 1..gt].trim();
            return Some(TomeAuthor {
                name: (!name.is_empty()).then(|| name.to_owned()),
                email: (!email.is_empty()).then(|| email.to_owned()),
            });
        }
        return Some(TomeAuthor {
            name: Some(s.to_owned()),
            email: None,
        });
    }
    if let Some(obj) = value.as_object() {
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let email = obj
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if name.is_none() && email.is_none() {
            return None;
        }
        return Some(TomeAuthor { name, email });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::ir::Severity;
    use std::fs;

    /// Write a minimal CC plugin fixture and open an `UntrustedRoot` over it.
    fn cc_plugin(setup: impl FnOnce(&Path)) -> (tempfile::TempDir, UntrustedRoot) {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        fs::create_dir(base.join(".claude-plugin")).unwrap();
        setup(&base);
        let root = UntrustedRoot::open(&base).unwrap();
        (tmp, root)
    }

    fn has(diags: &[Diagnostic], rule_id: &str) -> bool {
        diags.iter().any(|d| d.rule_id == rule_id)
    }

    #[test]
    fn imports_manifest_fields_and_defaults_version() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"my-plugin","description":"d","author":"Ada <ada@x.io>"}"#,
            )
            .unwrap();
        });
        let p = import_plugin(&root, "fallback", Path::new("/src")).unwrap();
        assert_eq!(p.name, "my-plugin");
        assert_eq!(p.version, "0.0.0");
        assert_eq!(p.description.as_deref(), Some("d"));
        let author = p.author.unwrap();
        assert_eq!(author.name.as_deref(), Some("Ada"));
        assert_eq!(author.email.as_deref(), Some("ada@x.io"));
        assert!(has(&p.diagnostics, rule::MISSING_VERSION));
    }

    #[test]
    fn falls_back_to_default_name_when_missing() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(base.join(".claude-plugin/plugin.json"), b"{}").unwrap();
        });
        let p = import_plugin(&root, "derived-name", Path::new("/src")).unwrap();
        assert_eq!(p.name, "derived-name");
    }

    #[test]
    fn imports_a_skill_and_rewrites_harness_isms() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("skills/foo/scripts")).unwrap();
            fs::write(
                base.join("skills/foo/SKILL.md"),
                "---\nname: foo\ndescription: a skill\nallowed-tools: Bash\n---\nRun ${CLAUDE_PLUGIN_ROOT}/x\n",
            )
            .unwrap();
            fs::write(base.join("skills/foo/scripts/run.sh"), b"#!/bin/sh\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        assert_eq!(p.entries.len(), 1);
        let e = &p.entries[0];
        assert_eq!(e.kind, EntryKind::Skill);
        assert_eq!(e.name, "foo");
        assert!(e.body.contains("${TOME_PLUGIN_DIR}/x"), "body: {}", e.body);
        assert!(!e.body.contains("CLAUDE_PLUGIN_ROOT"));
        // allowed-tools dropped with a Warning.
        assert!(has(&e.diagnostics, rule::TOOL_RESTRICTION_DROPPED));
        assert!(e.diagnostics.iter().any(
            |d| d.rule_id == rule::TOOL_RESTRICTION_DROPPED && d.severity == Severity::Warning
        ));
        // The supporting script is collected under its relative path.
        assert_eq!(e.supporting_files.len(), 1);
        assert_eq!(
            e.supporting_files[0].relative,
            PathBuf::from("scripts/run.sh")
        );
    }

    #[test]
    fn command_gets_legacy_positional_rewrite_agent_meta_is_preserved() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir(base.join("commands")).unwrap();
            fs::write(
                base.join("commands/do.md"),
                "---\nname: do\n---\nUse $1 and $2\n",
            )
            .unwrap();
            fs::create_dir(base.join("agents")).unwrap();
            fs::write(
                base.join("agents/helper.md"),
                "---\nname: helper\ndescription: h\nmodel: opus\ntools: Bash\n---\nbody\n",
            )
            .unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let cmd = p
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::Command)
            .unwrap();
        assert_eq!(cmd.body, "Use $0 and $1\n");
        let agent = p
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::Agent)
            .unwrap();
        // model + tools are now preserved via AgentMeta (G4) — they emit
        // Info-level agent-lossy diagnostics, not Warnings, since the data
        // is retained through the pipeline.
        let lossy = agent
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::AGENT_LOSSY)
            .count();
        assert_eq!(lossy, 2, "model + tools both produce info diagnostics");
        // Both must be Info (preserved, not dropped).
        use crate::authoring::ir::Severity;
        assert!(
            agent
                .diagnostics
                .iter()
                .filter(|d| d.rule_id == rule::AGENT_LOSSY)
                .all(|d| d.severity == Severity::Info),
            "preserved agent-meta keys must be Info, not Warning"
        );
        // AgentMeta is populated for the agent entry.
        let meta = agent.agent_meta.as_ref().expect("agent_meta should be set");
        assert_eq!(meta.model.as_deref(), Some("opus"));
        assert_eq!(meta.tools.as_deref(), Some(&["Bash".to_owned()][..]));
    }

    #[test]
    fn warns_on_unsupported_components_and_exotic_manifest_fields() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0","userConfig":{"k":1},"displayName":"P"}"#,
            )
            .unwrap();
            fs::create_dir(base.join("monitors")).unwrap();
            fs::write(base.join("settings.json"), b"{}").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        assert!(has(&p.diagnostics, rule::UNSUPPORTED_COMPONENT)); // monitors/ + settings.json
        assert!(has(&p.diagnostics, rule::UNSUPPORTED_MANIFEST_FIELD)); // userConfig
        assert!(has(&p.diagnostics, rule::DROPPED_MANIFEST_FIELD)); // displayName
    }

    #[test]
    fn synthesizes_mcp_servers_from_mcp_json() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::write(
                base.join(".mcp.json"),
                br#"{"mcpServers":{"local":{"command":"node","args":["s.js"]},"remote":{"url":"https://x/mcp"}}}"#,
            )
            .unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        assert_eq!(p.mcp_servers.len(), 2);
        // Sorted by name: local then remote.
        assert_eq!(p.mcp_servers[0].name, "local");
        assert!(matches!(
            p.mcp_servers[0].transport,
            McpTransport::Stdio { .. }
        ));
        assert!(matches!(
            p.mcp_servers[1].transport,
            McpTransport::Http { .. }
        ));
    }

    #[test]
    fn malformed_skill_is_skipped_with_a_warning_not_an_abort() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("skills/good")).unwrap();
            fs::write(
                base.join("skills/good/SKILL.md"),
                "---\nname: good\ndescription: g\n---\nok\n",
            )
            .unwrap();
            // A skill with no frontmatter delimiters — should be skipped, not fatal.
            fs::create_dir_all(base.join("skills/bad")).unwrap();
            fs::write(base.join("skills/bad/SKILL.md"), "no frontmatter here").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        assert_eq!(p.entries.len(), 1, "only the good skill imports");
        assert_eq!(p.entries[0].name, "good");
        assert!(has(&p.diagnostics, rule::SKIPPED_ENTRY));
    }

    #[test]
    fn hooks_pass_through_verbatim_not_unsupported() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("hooks/scripts")).unwrap();
            fs::write(
                base.join("hooks/hooks.json"),
                br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/hooks/scripts/run.sh"}]}]}}"#,
            )
            .unwrap();
            fs::write(base.join("hooks/scripts/run.sh"), b"#!/bin/sh\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        // hooks/ is NOT an unsupported component any more.
        assert!(
            !p.diagnostics
                .iter()
                .any(|d| d.rule_id == rule::UNSUPPORTED_COMPONENT && d.message.contains("hooks")),
            "{:?}",
            p.diagnostics
        );
        // The whole subtree is collected, hooks/-prefixed, sorted.
        let rels: Vec<_> = p
            .hooks_files
            .iter()
            .map(|f| f.relative.display().to_string())
            .collect();
        assert_eq!(rels, ["hooks/hooks.json", "hooks/scripts/run.sh"]);
        // hooks.json is normalised: the wrapped form is unwrapped to the event-map
        // so `harness sync` never sees the exit-43 shape.  The ${CLAUDE_PLUGIN_ROOT}
        // token is preserved intact inside the normalised JSON (the sync-time
        // rewriter still owns it).
        let hj = p.hooks_json.as_deref().unwrap();
        assert!(
            hj.contains("${CLAUDE_PLUGIN_ROOT}"),
            "token must survive normalisation: {hj}"
        );
        // After unwrapping, the top-level no longer contains the "hooks" wrapper key.
        let v: serde_json::Value = serde_json::from_str(hj).unwrap();
        assert!(
            !v.as_object().unwrap().contains_key("hooks"),
            "normalised JSON must be an event-map (no wrapper key): {hj}"
        );
        assert!(
            v.as_object().unwrap().contains_key("SessionStart"),
            "normalised JSON must expose the event directly: {hj}"
        );
    }

    #[test]
    fn invalid_json_manifest_is_a_usage_error() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(base.join(".claude-plugin/plugin.json"), b"{not json").unwrap();
        });
        let err = import_plugin(&root, "p", Path::new("/src")).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn parse_author_handles_object_and_string_and_empty() {
        assert!(parse_author(None).is_none());
        assert!(parse_author(Some(&serde_json::json!({}))).is_none());
        let a = parse_author(Some(&serde_json::json!({"name":"Bo","email":"bo@x"}))).unwrap();
        assert_eq!(a.name.as_deref(), Some("Bo"));
        assert_eq!(a.email.as_deref(), Some("bo@x"));
        let b = parse_author(Some(&serde_json::json!("Just A Name"))).unwrap();
        assert_eq!(b.name.as_deref(), Some("Just A Name"));
        assert!(b.email.is_none());
    }

    #[test]
    fn classify_plugin_source_distinguishes_relative_fetchable_and_unfetchable() {
        // A string source is a relative (vendored) path.
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!("./alpha"))),
            PluginSource::Relative(p) if p == "./alpha"
        ));
        // A local object source uses its `path`.
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!({"source":"local","path":"x"}))),
            PluginSource::Relative(p) if p == "x"
        ));
        // github synthesizes a clone URL from `repo` and honours a `ref` pin.
        assert!(matches!(
            classify_plugin_source(Some(
                &serde_json::json!({"source":"github","repo":"o/r","ref":"v1"})
            )),
            PluginSource::RemoteGit { kind, url, reference }
                if kind == "github" && url == "https://github.com/o/r.git"
                    && reference.as_deref() == Some("v1")
        ));
        // git + url kinds carry their URL as given (the real-world obra shape).
        assert!(matches!(
            classify_plugin_source(Some(
                &serde_json::json!({"source":"url","url":"https://github.com/o/r.git"})
            )),
            PluginSource::RemoteGit { kind, url, .. }
                if kind == "url" && url == "https://github.com/o/r.git"
        ));
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!({"source":"git","url":"u"}))),
            PluginSource::RemoteGit { kind, .. } if kind == "git"
        ));
        // npm cannot be git-fetched.
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!({"source":"npm","package":"p"}))),
            PluginSource::RemoteUnfetchable(k) if k == "npm"
        ));
        // Missing required fields / absent source → malformed.
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!({"source":"github"}))),
            PluginSource::Malformed
        ));
        assert!(matches!(
            classify_plugin_source(Some(&serde_json::json!({"source":"url"}))),
            PluginSource::Malformed
        ));
        assert!(matches!(
            classify_plugin_source(None),
            PluginSource::Malformed
        ));
    }

    #[test]
    fn parse_owner_synthesizes_missing_fields() {
        let mut diags = Vec::new();
        let full = parse_owner(
            Some(&serde_json::json!({"name":"O","email":"o@x.io"})),
            &mut diags,
        );
        assert_eq!(full.name, "O");
        assert_eq!(full.email, "o@x.io");
        assert!(diags.is_empty());

        let none = parse_owner(None, &mut diags);
        assert_eq!(none.name, "unknown");
        assert!(!diags.is_empty());
    }

    #[test]
    fn skill_without_description_falls_back_to_the_rewritten_body() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("skills/foo")).unwrap();
            // No `description`; body carries a harness-ism that gets rewritten.
            fs::write(
                base.join("skills/foo/SKILL.md"),
                "---\nname: foo\n---\nUse ${CLAUDE_PLUGIN_ROOT}/x\n",
            )
            .unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let desc = p.entries[0].description.as_deref().unwrap();
        assert!(
            desc.contains("${TOME_PLUGIN_DIR}/x"),
            "fallback uses the rewritten body: {desc}"
        );
        assert!(!desc.contains("CLAUDE_PLUGIN_ROOT"));
    }

    // --- normalize_hooks_json -------------------------------------------------

    #[test]
    fn normalize_hooks_json_unwraps_wrapped_form() {
        // The wrapped form has a top-level "hooks" key whose value is the event-map.
        let wrapped = r#"{"hooks":{"PreToolUse":[{"type":"command","command":"run.sh"}]}}"#;
        let out = normalize_hooks_json(wrapped.to_owned());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let obj = v.as_object().unwrap();
        assert!(
            !obj.contains_key("hooks"),
            "wrapper must be stripped: {out}"
        );
        assert!(
            obj.contains_key("PreToolUse"),
            "event must be at top-level: {out}"
        );
    }

    #[test]
    fn normalize_hooks_json_unwraps_with_description_sibling() {
        // Some CC plugins include a description alongside the hooks key.
        let wrapped = r#"{"description":"desc","hooks":{"SessionStart":[]}}"#;
        let out = normalize_hooks_json(wrapped.to_owned());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let obj = v.as_object().unwrap();
        assert!(
            !obj.contains_key("hooks"),
            "wrapper must be stripped: {out}"
        );
        assert!(
            !obj.contains_key("description"),
            "description sibling is in the wrapper: {out}"
        );
        assert!(
            obj.contains_key("SessionStart"),
            "event must be at top-level: {out}"
        );
    }

    #[test]
    fn normalize_hooks_json_leaves_event_map_unchanged() {
        // An already-unwrapped event-map must pass through verbatim.
        let event_map = r#"{"PreToolUse":[{"type":"command"}],"PostToolUse":[]}"#;
        let out = normalize_hooks_json(event_map.to_owned());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let obj = v.as_object().unwrap();
        assert!(
            obj.contains_key("PreToolUse"),
            "PreToolUse must survive: {out}"
        );
        assert!(
            obj.contains_key("PostToolUse"),
            "PostToolUse must survive: {out}"
        );
    }

    #[test]
    fn normalize_hooks_json_preserves_variable_tokens() {
        // ${CLAUDE_PLUGIN_ROOT} must survive unwrapping so sync-time rewrite still works.
        let wrapped = r#"{"hooks":{"SessionStart":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/run.sh"}]}}"#;
        let out = normalize_hooks_json(wrapped.to_owned());
        assert!(
            out.contains("${CLAUDE_PLUGIN_ROOT}"),
            "token must survive normalisation: {out}"
        );
    }

    #[test]
    fn normalize_hooks_json_returns_invalid_json_verbatim() {
        let invalid = "{not valid json";
        let out = normalize_hooks_json(invalid.to_owned());
        assert_eq!(out, invalid);
    }

    #[test]
    fn collect_hooks_normalises_wrapped_form_via_import_plugin() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("hooks")).unwrap();
            fs::write(
                base.join("hooks/hooks.json"),
                // Wrapped form — the kind that causes exit 43 in harness sync.
                br#"{"description":"my hooks","hooks":{"PreToolUse":[{"type":"command","command":"${CLAUDE_PLUGIN_ROOT}/run.sh"}]}}"#,
            )
            .unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let hj = p.hooks_json.as_deref().expect("hooks_json must be set");
        let v: serde_json::Value = serde_json::from_str(hj).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("hooks"), "wrapper must be stripped: {hj}");
        assert!(
            !obj.contains_key("description"),
            "description must be stripped: {hj}"
        );
        assert!(
            obj.contains_key("PreToolUse"),
            "event must be at top-level: {hj}"
        );
        assert!(
            hj.contains("${CLAUDE_PLUGIN_ROOT}"),
            "token must survive: {hj}"
        );
    }

    // --- unrecognised plugin-root dirs (closes #523) -------------------------

    #[test]
    fn warns_on_unrecognised_top_level_dir() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            // `scripts/` is a common support directory that was previously
            // silently dropped; it should now produce an actionable warning.
            fs::create_dir_all(base.join("scripts")).unwrap();
            fs::write(base.join("scripts/run.sh"), b"#!/bin/sh\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        // Must have exactly one UNRECOGNISED_PLUGIN_DIR warning naming "scripts/".
        let unrecognised: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::UNRECOGNISED_PLUGIN_DIR)
            .collect();
        assert_eq!(
            unrecognised.len(),
            1,
            "expected exactly one unrecognised-dir diagnostic, got: {:?}",
            p.diagnostics
        );
        assert_eq!(unrecognised[0].severity, Severity::Warning);
        assert!(
            unrecognised[0].message.contains("scripts/"),
            "message must name the directory: {}",
            unrecognised[0].message
        );
        assert!(
            unrecognised[0]
                .message
                .contains("${CLAUDE_PLUGIN_ROOT}/scripts/"),
            "message must show the broken reference path: {}",
            unrecognised[0].message
        );
    }

    #[test]
    fn warns_on_multiple_unrecognised_top_level_entries() {
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::create_dir_all(base.join("scripts")).unwrap();
            fs::write(base.join("scripts/run.sh"), b"#!/bin/sh\n").unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            fs::write(base.join("lib/helper.py"), b"# helper\n").unwrap();
            // A top-level file (not a dir) that is also unrecognised.
            fs::write(base.join("extra.sh"), b"#!/bin/sh\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let unrecognised: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::UNRECOGNISED_PLUGIN_DIR)
            .collect();
        // scripts/, lib/, and extra.sh each produce a warning.
        assert_eq!(
            unrecognised.len(),
            3,
            "expected three unrecognised-dir diagnostics, got: {:?}",
            unrecognised
        );
        let messages: Vec<&str> = unrecognised.iter().map(|d| d.message.as_str()).collect();
        assert!(messages.iter().any(|m| m.contains("scripts/")));
        assert!(messages.iter().any(|m| m.contains("lib/")));
        // extra.sh is a file, not a dir; message should not append a trailing slash.
        assert!(messages.iter().any(|m| m.contains("extra.sh")));
    }

    #[test]
    fn known_dirs_do_not_produce_unrecognised_warnings() {
        // skills/, commands/, agents/, hooks/, .claude-plugin/, .mcp.json,
        // every UNSUPPORTED_COMPONENTS entry, settings.json, .git, and common
        // documentation files must all be silent — no UNRECOGNISED_PLUGIN_DIR
        // diagnostic.  This test is structural: it creates a fixture that covers
        // every branch of the skip logic and asserts zero warnings.
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            // Handled entry-point directories.
            fs::create_dir(base.join("skills")).unwrap();
            fs::create_dir(base.join("commands")).unwrap();
            fs::create_dir(base.join("agents")).unwrap();
            fs::create_dir(base.join("hooks")).unwrap();
            // Handled files.
            fs::write(base.join(".mcp.json"), br#"{"mcpServers":{}}"#).unwrap();
            // All UNSUPPORTED_COMPONENTS entries (structural, not manual mirror).
            for (name, _) in UNSUPPORTED_COMPONENTS {
                fs::create_dir(base.join(name)).unwrap();
            }
            // settings.json is warned by the explicit check, not UNSUPPORTED_COMPONENTS.
            fs::write(base.join("settings.json"), b"{}").unwrap();
            // VCS metadata: covered by SKIP_SUPPORTING_NAMES.
            fs::create_dir(base.join(".git")).unwrap();
            // Common documentation files: covered by SKIP_PLUGIN_ROOT_FILES.
            fs::write(base.join("README.md"), b"# readme\n").unwrap();
            fs::write(base.join("LICENSE"), b"MIT\n").unwrap();
            fs::write(base.join("CHANGELOG.md"), b"## Changes\n").unwrap();
            fs::write(base.join(".gitignore"), b"target/\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let unrecognised: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::UNRECOGNISED_PLUGIN_DIR)
            .collect();
        assert!(
            unrecognised.is_empty(),
            "no UNRECOGNISED_PLUGIN_DIR expected for known/skip entries, got: {:?}",
            unrecognised
        );
    }

    #[test]
    fn common_doc_files_do_not_produce_unrecognised_warnings() {
        // README.md, LICENSE, .gitignore etc. must be silently skipped even when
        // they are the ONLY extra root-level files — the warning message "will
        // break at runtime" would be a false positive for documentation files.
        let (_t, root) = cc_plugin(|base| {
            fs::write(
                base.join(".claude-plugin/plugin.json"),
                br#"{"name":"p","version":"1.0.0"}"#,
            )
            .unwrap();
            fs::write(base.join("README.md"), b"# My plugin\n").unwrap();
            fs::write(base.join("LICENSE"), b"MIT License\n").unwrap();
            fs::write(base.join("LICENSE.txt"), b"MIT License\n").unwrap();
            fs::write(base.join(".gitignore"), b"target/\n").unwrap();
            fs::write(base.join("CHANGELOG.md"), b"## Changes\n").unwrap();
            fs::write(base.join(".editorconfig"), b"[*]\n").unwrap();
        });
        let p = import_plugin(&root, "p", Path::new("/src")).unwrap();
        let unrecognised: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::UNRECOGNISED_PLUGIN_DIR)
            .collect();
        assert!(
            unrecognised.is_empty(),
            "documentation files must not produce UNRECOGNISED_PLUGIN_DIR warnings, got: {:?}",
            unrecognised
        );
    }
}
