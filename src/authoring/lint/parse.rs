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
use crate::catalog::manifest::{Owner, validate_source};
use crate::error::TomeError;
use crate::plugin::frontmatter::parse_skill_frontmatter_str;
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomeAuthor;
use crate::util::{ENTRY_BODY_MAX, PLUGIN_MANIFEST_MAX, TOME_CONFIG_MAX, bounded_read_to_string};

use super::rules::rule;

/// Parse the native Tome artifact rooted at `root` (auto-detecting its level
/// from the manifest/`SKILL.md` present) into the lint IR.
///
/// # Errors
/// [`TomeError::Usage`] if `root` holds no Tome artifact;
/// [`TomeError::Io`] on an unreadable/over-cap file.
pub fn parse_artifact(root: &Path) -> Result<Artifact, TomeError> {
    if root.join("tome-catalog.toml").is_file() {
        Ok(Artifact::Catalog(parse_catalog(root)?))
    } else if root.join("tome-plugin.toml").is_file() {
        Ok(Artifact::Plugin(parse_plugin(root)?))
    } else if root.join("SKILL.md").is_file() {
        Ok(Artifact::Skill(parse_entry(
            &root.join("SKILL.md"),
            EntryKind::Skill,
            dir_name(root),
        )?))
    } else {
        Err(TomeError::Usage(format!(
            "`{}` is not a Tome artifact (expected tome-catalog.toml, tome-plugin.toml, or SKILL.md)",
            root.display()
        )))
    }
}

/// Parse a catalog: `tome-catalog.toml` + its vendored relative-path plugins.
fn parse_catalog(root: &Path) -> Result<CatalogIr, TomeError> {
    let mut diagnostics = Vec::new();
    let body = bounded_read_to_string(&root.join("tome-catalog.toml"), TOME_CONFIG_MAX)?;
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
            if !plugin_dir.join("tome-plugin.toml").is_file() {
                diagnostics.push(Diagnostic::warning(
                    rule::CATALOG_PLUGIN_MISSING,
                    format!("catalog plugin source `{source}` has no tome-plugin.toml"),
                ));
                continue;
            }
            // CON-1: never-halt — a per-plugin parse error is a finding, not an
            // abort that hides later plugins.
            let plugin = match parse_plugin(&plugin_dir) {
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
        provenance: Provenance::local("tome", root.to_path_buf()),
        diagnostics,
    })
}

/// Parse a plugin: `tome-plugin.toml` + its `skills/`/`commands/`/`agents/`.
fn parse_plugin(plugin_dir: &Path) -> Result<PluginIr, TomeError> {
    let mut diagnostics = Vec::new();
    // CON-1: an over-cap / non-UTF-8 manifest read is a finding, not an abort.
    let value =
        match bounded_read_to_string(&plugin_dir.join("tome-plugin.toml"), PLUGIN_MANIFEST_MAX) {
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
    parse_skill_entries(plugin_dir, &mut entries)?;
    parse_md_entries(plugin_dir, "commands", EntryKind::Command, &mut entries)?;
    parse_md_entries(plugin_dir, "agents", EntryKind::Agent, &mut entries)?;

    Ok(PluginIr {
        name,
        version,
        description,
        author,
        license,
        entries,
        mcp_servers: Vec::new(),
        provenance: Provenance::local("tome", plugin_dir.to_path_buf()),
        diagnostics,
    })
}

fn parse_skill_entries(plugin_dir: &Path, entries: &mut Vec<EntryIr>) -> Result<(), TomeError> {
    let skills = plugin_dir.join("skills");
    if !skills.is_dir() {
        return Ok(());
    }
    for (name, path) in sorted_children(&skills)? {
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                entries.push(parse_entry(&skill_md, EntryKind::Skill, name)?);
            }
        }
    }
    Ok(())
}

fn parse_md_entries(
    plugin_dir: &Path,
    sub: &str,
    kind: EntryKind,
    entries: &mut Vec<EntryIr>,
) -> Result<(), TomeError> {
    let dir = plugin_dir.join(sub);
    if !dir.is_dir() {
        return Ok(());
    }
    for (name, path) in sorted_children(&dir)? {
        if path.is_file()
            && let Some(stem) = name.strip_suffix(".md")
        {
            entries.push(parse_entry(&path, kind, stem.to_owned())?);
        }
    }
    Ok(())
}

/// Parse one entry file. A malformed entry yields an `EntryIr` carrying an
/// error diagnostic (so the runner reports it) rather than aborting the lint.
fn parse_entry(path: &Path, kind: EntryKind, dir_or_stem: String) -> Result<EntryIr, TomeError> {
    // CON-1: an over-cap / non-UTF-8 entry read is a finding, not an abort.
    let content = match bounded_read_to_string(path, ENTRY_BODY_MAX) {
        Ok(c) => c,
        Err(e) => {
            return Ok(EntryIr {
                kind,
                name: dir_or_stem,
                description: None,
                frontmatter: MappedFrontmatter::default(),
                body: String::new(),
                supporting_files: Vec::new(),
                source_path: path.to_path_buf(),
                diagnostics: vec![Diagnostic::error(
                    rule::ENTRY_INVALID,
                    format!("could not read entry: {e}"),
                )],
            });
        }
    };
    match parse_skill_frontmatter_str(path, &content) {
        Ok(parsed) => {
            let (name, _) = parsed.resolved_name(&dir_or_stem);
            Ok(EntryIr {
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
                source_path: path.to_path_buf(),
                diagnostics: Vec::new(),
            })
        }
        Err(e) => Ok(EntryIr {
            kind,
            name: dir_or_stem,
            description: None,
            frontmatter: MappedFrontmatter::default(),
            body: String::new(),
            supporting_files: Vec::new(),
            source_path: path.to_path_buf(),
            diagnostics: vec![Diagnostic::error(
                rule::ENTRY_INVALID,
                format!("entry frontmatter could not be parsed: {e}"),
            )],
        }),
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

/// Children of `dir` as `(name, path)`, sorted by name for deterministic
/// findings order (FR-027).
fn sorted_children(dir: &Path) -> Result<Vec<(String, std::path::PathBuf)>, TomeError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(TomeError::Io)? {
        let entry = entry.map_err(TomeError::Io)?;
        if let Some(name) = entry.file_name().to_str() {
            out.push((name.to_owned(), entry.path()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
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
