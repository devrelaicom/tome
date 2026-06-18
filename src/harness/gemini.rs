//! `gemini` — Google Gemini CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.gemini/`.
//! - Rules-file target: `AGENTS.md` > `GEMINI.md` > `.gemini/GEMINI.md`
//!   (first existing wins; falls back to `AGENTS.md` if none exist).
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `~/.gemini/settings.json` (global).
//! - Parent key: `"mcpServers"`.
//! - Guardrails target (Phase 6): `AGENTS.md` preferred else `GEMINI.md`,
//!   in-file region, no suppression — the trait default (`InFileRegion` on
//!   the rules-file target) yields exactly this, so no `guardrails_target`
//!   override is needed (FR-012).

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Gemini CLI.
pub struct Gemini;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const GEMINI: Gemini = Gemini;

impl HarnessModule for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn description(&self) -> &'static str {
        "Google Gemini CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".gemini").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        for candidate in ["AGENTS.md", "GEMINI.md", ".gemini/GEMINI.md"] {
            let p = project_root.join(candidate);
            if p.exists() {
                return p;
            }
        }
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        home.join(".gemini/settings.json")
    }

    // MCP dialect: the trait default ([`McpDialect::LEGACY`]) is exactly
    // Gemini's shape (JSON `mcpServers` + `CommandArgs`), so no override.
}
