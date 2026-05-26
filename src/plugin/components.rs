//! Walk a plugin directory and count its components.
//!
//! Conventions follow the Claude Code plugin layout: `skills/<name>/SKILL.md`,
//! `agents/<name>.md`, `commands/<name>.md`, `hooks/<name>.md` (or `hooks.json`),
//! and a top-level `.mcp.json` whose `mcpServers` map enumerates servers.
//!
//! Walks are I/O-tolerant: a missing or unreadable component directory is
//! reported as a zero count rather than an error — the plugin is allowed to
//! omit any subset of components.
//!
//! Spec: data-model.md §2 (`ComponentCounts`), tasks.md T035.
//!
//! Phase 5 / US1.a additionally exposes [`list_command_files`] which
//! enumerates `commands/*.md` for the lifecycle pipeline — see
//! `contracts/entry-schema-p5.md` for the kind-discriminated entry model.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ComponentCounts {
    pub skills: u32,
    pub agents: u32,
    pub commands: u32,
    pub hooks: u32,
    pub mcp_servers: u32,
}

/// Inspect `plugin_dir` and return component counts. Never fails: directories
/// that are missing or unreadable contribute zero.
pub fn count_components(plugin_dir: &Path) -> ComponentCounts {
    ComponentCounts {
        skills: count_skill_dirs(&plugin_dir.join("skills")),
        agents: count_markdown_files(&plugin_dir.join("agents")),
        commands: count_markdown_files(&plugin_dir.join("commands")),
        hooks: count_markdown_files(&plugin_dir.join("hooks")),
        mcp_servers: count_mcp_servers(&plugin_dir.join(".mcp.json")),
    }
}

/// Count subdirectories that contain a `SKILL.md`. Bare files (e.g. a README)
/// at `skills/` root do not count.
fn count_skill_dirs(dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count: u32 = 0;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            && entry.path().join("SKILL.md").is_file()
        {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Count direct-child files ending in `.md`. Non-existent directories yield 0.
fn count_markdown_files(dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count: u32 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        {
            count = count.saturating_add(1);
        }
    }
    count
}

/// One discovered command entry: the on-disk file plus the sanitised
/// `name` Tome will record (the filename stem). Phase 5 / US1.a consumer
/// is `plugin::lifecycle::collect_pending_commands`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFile {
    /// Absolute path to the `.md` file.
    pub path: PathBuf,
    /// Filename stem with the `.md` extension stripped — used as the
    /// fallback `name` if the file's frontmatter does not declare one.
    pub name: String,
}

/// Enumerate `<plugin_dir>/commands/*.md` non-recursively. Returned in
/// case-insensitive ascending order of the filename stem so the on-disk
/// snapshot stays deterministic across platforms.
///
/// Per Phase 5 the walk is FLAT (commands live directly under
/// `commands/`; sub-directories are ignored). Files whose names start
/// with `.` are skipped (hidden / editor-temp files). A missing
/// `commands/` directory yields an empty `Vec`, never an error.
pub fn list_command_files(plugin_dir: &Path) -> Vec<CommandFile> {
    let dir = plugin_dir.join("commands");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<CommandFile> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }
        let extension_ok = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !extension_ok {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        out.push(CommandFile {
            path: path.clone(),
            name: stem.to_owned(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Parse `.mcp.json` and count entries under `mcpServers`. A missing,
/// unreadable, or malformed file yields 0 — this is a counts-only helper,
/// not a strict validator.
fn count_mcp_servers(path: &Path) -> u32 {
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return 0;
    };
    value
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .map(|obj| u32::try_from(obj.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}
