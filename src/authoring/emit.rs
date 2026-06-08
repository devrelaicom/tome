//! IR → on-disk Tome format. Writes `tome-catalog.toml` / `tome-plugin.toml` /
//! `SKILL.md` (+ command/agent markdown, `.mcp.json`, supporting files) via
//! `util::atomic_dir` staging and the `util::symlink_safe` write guard.
//!
//! Output is **deterministic** (FR-027): manifest field order follows the
//! struct declaration, `plugins[]`/MCP-server entries are sorted by name,
//! frontmatter fields emit in a fixed order, and `env` maps are `BTreeMap`s.
//! Re-emitting an unchanged IR is byte-identical, which is what the snapshot
//! pins assert.
//!
//! ## Landing strategy
//!
//! - **Fresh target** (does not exist): stage the entire tree in a sibling
//!   `.tome.tmp.*` dir and `rename` it into place — all-or-nothing atomicity
//!   (`util::atomic_dir::land_directory`; FR-014a). A failure mid-populate
//!   leaves nothing on disk.
//! - **Existing target + `--force`**: write file-by-file, overwriting only the
//!   files the artifact contributes (never a directory wipe — the contract).
//!   Each write is atomic-replace + symlink-safe; non-colliding existing files
//!   are preserved.
//! - **Existing target, no `--force`**: any colliding file → `OutputExists`
//!   (81), naming the first collision.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::authoring::ir::{Artifact, CatalogIr, EntryIr, McpServerIr, McpTransport, PluginIr};
use crate::catalog::manifest::{CatalogManifest, Owner, PluginDeclaration};
use crate::error::TomeError;
use crate::plugin::identity::EntryKind;
use crate::plugin::manifest::TomePluginManifest;

/// Emit options. `--dry-run` computes the plan and returns it without touching
/// the filesystem; `--force` enables overwriting colliding files in an
/// existing target.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmitOptions {
    pub force: bool,
    pub dry_run: bool,
}

/// Result of an emit: the landed root and the paths written (or, under
/// `--dry-run`, the paths that *would* be written), relative to `root`.
#[derive(Debug, Clone)]
pub struct EmitOutcome {
    pub root: PathBuf,
    pub written: Vec<PathBuf>,
}

/// One planned filesystem node, relative to the artifact root.
#[derive(Debug, Clone)]
struct PlannedFile {
    rel: PathBuf,
    content: PlannedContent,
}

#[derive(Debug, Clone)]
enum PlannedContent {
    /// A generated text file (manifest, markdown, JSON).
    Text(String),
    /// A supporting file copied verbatim from this absolute source path.
    Copy(PathBuf),
}

/// Emit `artifact` into `target_dir` (which becomes the artifact root —
/// e.g. `<output>/<name>/`). Returns the set of files written.
pub fn emit(
    artifact: &Artifact,
    target_dir: &Path,
    opts: EmitOptions,
) -> Result<EmitOutcome, TomeError> {
    let files = plan(artifact)?;

    // Containment at the sink: every planned path MUST be a pure-relative path
    // of `Normal` components, so `staged.join(rel)` / `target_dir.join(rel)`
    // can never escape the artifact root. The IR carries source-derived names
    // (a vendored plugin's own `name`, a marketplace `plugins[].name`) that the
    // importers should validate, but the emitter does not trust the IR to be
    // in-bounds — an absolute or `..`-bearing `rel` would otherwise redirect a
    // write outside `target_dir` (SEC-1). This is the write-side analogue of
    // `UntrustedRoot::resolve`'s read-side containment.
    for f in &files {
        ensure_in_bounds(&f.rel)?;
    }

    if opts.dry_run {
        return Ok(EmitOutcome {
            root: target_dir.to_path_buf(),
            written: files.iter().map(|f| f.rel.clone()).collect(),
        });
    }

    if target_dir.exists() {
        land_merge(target_dir, &files, opts.force)
    } else {
        land_fresh(target_dir, &files)
    }
}

