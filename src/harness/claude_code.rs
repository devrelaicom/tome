//! `claude-code` — Anthropic's Claude Code CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.claude/`.
//! - Rules-file target: `CLAUDE.md` > `.claude/CLAUDE.md` (first existing
//!   wins; creates `CLAUDE.md` when none exist). Phase 6 correction
//!   (FR-020/021/022): Claude Code does not natively read `AGENTS.md`, so
//!   `AGENTS.md` is no longer a candidate — its rules-include block (and
//!   guardrails region) land in `CLAUDE.md`.
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `<project>/.claude/settings.json` (per-project).
//! - Parent key: `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, BlockBodyStyle, GuardrailsPlacement, GuardrailsTarget, HarnessModule,
    HooksStrategy, McpConfigFormat, RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for Claude Code.
pub struct ClaudeCode;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CLAUDE_CODE: ClaudeCode = ClaudeCode;

impl HarnessModule for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn description(&self) -> &'static str {
        "Anthropic's Claude Code CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".claude").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "claude-code" but the per-user dir is `~/.claude/`
        // (no `-code` suffix). Override the default to keep the path
        // reported by `tome harness info`'s `detected_path` in lockstep
        // with what `detect` actually probes.
        home.join(".claude")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // Precedence: CLAUDE.md > .claude/CLAUDE.md. First existing candidate
        // wins; fall back to CLAUDE.md if none exist (the sync algorithm will
        // create it on first write).
        //
        // Phase 6 correction (FR-020/021/022): AGENTS.md MUST NOT appear in
        // this candidate set — Claude Code does not natively read it, so a
        // rules-include block written there would be invisible. Codex / Gemini
        // / OpenCode keep sharing AGENTS.md; both that block and this CLAUDE.md
        // block resolve the same `.tome/RULES.md`.
        for candidate in ["CLAUDE.md", ".claude/CLAUDE.md"] {
            let p = project_root.join(candidate);
            if p.exists() {
                return p;
            }
        }
        project_root.join("CLAUDE.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".claude/settings.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }

    // -- Real hooks (FR-001, FR-002) ----------------------------------------

    /// Claude Code is the only harness with native JSON hook support — its
    /// plugins' `hooks/hooks.json` merges into the machine-local settings.
    fn hooks_strategy(&self) -> HooksStrategy {
        HooksStrategy::RealJson
    }

    /// The local, gitignored settings file (FR-002). Rewritten hooks carry
    /// machine-specific absolute paths, so they land in `settings.local.json`,
    /// never the committed `settings.json`.
    fn hook_settings_path(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".claude/settings.local.json"))
    }

    // -- Guardrails fallback (FR-011, FR-012, FR-013) -----------------------

    /// Claude Code's guardrails region lands in `CLAUDE.md` (the corrected
    /// rules-file target, never `AGENTS.md`). It is the ONLY harness that
    /// suppresses a plugin's region when that plugin ships real JSON hooks
    /// (FR-013): the merged hooks supersede the prose fallback.
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::InFileRegion {
                file: self.rules_file_target(project_root),
            },
            suppress_if_hooks_present: true,
        }
    }

    // -- Native agents (FR-030–FR-032, FR-050) ------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".claude/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    /// Claude Code is the canonical vendor: it keeps the full canonical
    /// frontmatter vocabulary, including the privileged `hooks` /
    /// `mcpServers` / `permissionMode` blobs which are a Claude Code-only
    /// capability advantage (FR-050; the `strip_plugin_agent_privileges`
    /// suppression is US5, NOT applied here). `model` ports through the
    /// same-vendor alias table; the body lands in the file body.
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        let name = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
        frontmatter.push(("name".to_owned(), yaml_str(&name)));

        if let Some(desc) = &canonical.description {
            frontmatter.push(("description".to_owned(), yaml_str(desc)));
        }

        // `model` is same-vendor; Claude Code passes its aliases verbatim.
        if let Some(src) = canonical.model.as_deref() {
            match agents::map_model("claude-code", src) {
                Some(mapped) => frontmatter.push(("model".to_owned(), yaml_str(&mapped))),
                None => dropped.push("model".to_owned()),
            }
        }

        if let Some(tools) = &canonical.tools {
            frontmatter.push(("tools".to_owned(), yaml_str_seq(tools)));
        }
        if let Some(disallowed) = &canonical.disallowed_tools {
            frontmatter.push(("disallowedTools".to_owned(), yaml_str_seq(disallowed)));
        }

        // Privileged passthrough — the Claude Code capability advantage.
        if let Some(hooks) = &canonical.hooks {
            frontmatter.push(("hooks".to_owned(), json_to_yaml(hooks)));
        }
        if let Some(mcp) = &canonical.mcp_servers {
            frontmatter.push(("mcpServers".to_owned(), json_to_yaml(mcp)));
        }
        if let Some(mode) = &canonical.permission_mode {
            frontmatter.push(("permissionMode".to_owned(), yaml_str(mode)));
        }

        let rendered = agents::render_markdown_yaml(&frontmatter, &canonical.body);
        let filename = agent_filename(
            &canonical.plugin,
            &canonical.name,
            agent_extension(AgentFormat::MarkdownYaml),
        );

        Ok(TranslatedAgent {
            // Relative agent dir — informational. The sync writes to the
            // harness's `agent_dir(project_root)`; this records the
            // harness-relative directory for diagnostics without threading
            // the project root through the translation signature.
            dir: PathBuf::from(".claude/agents"),
            filename,
            displayed_name: name,
            format: AgentFormat::MarkdownYaml,
            rendered,
            dropped_fields: dropped,
        })
    }
}

