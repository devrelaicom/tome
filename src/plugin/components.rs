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

use std::path::Path;

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