/// Refuse a planned relative path that is not composed solely of `Normal`
/// components (any `..`, absolute, root, or prefix component would let the
/// subsequent `join` escape the artifact root). Fail-closed (`Io`/`InvalidInput`).
fn ensure_in_bounds(rel: &Path) -> Result<(), TomeError> {
    for comp in rel.components() {
        if !matches!(comp, std::path::Component::Normal(_)) {
            return Err(TomeError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "refusing to emit a path that escapes the artifact root: {}",
                    rel.display()
                ),
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Planning — Artifact → deterministic file list.
// ---------------------------------------------------------------------------

fn plan(artifact: &Artifact) -> Result<Vec<PlannedFile>, TomeError> {
    let mut files = Vec::new();
    match artifact {
        Artifact::Catalog(cat) => plan_catalog(cat, &mut files)?,
        Artifact::Plugin(plugin) => plan_plugin(plugin, Path::new(""), &mut files)?,
        Artifact::Skill(entry) => plan_bare_entry(entry, &mut files)?,
    }
    Ok(files)
}

fn plan_catalog(cat: &CatalogIr, files: &mut Vec<PlannedFile>) -> Result<(), TomeError> {
    let mut plugin_decls: Vec<PluginDeclaration> = cat
        .plugins
        .iter()
        .map(|p| PluginDeclaration {
            name: p.name.clone(),
            // Relative-path source within the catalog repo — the vendored
            // plugin subdirectory (data-model §3: relative-path-only).
            source: p.name.clone(),
        })
        .collect();
    plugin_decls.sort_by(|a, b| a.name.cmp(&b.name));

    let manifest = CatalogManifest {
        name: cat.name.clone(),
        description: cat.description.clone(),
        version: cat.version.clone(),
        owner: Owner {
            name: cat.owner.name.clone(),
            email: cat.owner.email.clone(),
        },
        plugins: plugin_decls,
    };
    let toml = toml::to_string(&manifest)
        .map_err(|e| TomeError::Internal(anyhow::anyhow!("serialise tome-catalog.toml: {e}")))?;
    files.push(PlannedFile {
        rel: PathBuf::from("tome-catalog.toml"),
        content: PlannedContent::Text(toml),
    });

    for plugin in &cat.plugins {
        plan_plugin(plugin, Path::new(&plugin.name), files)?;
    }
    Ok(())
}

fn plan_plugin(
    plugin: &PluginIr,
    prefix: &Path,
    files: &mut Vec<PlannedFile>,
) -> Result<(), TomeError> {
    let manifest = TomePluginManifest {
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        description: plugin.description.clone(),
        license: plugin.license.clone(),
        author: plugin.author.clone(),
    };
    let toml = toml::to_string(&manifest)
        .map_err(|e| TomeError::Internal(anyhow::anyhow!("serialise tome-plugin.toml: {e}")))?;
    files.push(PlannedFile {
        rel: prefix.join("tome-plugin.toml"),
        content: PlannedContent::Text(toml),
    });

    for entry in &plugin.entries {
        plan_entry(entry, prefix, files)?;
    }

    if !plugin.mcp_servers.is_empty() {
        files.push(PlannedFile {
            rel: prefix.join(".mcp.json"),
            content: PlannedContent::Text(mcp_json(&plugin.mcp_servers)),
        });
    }
    Ok(())
}

/// An entry inside a plugin: `skills/<n>/SKILL.md`, `commands/<n>.md`, or
/// `agents/<n>.md`.
fn plan_entry(
    entry: &EntryIr,
    prefix: &Path,
    files: &mut Vec<PlannedFile>,
) -> Result<(), TomeError> {
    let (dir, entry_root): (PathBuf, PathBuf) = match entry.kind {
        EntryKind::Skill => {
            let root = prefix.join("skills").join(&entry.name);
            (root.join("SKILL.md"), root)
        }
        EntryKind::Command => (
            prefix.join("commands").join(format!("{}.md", entry.name)),
            prefix.join("commands"),
        ),
        EntryKind::Agent => (
            prefix.join("agents").join(format!("{}.md", entry.name)),
            prefix.join("agents"),
        ),
    };
    files.push(PlannedFile {
        rel: dir,
        content: PlannedContent::Text(entry_markdown(entry)),
    });

    // Supporting files land under the entry's directory (skills only carry a
    // directory; commands/agents are single files, so their supporting files
    // are placed next to the entry's markdown).
    let support_root = match entry.kind {
        EntryKind::Skill => entry_root,
        EntryKind::Command | EntryKind::Agent => entry_root,
    };
    for sf in &entry.supporting_files {
        files.push(PlannedFile {
            rel: support_root.join(&sf.relative),
            content: PlannedContent::Copy(sf.source.clone()),
        });
    }
    Ok(())
}

/// A bare (naked) skill: `SKILL.md` at the artifact root.
fn plan_bare_entry(entry: &EntryIr, files: &mut Vec<PlannedFile>) -> Result<(), TomeError> {
    files.push(PlannedFile {
        rel: PathBuf::from("SKILL.md"),
        content: PlannedContent::Text(entry_markdown(entry)),
    });
    for sf in &entry.supporting_files {
        files.push(PlannedFile {
            rel: sf.relative.clone(),
            content: PlannedContent::Copy(sf.source.clone()),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Serialisation — deterministic frontmatter + .mcp.json.
// ---------------------------------------------------------------------------

/// The frontmatter fields Tome emits, in a fixed order, with `skip` for absent
/// optionals. Key names match the parser ([`crate::plugin::frontmatter`]):
/// kebab-case throughout except the explicit `when_to_use` snake rename.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct FrontmatterEmit<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(rename = "when_to_use", skip_serializing_if = "Option::is_none")]
    when_to_use: Option<&'a str>,
    // Owned (not a slice ref) so `Vec::is_empty` is a clean skip predicate; the
    // clone is a handful of short strings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    arguments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    argument_hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disable_model_invocation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_invocable: Option<bool>,
}

/// Render an entry as `---\n<frontmatter>---\n<body>`. The frontmatter `name`
/// and `description` are the entry's resolved values (so `name == dir` holds);
/// the remaining fields come from the mapped frontmatter.
fn entry_markdown(entry: &EntryIr) -> String {
    let fm = &entry.frontmatter;
    let emit = FrontmatterEmit {
        name: &entry.name,
        description: entry.description.as_deref(),
        when_to_use: fm.when_to_use.as_deref(),
        arguments: fm.arguments.clone(),
        argument_hint: fm.argument_hint.as_deref(),
        disable_model_invocation: fm.disable_model_invocation,
        user_invocable: fm.user_invocable,
    };
    // serde_yaml is deterministic in struct field order. It does not prepend a
    // `---` document marker; we add the SKILL.md delimiters ourselves.
    let yaml = serde_yaml::to_string(&emit).unwrap_or_default();
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let mut out = String::with_capacity(yaml.len() + entry.body.len() + 16);
    out.push_str("---\n");
    out.push_str(yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&entry.body);
    out
}

/// Render a `.mcp.json` from the plugin's MCP servers. Servers sorted by name;
/// per-server keys in a fixed order; `env` is already a `BTreeMap` (FR-027).
fn mcp_json(servers: &[McpServerIr]) -> String {
    use serde_json::{Map, Value, json};

    let mut by_name: BTreeMap<&str, &McpServerIr> = BTreeMap::new();
    for s in servers {
        by_name.insert(s.name.as_str(), s);
    }

    let mut servers_obj = Map::new();
    for (name, srv) in by_name {
        let entry = match &srv.transport {
            McpTransport::Stdio { command, args, env } => {
                let mut m = Map::new();
                m.insert("command".into(), json!(command));
                if !args.is_empty() {
                    m.insert("args".into(), json!(args));
                }
                if !env.is_empty() {
                    let env_obj: Map<String, Value> =
                        env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                    m.insert("env".into(), Value::Object(env_obj));
                }
                Value::Object(m)
            }
            McpTransport::Http { url } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("http"));
                m.insert("url".into(), json!(url));
                Value::Object(m)
            }
        };
        servers_obj.insert(name.to_owned(), entry);
    }

    let root = json!({ "mcpServers": Value::Object(servers_obj) });
    // Pretty-printed for a human-readable, diff-friendly file; deterministic
    // because the maps above were built in sorted key order.
    let mut s = serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_owned());
    s.push('\n');
    s
}

// ---------------------------------------------------------------------------
// Landing.
// ---------------------------------------------------------------------------

/// Land a fresh artifact tree atomically: stage in a sibling `.tome.tmp.*`,
/// populate, then rename into `target_dir`.
fn land_fresh(target_dir: &Path, files: &[PlannedFile]) -> Result<EmitOutcome, TomeError> {
    // `land_directory` refuses a symlinked `target_dir` (and its intermediate
    // components) before staging; the staging dir itself is freshly created
    // and private, so direct writes into it are safe — no per-file symlink
    // walk needed (that is the existing-target merge path's concern).
    crate::util::land_directory(target_dir, 0o755, |staged| {
        for f in files {
            let dest = staged.join(&f.rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match &f.content {
                PlannedContent::Text(text) => std::fs::write(&dest, text)?,
                PlannedContent::Copy(src) => {
                    std::fs::copy(src, &dest)?;
                }
            }
        }
        Ok(())
    })?;
    Ok(EmitOutcome {
        root: target_dir.to_path_buf(),
        written: files.iter().map(|f| f.rel.clone()).collect(),
    })
}

/// Merge into an existing target: refuse on collision unless `force`, then
/// write each file atomic-replace + symlink-safe, preserving non-colliding
/// existing files (never a directory wipe).
fn land_merge(
    target_dir: &Path,
    files: &[PlannedFile],
    force: bool,
) -> Result<EmitOutcome, TomeError> {
    if !force {
        for f in files {
            let dest = target_dir.join(&f.rel);
            if dest.exists() {
                return Err(TomeError::OutputExists { path: dest });
            }
        }
    }
    for f in files {
        write_planned(target_dir, f)?;
    }
    Ok(EmitOutcome {
        root: target_dir.to_path_buf(),
        written: files.iter().map(|f| f.rel.clone()).collect(),
    })
}

/// Write one planned file under `root` at its relative path. Creates parent
/// directories, refuses symlinked components, and writes via an atomic
/// temp-file replace within the destination directory.
fn write_planned(root: &Path, f: &PlannedFile) -> Result<(), TomeError> {
    let dest = root.join(&f.rel);
    let parent = dest.parent().ok_or_else(|| {
        TomeError::Internal(anyhow::anyhow!("emit: {} has no parent", dest.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    crate::util::refuse_symlinked_component(&dest).map_err(TomeError::Io)?;

    match &f.content {
        PlannedContent::Text(text) => atomic_write(&dest, text.as_bytes()),
        PlannedContent::Copy(src) => {
            // Supporting files were validated safe at import; copy verbatim.
            let bytes = std::fs::read(src)?;
            atomic_write(&dest, &bytes)
        }
    }
}

/// Atomic file write: write to a temp file in the destination's directory,
/// fsync, then rename over the target (POSIX-atomic, same FS).
fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<(), TomeError> {
    use std::io::Write;
    let dir = dest.parent().ok_or_else(|| {
        TomeError::Internal(anyhow::anyhow!(
            "atomic_write: {} has no parent",
            dest.display()
        ))
    })?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tome.tmp.")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dest).map_err(|e| TomeError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // `EntryIr`, `PluginIr`, `TomePluginManifest` already arrive via `super::*`
    // (the parent's `use` imports); only the names the parent doesn't import
    // are listed explicitly to avoid a redundant-import warning.
    use crate::authoring::ir::{MappedFrontmatter, Provenance};
    use crate::plugin::frontmatter::parse_skill_frontmatter_str;
    use crate::plugin::manifest::TomeAuthor;

    fn skill_entry(name: &str, desc: &str, body: &str) -> EntryIr {
        EntryIr {
            kind: EntryKind::Skill,
            name: name.to_owned(),
            description: Some(desc.to_owned()),
            frontmatter: MappedFrontmatter::default(),
            body: body.to_owned(),
            supporting_files: Vec::new(),
            source_path: PathBuf::from("src"),
            diagnostics: Vec::new(),
        }
    }

    fn plugin_ir() -> PluginIr {
        PluginIr {
            name: "toolkit".into(),
            version: "0.1.0".into(),
            description: Some("a toolkit".into()),
            author: Some(TomeAuthor {
                name: Some("Jo".into()),
                email: Some("jo@example.com".into()),
            }),
            license: Some("MIT".into()),
            entries: vec![skill_entry(
                "review",
                "Reviews code",
                "# Review\n\nDo it.\n",
            )],
            mcp_servers: Vec::new(),
            provenance: Provenance::local("test", PathBuf::from("src")),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn plugin_round_trips_through_files() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("toolkit");
        let outcome = emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions::default(),
        )
        .expect("emit");

        // Manifest landed + re-parses to an equal manifest.
        let m = TomePluginManifest::read(&target).expect("read manifest");
        assert_eq!(m.name, "toolkit");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.license.as_deref(), Some("MIT"));
        assert_eq!(m.author.unwrap().email.as_deref(), Some("jo@example.com"));

        // Skill landed at skills/review/SKILL.md and re-parses.
        let skill_path = target.join("skills/review/SKILL.md");
        assert!(skill_path.is_file());
        let parsed = parse_skill_frontmatter_str(
            &skill_path,
            &std::fs::read_to_string(&skill_path).unwrap(),
        )
        .expect("parse skill");
        assert_eq!(parsed.frontmatter.name.as_deref(), Some("review"));
        assert_eq!(
            parsed.frontmatter.description.as_deref(),
            Some("Reviews code")
        );
        assert_eq!(parsed.body, "# Review\n\nDo it.\n");

        assert!(outcome.written.iter().any(|p| p.ends_with("SKILL.md")));
    }

    #[test]
    fn emit_is_deterministic() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        emit(
            &Artifact::Plugin(plugin_ir()),
            &a.path().join("p"),
            EmitOptions::default(),
        )
        .unwrap();
        emit(
            &Artifact::Plugin(plugin_ir()),
            &b.path().join("p"),
            EmitOptions::default(),
        )
        .unwrap();
        let ma = std::fs::read_to_string(a.path().join("p/tome-plugin.toml")).unwrap();
        let mb = std::fs::read_to_string(b.path().join("p/tome-plugin.toml")).unwrap();
        assert_eq!(ma, mb, "manifest emission must be byte-identical");
        let sa = std::fs::read_to_string(a.path().join("p/skills/review/SKILL.md")).unwrap();
        let sb = std::fs::read_to_string(b.path().join("p/skills/review/SKILL.md")).unwrap();
        assert_eq!(sa, sb, "skill emission must be byte-identical");
    }

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("toolkit");
        let outcome = emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions {
                force: false,
                dry_run: true,
            },
        )
        .unwrap();
        assert!(!target.exists(), "dry-run must not create the target");
        assert!(
            !outcome.written.is_empty(),
            "dry-run still reports the plan"
        );
    }

    #[test]
    fn existing_target_without_force_errors_output_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("toolkit");
        emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions::default(),
        )
        .unwrap();
        // Re-emit into the now-existing dir without force → OutputExists.
        let err = emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, TomeError::OutputExists { .. }));
    }

    #[test]
    fn force_overwrites_colliding_but_preserves_others() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("toolkit");
        emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions::default(),
        )
        .unwrap();
        // A user file the artifact does not contribute.
        std::fs::write(target.join("NOTES.md"), b"keep me").unwrap();
        emit(
            &Artifact::Plugin(plugin_ir()),
            &target,
            EmitOptions {
                force: true,
                dry_run: false,
            },
        )
        .expect("force re-emit");
        assert_eq!(
            std::fs::read_to_string(target.join("NOTES.md")).unwrap(),
            "keep me",
            "non-colliding files must survive --force"
        );
    }
}
