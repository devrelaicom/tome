//! Parse a **native Tome artifact** tree into the IR for `lint`.
//!
//! Deliberately **lenient**: Tome reads `tome-plugin.toml` strictly at *load*
//! time (`plugin::manifest::read_plugin_manifest`, `deny_unknown_fields`,
//! required `version`), but `lint` parses the same files with `toml::Value` so
//! a missing/invalid field becomes a reported *finding* (a manifest-validity
//! rule fires) rather than a parse abort that hides every other issue. A single
//! `lint` run reports ALL findings — the parser surfaces what it can and leaves
//! the rest to the rules.
//!
//! This is the native-format counterpart to the `convert` importers (which read
//! foreign formats): same `SKILL.md`/dir layout, but the manifest is
//! `tome-plugin.toml`/`tome-catalog.toml` and **no** harness-ism rewrite is
//! applied (lint flags residual harness-isms; `--autofix` rewrites).

use std::path::Path;

use crate::authoring::ir::{
    Artifact, CatalogIr, Diagnostic, EntryIr, MappedFrontmatter, PluginIr, Provenance,
};
use crate::authoring::untrusted::UntrustedRoot;
use crate::catalog::manifest::{Owner, validate_source};
use crate::error::TomeError;
use crate::plugin::frontmatter::parse_skill_frontmatter_str;
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomeAuthor;
use crate::util::{HARNESS_MCP_MAX, PLUGIN_MANIFEST_MAX, TOME_CONFIG_MAX};

use super::rules::rule;

/// Parse the native Tome artifact rooted at `root` (auto-detecting its level
/// from the manifest/`SKILL.md` present) into the lint IR.
///
/// # Errors
/// [`TomeError::Usage`] if `root` holds no Tome artifact;
/// [`TomeError::Io`] on an unreadable/over-cap file.
pub fn parse_artifact(root: &Path) -> Result<Artifact, TomeError> {
    // Route every read of the (untrusted) artifact tree through `UntrustedRoot`,
    // exactly as the `convert` importers do: each path is resolved by a walk
    // from the canonical root that refuses a symlinked component BEFORE any read,
    // so a malicious native plugin (`skills/evil -> /outside`) can never disclose
    // out-of-tree content into a finding. A failure to open the root as a
    // directory is reported as "not a Tome artifact" (Usage 2), preserving the
    // prior behaviour for a missing/non-dir path.
    let Ok(ur) = UntrustedRoot::open(root) else {
        return Err(not_an_artifact(root));
    };
    if ur.is_file(Path::new("tome-catalog.toml")) {
        Ok(Artifact::Catalog(parse_catalog(&ur)?))
    } else if ur.is_file(Path::new("tome-plugin.toml")) {
        Ok(Artifact::Plugin(parse_plugin(&ur)?))
    } else if ur.is_file(Path::new("SKILL.md")) {
        Ok(Artifact::Skill(parse_entry(
            &ur,
            Path::new("SKILL.md"),
            EntryKind::Skill,
            dir_name(root),
        )))
    } else {
        Err(not_an_artifact(root))
    }
}

fn not_an_artifact(root: &Path) -> TomeError {
    TomeError::Usage(format!(
        "`{}` is not a Tome artifact (expected tome-catalog.toml, tome-plugin.toml, or SKILL.md)",
        root.display()
    ))
}

