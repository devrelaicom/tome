//! Canonical + translated agent types (data-model §4).
//!
//! **Skeleton.** F3 lands the type definitions the `HarnessModule`
//! `translate_agent` method signature needs; the parsing of `agents/*.md`
//! into a [`CanonicalAgent`], the per-harness translation rules, the model
//! alias table, and the clash-set machinery are all US1 (T034) work. The
//! types are real (not placeholders) so US1 fleshes out behaviour without
//! reshaping the public surface.

use std::path::PathBuf;

use super::AgentFormat;

/// A plugin's source agent, parsed from `<plugin>/agents/<name>.md`
/// (data-model §4). The privileged fields (`hooks`, `mcp_servers`,
/// `permission_mode`) are passed through to Claude Code by default and
/// stripped under the `strip_plugin_agent_privileges` setting (FR-050 /
/// FR-052). `serde_json::Value` keeps the privileged blobs opaque — Tome
/// neither interprets nor validates their internal shape, it only forwards
/// or drops them wholesale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAgent {
    /// Frontmatter `name`, else the filename stem.
    pub name: String,
    /// Frontmatter `description`, if present.
    pub description: Option<String>,
    /// System-prompt Markdown (the body below the frontmatter).
    pub body: String,
    /// Canonical model value (`opus`, `inherit`, …), if declared. Mapped
    /// per-harness via the same-vendor-only model alias table (FR-037).
    pub model: Option<String>,
    /// Allowed tools posture (drives read-only inference, FR-036).
    pub tools: Option<Vec<String>>,
    /// Disallowed tools posture.
    pub disallowed_tools: Option<Vec<String>>,
    /// Privileged: hook spec passed through to Claude Code (FR-050).
    pub hooks: Option<serde_json::Value>,
    /// Privileged: MCP server spec passed through to Claude Code (FR-050).
    pub mcp_servers: Option<serde_json::Value>,
    /// Privileged: permission mode passed through to Claude Code (FR-050).
    pub permission_mode: Option<String>,
}

/// The per-harness emission result for one agent (data-model §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedAgent {
    /// Target directory — the harness's `agent_dir(project)`.
    pub dir: PathBuf,
    /// Always `<plugin>__<name>.<ext>` (FR-040).
    pub filename: String,
    /// Clean `<name>`, or a clash-prefixed `<plugin>-<name>` (FR-041);
    /// OpenCode always uses `<plugin>__<name>` (FR-042).
    pub displayed_name: String,
    /// MarkdownYaml or Toml, per the harness's `agent_format()`.
    pub format: AgentFormat,
    /// The rendered file content (body in the file body, or in a
    /// triple-quoted `developer_instructions` TOML string — FR-033).
    pub rendered: String,
    /// Frontmatter fields dropped during translation, recorded for
    /// diagnostics (FR-032 / FR-034 / FR-036).
    pub dropped_fields: Vec<String>,
}
