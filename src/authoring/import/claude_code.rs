//! Claude Code → Tome IR importer (Tier 1, FR-010/FR-012/FR-013).
//!
//! Reads a Claude Code plugin directory — `.claude-plugin/plugin.json` plus the
//! conventional `skills/`, `commands/`, `agents/` trees and an optional
//! `.mcp.json` — through the [`UntrustedRoot`] guard and produces a
//! [`PluginIr`]. Honest by construction:
//!
//! * manifest fields map 1:1 where Tome models them; dropped fields surface as
//!   `Info`, exotic fields (`userConfig`/`dependencies`) as `Warning`
//!   (`data-model.md §1`);
//! * unsupported component directories (`monitors/`, `themes/`, `lsp/`, …) and
//!   plugin `settings.json`/`hooks/` surface as `Warning`s (FR-012, §8);
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
    CatalogIr, Diagnostic, EntryIr, McpServerIr, McpTransport, PluginIr, Provenance, SupportingFile,
};
use crate::authoring::rewrite::{RewriteOptions, rewrite_body};
use crate::authoring::untrusted::UntrustedRoot;
use crate::catalog::manifest::Owner;
use crate::error::TomeError;
use crate::plugin::frontmatter::{frontmatter_keys, parse_skill_frontmatter_str};
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomeAuthor;
use crate::util::{HARNESS_MCP_MAX, PLUGIN_MANIFEST_MAX};

// The diagnostic rule ids this importer emits live in the shared
// `super::rule` SSOT (promoted when Codex became the second consumer).
use super::rule;

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

/// Frontmatter keys whose loss silently broadens capability — always a Warning.
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

/// Unsupported component directories/files (FR-012, §8): present ⇒ Warning.
const UNSUPPORTED_COMPONENTS: &[(&str, &str)] = &[
    ("monitors", "monitors"),
    ("themes", "themes"),
    ("lsp", "LSP servers"),
    ("output-styles", "output styles"),
    ("channels", "channels"),
    ("bin", "`bin/` executables"),
    ("hooks", "hooks"),
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
/// all-or-nothing, FR-014a). Remote-source plugins (github/git/url/npm) are
/// warned-and-skipped, and that warning is strict-blocking so `--strict`
/// hard-fails them (FR-014).
pub fn import_marketplace(
    root: &UntrustedRoot,
    source_path: &Path,
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
                // TEMPORARY (replaced by the fetch task): both remote variants
                // keep the historical warn-and-skip until fetching lands.
                PluginSource::RemoteGit { kind, .. }
                | PluginSource::RemoteUnfetchable(kind) => diagnostics.push(Diagnostic::warning(
                    rule::REMOTE_PLUGIN_SKIPPED,
                    format!(
                        "plugin `{label}` has a remote source ({kind}); Tome catalogs are relative-path-only, so it is skipped"
                    ),
                )),
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
// `RemoteGit.url` and `RemoteGit.reference` are read by the fetch task (Task 3).
#[allow(dead_code)]
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

    let supporting_files = collect_supporting(root, rel_dir, "SKILL.md")?;

    Ok(EntryIr {
        kind: EntryKind::Skill,
        name,
        description: Some(description),
        frontmatter: parsed.frontmatter,
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

    Ok(EntryIr {
        kind,
        name,
        description: Some(description),
        frontmatter: parsed.frontmatter,
        body: rewritten.text,
        supporting_files: Vec::new(),
        source_path: root.resolve(rel_file)?,
        diagnostics,
    })
}

/// Emit `Info`/`Warning` diagnostics for every source frontmatter key Tome does
/// not model (`data-model.md §6`).
fn classify_dropped_frontmatter(content: &str, kind: EntryKind, diagnostics: &mut Vec<Diagnostic>) {
    for key in frontmatter_keys(content) {
        if MODELLED_FRONTMATTER.contains(&key.as_str()) {
            continue;
        }
        if TOOL_RESTRICTION_KEYS.contains(&key.as_str()) {
            diagnostics.push(Diagnostic::warning(
                rule::TOOL_RESTRICTION_DROPPED,
                format!(
                    "frontmatter `{key}` (a tool restriction) is dropped — Tome does not constrain tools, so dropping it silently broadens capability"
                ),
            ));
        } else if kind == EntryKind::Agent {
            diagnostics.push(Diagnostic::warning(
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
fn collect_supporting(
    root: &UntrustedRoot,
    rel_dir: &Path,
    exclude: &str,
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
            // Skip the entry's own SKILL.md (it is rendered, not copied).
            if depth == 0 && child.name == exclude {
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
    fn command_gets_legacy_positional_rewrite_agent_is_lossy() {
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
        // model + tools are dropped as agent-lossy warnings.
        let lossy = agent
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == rule::AGENT_LOSSY)
            .count();
        assert_eq!(lossy, 2, "model + tools should both warn");
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
}