/// Parse a catalog: `tome-catalog.toml` + its vendored relative-path plugins.
fn parse_catalog(ur: &UntrustedRoot) -> Result<CatalogIr, TomeError> {
    let mut diagnostics = Vec::new();
    let body = ur.read_text(Path::new("tome-catalog.toml"), TOME_CONFIG_MAX)?;
    let value = parse_toml(&body, "tome-catalog.toml", &mut diagnostics);

    let name = str_field(value.as_ref(), "name");
    let version = str_field(value.as_ref(), "version");
    let description = opt_str_field(value.as_ref(), "description").unwrap_or_default();
    let owner = parse_owner(value.as_ref().and_then(|v| v.get("owner")));

    let mut plugins = Vec::new();
    if let Some(arr) = value
        .as_ref()
        .and_then(|v| v.get("plugins"))
        .and_then(|v| v.as_array())
    {
        for decl in arr {
            let declared = decl.get("name").and_then(|v| v.as_str());
            let Some(source) = decl.get("source").and_then(|v| v.as_str()) else {
                continue;
            };
            // SEC-1: validate the source is in-root (no `..`/absolute/URL/escape)
            // BEFORE joining and reading — the resulting `source_path` flows into
            // `--autofix` writes, so an unvalidated source could redirect a write
            // outside the artifact tree.
            let root = ur.root();
            let plugin_dir = match validate_source(root, &root.join("tome-catalog.toml"), source) {
                Ok(p) => p,
                Err(e) => {
                    diagnostics.push(Diagnostic::warning(
                        rule::CATALOG_PLUGIN_INVALID,
                        format!("catalog plugin source `{source}` is invalid: {e}"),
                    ));
                    continue;
                }
            };
            // The vendored plugin is its own untrusted subtree — open a fresh
            // guard rooted at it (validate_source already proved containment;
            // this re-asserts symlink-safety for the plugin's own walk).
            let plugin_ur = match UntrustedRoot::open(&plugin_dir) {
                Ok(p) => p,
                Err(_) => {
                    diagnostics.push(Diagnostic::warning(
                        rule::CATALOG_PLUGIN_MISSING,
                        format!("catalog plugin source `{source}` is not a readable directory"),
                    ));
                    continue;
                }
            };
            if !plugin_ur.is_file(Path::new("tome-plugin.toml")) {
                diagnostics.push(Diagnostic::warning(
                    rule::CATALOG_PLUGIN_MISSING,
                    format!("catalog plugin source `{source}` has no tome-plugin.toml"),
                ));
                continue;
            }
            // CON-1: never-halt — a per-plugin parse error is a finding, not an
            // abort that hides later plugins.
            let plugin = match parse_plugin(&plugin_ur) {
                Ok(p) => p,
                Err(e) => {
                    diagnostics.push(Diagnostic::error(
                        rule::MANIFEST_INVALID,
                        format!("could not lint plugin `{source}`: {e}"),
                    ));
                    continue;
                }
            };
            // §9 row 6: the declared name should match the plugin's own name.
            // This is emitted at PARSE time (not as a registered `Rule`) on
            // purpose: only the parser holds both the catalog declaration and
            // the parsed plugin's name, and `convert` *produces* catalogs whose
            // declaration always matches the vendored plugin's name, so the
            // check is meaningful only for a hand-authored catalog (`lint`).
            if let Some(declared) = declared
                && declared != plugin.name
            {
                diagnostics.push(Diagnostic::warning(
                    rule::CATALOG_NAME_MISMATCH,
                    format!(
                        "catalog declares plugin `{declared}` but its manifest name is `{}`",
                        plugin.name
                    ),
                ));
            }
            plugins.push(plugin);
        }
    }

    Ok(CatalogIr {
        name,
        version,
        description,
        owner,
        plugins,
        provenance: Provenance::local("tome", ur.root().to_path_buf()),
        diagnostics,
    })
}

/// Parse a plugin: `tome-plugin.toml` + its `skills/`/`commands/`/`agents/`.
fn parse_plugin(ur: &UntrustedRoot) -> Result<PluginIr, TomeError> {
    let mut diagnostics = Vec::new();
    // CON-1: an over-cap / non-UTF-8 manifest read (or a refused symlinked
    // manifest) is a finding, not an abort.
    let value = match ur.read_text(Path::new("tome-plugin.toml"), PLUGIN_MANIFEST_MAX) {
        Ok(body) => parse_toml(&body, "tome-plugin.toml", &mut diagnostics),
        Err(e) => {
            diagnostics.push(Diagnostic::error(
                rule::MANIFEST_INVALID,
                format!("could not read tome-plugin.toml: {e}"),
            ));
            None
        }
    };

    let name = str_field(value.as_ref(), "name");
    let version = str_field(value.as_ref(), "version");
    let description = opt_str_field(value.as_ref(), "description");
    let license = opt_str_field(value.as_ref(), "license");
    let author = parse_author(value.as_ref().and_then(|v| v.get("author")));

    let mut entries = Vec::new();
    parse_skill_entries(ur, &mut entries, &mut diagnostics);
    parse_md_entries(
        ur,
        "commands",
        EntryKind::Command,
        &mut entries,
        &mut diagnostics,
    );
    parse_md_entries(
        ur,
        "agents",
        EntryKind::Agent,
        &mut entries,
        &mut diagnostics,
    );

    // hooks/hooks.json content for the HooksSpec rule — same UntrustedRoot
    // guard as every other read (P8: one read guard, no sibling hand-rolls).
    // An unreadable file is itself a finding (never-halt).
    let hooks_json = if ur.is_file(Path::new("hooks/hooks.json")) {
        match ur.read_text(Path::new("hooks/hooks.json"), HARNESS_MCP_MAX) {
            Ok(s) => Some(s),
            Err(e) => {
                diagnostics.push(Diagnostic::warning(
                    rule::HOOKS_INVALID,
                    format!("could not read hooks/hooks.json: {e}"),
                ));
                None
            }
        }
    } else {
        None
    };

    Ok(PluginIr {
        name,
        version,
        description,
        author,
        license,
        entries,
        mcp_servers: Vec::new(),
        hooks_files: Vec::new(),
        hooks_json,
        provenance: Provenance::local("tome", ur.root().to_path_buf()),
        diagnostics,
    })
}