/// A YAML scalar string value.
fn yaml_str(s: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(s.to_owned())
}

/// A YAML sequence of string values.
fn yaml_str_seq(items: &[String]) -> serde_yaml::Value {
    serde_yaml::Value::Sequence(items.iter().map(|s| yaml_str(s)).collect())
}

/// Convert an opaque privileged JSON blob into the equivalent YAML value.
///
/// The privileged fields (`hooks` / `mcpServers`) are forwarded verbatim
/// (FR-050); Tome neither interprets nor validates their shape. The
/// `serde_json::Value` → `serde_yaml::Value` round-trip through a string is
/// the simplest faithful conversion and never fails for an already-parsed
/// JSON value (every JSON value is a valid YAML value). On the impossible
/// failure path we fall back to a null so a malformed blob cannot abort the
/// whole translation.
fn json_to_yaml(value: &serde_json::Value) -> serde_yaml::Value {
    serde_json::to_string(value)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or(serde_yaml::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> CanonicalAgent {
        CanonicalAgent {
            catalog: "cat".into(),
            plugin: "myplugin".into(),
            name: "reviewer".into(),
            description: Some("Reviews code".into()),
            body: "You review.\n".into(),
            model: Some("opus".into()),
            tools: Some(vec!["Read".into(), "Grep".into()]),
            disallowed_tools: Some(vec!["Bash".into()]),
            hooks: Some(serde_json::json!({"PreToolUse": []})),
            mcp_servers: Some(serde_json::json!({"foo": {"command": "x"}})),
            permission_mode: Some("ask".into()),
        }
    }

    #[test]
    fn passes_privileged_fields_through_verbatim() {
        let t = CLAUDE_CODE.translate_agent(&agent(), false).unwrap();
        assert_eq!(t.filename, "myplugin__reviewer.md");
        assert_eq!(t.displayed_name, "reviewer");
        // Privileged fields survive (FR-050 default, no stripping in US1).
        assert!(
            t.rendered.contains("hooks:"),
            "hooks passed through:\n{}",
            t.rendered
        );
        assert!(
            t.rendered.contains("mcpServers:"),
            "mcpServers passed through"
        );
        assert!(t.rendered.contains("permissionMode: ask"));
        // model is same-vendor → verbatim alias.
        assert!(t.rendered.contains("model: opus"));
        assert!(t.dropped_fields.is_empty());
    }

    #[test]
    fn clash_prefixes_displayed_name_only() {
        let t = CLAUDE_CODE.translate_agent(&agent(), true).unwrap();
        // Filename unchanged; displayed name is plugin-prefixed.
        assert_eq!(t.filename, "myplugin__reviewer.md");
        assert_eq!(t.displayed_name, "myplugin-reviewer");
        assert!(t.rendered.contains("name: myplugin-reviewer"));
    }
}
