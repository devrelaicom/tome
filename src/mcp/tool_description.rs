//! Compose the runtime `search_skills` tool description from a fixed
//! scaffold + the workspace's cached short summary.
//!
//! Phase 4 / US4.b. Contract reference: FR-425.
//!
//! ## Why a runtime override?
//!
//! The `#[tool]` macro on `mcp::server::Server::search_skills` accepts
//! description text only as a string-literal or doc comment — both are
//! compile-time. The cached short summary is a workspace-level runtime
//! value, so we mutate `ToolRouter::map`'s entry for `search_skills`
//! after the router is built. `ToolRoute::attr.description` is the
//! advertised string the protocol surfaces in `list_tools`.
//!
//! ## Sync-boundary discipline (FR-425 + NFR-103)
//!
//! This module performs a single-shot synchronous read of
//! `<root>/workspaces/<name>/settings.toml` at MCP startup. It does
//! NOT invoke the summariser. Subsequent regenerations from
//! out-of-process CLI invocations write to the same file, but the
//! running MCP server keeps its in-memory description — re-starting
//! the server picks up the new one.
//!
//! ## Length-window
//!
//! When the composed description exceeds [`MAX_DESCRIPTION_LEN`], we
//! emit a `tracing::warn!` but still apply it. The agent-host budget
//! varies by client; truncating server-side would silently lose
//! information that some clients can render. The warning gives the
//! operator a clear signal in `mcp.log`.

use std::path::Path;

use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Best-effort soft cap on the composed tool description. Most agent
/// hosts cap tool descriptions around 1–2 KB; 1500 chars leaves
/// headroom for JSON-encoding overhead. Above this we warn but still
/// apply.
pub const MAX_DESCRIPTION_LEN: usize = 1500;

/// Fixed scaffold that explains the tool's purpose and when to call
/// it. Identical to the `#[tool]` doc-comment on
/// `mcp::server::Server::search_skills`. We duplicate it here so the
/// composed description always carries the scaffold whether or not a
/// cached summary exists.
pub const SCAFFOLD: &str = "Find the most relevant skills in the local Tome index for a natural-language task description. Call this proactively before approaching any non-trivial task to discover existing skills you can rely on. Returns a ranked list of candidates with on-disk paths; follow up with `get_skill` to load the skill body and resource files.";

/// Compose the runtime description from the scaffold plus the
/// workspace's cached short summary (if any).
///
/// Returns the scaffold alone when:
/// * the workspace's `settings.toml` is absent (fresh workspace),
/// * the `[summaries].short` field is absent or empty,
/// * the file is unparsable (best-effort fallback, not an error
///   — a malformed cache shouldn't refuse the MCP server),
///
/// otherwise returns `"{scaffold}\n\n{cached_short}"`.
pub fn compose(name: &WorkspaceName, paths: &Paths) -> String {
    let settings_path = paths.workspace_settings_file(name);
    match read_cached_short(&settings_path) {
        Some(short) if !short.trim().is_empty() => format!("{SCAFFOLD}\n\n{short}"),
        _ => SCAFFOLD.to_owned(),
    }
}

/// Read `[summaries].short` from `settings.toml`. Returns `None` for
/// any of the documented fallback paths (file absent / unparsable /
/// section missing / field missing / field not a string).
fn read_cached_short(settings_path: &Path) -> Option<String> {
    let body =
        crate::util::bounded_read_to_string(settings_path, crate::util::TOME_CONFIG_MAX).ok()?;
    let doc: toml::Value = toml::from_str(&body).ok()?;
    let short = doc.get("summaries")?.get("short")?.as_str()?;
    Some(short.to_owned())
}

/// Emit a `tracing::warn!` if `description` exceeds
/// [`MAX_DESCRIPTION_LEN`]. Best-effort: callers still apply the
/// composed string to the tool router.
pub fn warn_if_too_long(workspace: &WorkspaceName, description: &str) {
    let len = description.chars().count();
    if len > MAX_DESCRIPTION_LEN {
        tracing::warn!(
            target: "tome::mcp::tool_description",
            workspace = workspace.as_str(),
            description_chars = len,
            limit = MAX_DESCRIPTION_LEN,
            "search_skills tool description exceeds soft length cap; some agent hosts may truncate",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_falls_back_to_scaffold_when_settings_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("demo").unwrap();
        let out = compose(&name, &paths);
        assert_eq!(out, SCAFFOLD);
    }

    #[test]
    fn compose_includes_cached_short_when_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("demo").unwrap();
        std::fs::create_dir_all(paths.workspace_dir(&name)).unwrap();
        std::fs::write(
            paths.workspace_settings_file(&name),
            "name = \"demo\"\n\n[summaries]\nshort = \"covers ai-llm topics\"\nlong = \"long text\"\n",
        )
        .unwrap();
        let out = compose(&name, &paths);
        assert!(out.starts_with(SCAFFOLD));
        assert!(out.contains("covers ai-llm topics"));
    }

    #[test]
    fn compose_falls_back_when_short_is_empty_string() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("demo").unwrap();
        std::fs::create_dir_all(paths.workspace_dir(&name)).unwrap();
        std::fs::write(
            paths.workspace_settings_file(&name),
            "name = \"demo\"\n[summaries]\nshort = \"\"\nlong = \"\"\n",
        )
        .unwrap();
        let out = compose(&name, &paths);
        assert_eq!(out, SCAFFOLD);
    }

    #[test]
    fn compose_falls_back_when_settings_is_malformed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        let name = WorkspaceName::parse("demo").unwrap();
        std::fs::create_dir_all(paths.workspace_dir(&name)).unwrap();
        std::fs::write(paths.workspace_settings_file(&name), "::: not toml at all").unwrap();
        let out = compose(&name, &paths);
        assert_eq!(out, SCAFFOLD);
    }
}