fn parse_skill_entries(
    ur: &UntrustedRoot,
    entries: &mut Vec<EntryIr>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let skills = Path::new("skills");
    if !ur.is_dir(skills) {
        return;
    }
    // `list_dir` refuses a symlinked child outright (fail-closed); degrade that
    // refusal to a finding so the rest of the plugin still lints (never-halt).
    let children = match ur.list_dir(skills) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(Diagnostic::warning(
                rule::UNSAFE_PATH,
                format!("skipped `skills/` (refused an unsafe entry): {e}"),
            ));
            return;
        }
    };
    for child in children {
        if child.is_dir {
            let skill_md = child.rel.join("SKILL.md");
            if ur.is_file(&skill_md) {
                entries.push(parse_entry(ur, &skill_md, EntryKind::Skill, child.name));
            }
        }
    }
}

fn parse_md_entries(
    ur: &UntrustedRoot,
    sub: &str,
    kind: EntryKind,
    entries: &mut Vec<EntryIr>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let dir = Path::new(sub);
    if !ur.is_dir(dir) {
        return;
    }
    let children = match ur.list_dir(dir) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(Diagnostic::warning(
                rule::UNSAFE_PATH,
                format!("skipped `{sub}/` (refused an unsafe entry): {e}"),
            ));
            return;
        }
    };
    for child in children {
        if !child.is_dir
            && let Some(stem) = child.name.strip_suffix(".md")
        {
            entries.push(parse_entry(ur, &child.rel, kind, stem.to_owned()));
        }
    }
}

/// Parse one entry file (its path relative to the artifact root). A malformed OR
/// unreadable/refused entry yields an `EntryIr` carrying an error diagnostic (so
/// the runner reports it) rather than aborting the lint — the read is guarded by
/// `UntrustedRoot::read_body`, which refuses a symlinked component before reading.
fn parse_entry(ur: &UntrustedRoot, rel: &Path, kind: EntryKind, dir_or_stem: String) -> EntryIr {
    // `source_path` is the resolved in-root absolute path (for Location +
    // autofix Fix.path); `rel` was already proven `Normal`-only by the
    // `is_file`/`list_dir` that reached here, so `root.join(rel)` is in-bounds.
    let abs = ur.root().join(rel);
    // CON-1: an over-cap / non-UTF-8 / refused read is a finding, not an abort.
    let content = match ur.read_body(rel) {
        Ok(c) => c,
        Err(e) => {
            return EntryIr {
                kind,
                name: dir_or_stem,
                description: None,
                frontmatter: MappedFrontmatter::default(),
                body: String::new(),
                supporting_files: Vec::new(),
                source_path: abs,
                diagnostics: vec![Diagnostic::error(
                    rule::ENTRY_INVALID,
                    format!("could not read entry: {e}"),
                )],
            };
        }
    };
    match parse_skill_frontmatter_str(&abs, &content) {
        Ok(parsed) => {
            let (name, _) = parsed.resolved_name(&dir_or_stem);
            EntryIr {
                kind,
                name,
                // Raw frontmatter description (no body fallback — lint flags a
                // missing description as a finding).
                description: parsed
                    .frontmatter
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                frontmatter: parsed.frontmatter,
                body: parsed.body,
                supporting_files: Vec::new(),
                source_path: abs,
                diagnostics: Vec::new(),
            }
        }
        Err(e) => EntryIr {
            kind,
            name: dir_or_stem,
            description: None,
            frontmatter: MappedFrontmatter::default(),
            body: String::new(),
            supporting_files: Vec::new(),
            source_path: abs,
            diagnostics: vec![Diagnostic::error(
                rule::ENTRY_INVALID,
                format!("entry frontmatter could not be parsed: {e}"),
            )],
        },
    }
}

// --- small helpers ----------------------------------------------------------

fn dir_name(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("skill")
        .to_owned()
}

/// Parse a manifest body leniently, pushing an error diagnostic on a syntax
/// failure and returning `None` so field extraction yields empties.
fn parse_toml(body: &str, file: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<toml::Value> {
    match toml::from_str::<toml::Value>(body) {
        Ok(v) => Some(v),
        Err(e) => {
            diagnostics.push(Diagnostic::error(
                rule::MANIFEST_INVALID,
                format!("{file} is not valid TOML: {e}"),
            ));
            None
        }
    }
}

fn str_field(value: Option<&toml::Value>, key: &str) -> String {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

fn opt_str_field(value: Option<&toml::Value>, key: &str) -> Option<String> {
    value
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

fn parse_owner(value: Option<&toml::Value>) -> Owner {
    let table = value.and_then(|v| v.as_table());
    Owner {
        name: table
            .and_then(|t| t.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
        email: table
            .and_then(|t| t.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
    }
}

fn parse_author(value: Option<&toml::Value>) -> Option<TomeAuthor> {
    let table = value?.as_table()?;
    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let email = table
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    if name.is_none() && email.is_none() {
        None
    } else {
        Some(TomeAuthor { name, email })
    }
}
