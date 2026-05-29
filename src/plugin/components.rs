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
//!
//! Phase 6 / US1 adds [`list_agent_files`], the agent analogue: a flat walk
//! of `agents/*.md` indexed as `kind='agent'` rows. The on-disk layout is
//! identical to commands (flat `.md` files), so both share the
//! [`EntryFile`] shape and the [`list_entry_files`] walk — see
//! `contracts/entry-schema-p6.md` § "Indexing pipeline".

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

/// One discovered flat-file entry (command or agent): the on-disk file
/// plus the sanitised `name` Tome will record (the filename stem). Phase 5
/// / US1.a command consumer is `plugin::lifecycle::collect_command_entries`;
/// Phase 6 / US1 adds the agent consumer `collect_agent_entries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFile {
    /// Absolute path to the `.md` file.
    pub path: PathBuf,
    /// Filename stem with the `.md` extension stripped — used as the
    /// fallback `name` if the file's frontmatter does not declare one.
    pub name: String,
}

/// Phase 5 name for a discovered command entry. Retained as an alias so the
/// command lifecycle path reads unchanged; Phase 6 generalised the struct to
/// [`EntryFile`] to share the flat `.md` walk with agents.
pub type CommandFile = EntryFile;

/// Enumerate `<plugin_dir>/commands/*.md` non-recursively. Returned in
/// case-insensitive ascending order of the filename stem so the on-disk
/// snapshot stays deterministic across platforms.
///
/// Per Phase 5 the walk is FLAT (commands live directly under
/// `commands/`; sub-directories are ignored). Files whose names start
/// with `.` are skipped (hidden / editor-temp files). A missing
/// `commands/` directory yields an empty `Vec`, never an error.
pub fn list_command_files(plugin_dir: &Path) -> Vec<CommandFile> {
    list_entry_files(plugin_dir, "commands")
}

/// Enumerate `<plugin_dir>/agents/*.md` non-recursively (Phase 6 / US1).
/// Mirrors [`list_command_files`] exactly — the agent on-disk layout is the
/// same flat `.md` convention — so the same FLAT walk, dotfile skip,
/// case-insensitive `.md` filter, and deterministic stem sort apply. A
/// missing `agents/` directory yields an empty `Vec`, never an error
/// (`entry-schema-p6.md` § "Indexing pipeline").
pub fn list_agent_files(plugin_dir: &Path) -> Vec<EntryFile> {
    list_entry_files(plugin_dir, "agents")
}

/// Shared flat `<plugin_dir>/<subdir>/*.md` walk backing both
/// [`list_command_files`] and [`list_agent_files`]. Non-recursive;
/// dotfiles skipped; `.md` matched case-insensitively; results sorted by
/// filename stem for cross-platform determinism. A missing/unreadable
/// `<subdir>` yields an empty `Vec`, never an error.
fn list_entry_files(plugin_dir: &Path, subdir: &str) -> Vec<EntryFile> {
    let dir = plugin_dir.join(subdir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<EntryFile> = Vec::new();
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
        out.push(EntryFile {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Missing `agents/` directory yields an empty Vec, never an error.
    #[test]
    fn list_agent_files_missing_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(list_agent_files(tmp.path()).is_empty());
    }

    /// Flat `.md` walk: deterministic stem sort, dotfiles skipped,
    /// non-`.md` ignored, sub-directories not recursed.
    #[test]
    fn list_agent_files_walks_flat_md_sorted_skipping_dotfiles() {
        let tmp = TempDir::new().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("zeta.md"), "z").unwrap();
        fs::write(agents.join("alpha.md"), "a").unwrap();
        fs::write(agents.join(".hidden.md"), "h").unwrap();
        fs::write(agents.join("notes.txt"), "t").unwrap();
        fs::create_dir_all(agents.join("nested")).unwrap();
        fs::write(agents.join("nested").join("deep.md"), "d").unwrap();

        let found = list_agent_files(tmp.path());
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"], "sorted by stem, flat only");
        assert!(found.iter().all(|e| e.path.is_file()));
    }
}
