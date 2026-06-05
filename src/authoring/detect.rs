//! Source-format detection for `convert`. Structural signals decide the source
//! harness and artifact level: `marketplace.json` → catalog; legacy
//! `.claude-plugin/plugin.json` → plugin; native `SKILL.md` under a harness dir
//! → skill; `config.toml [mcp_servers]` + `.agents/skills/` → Codex project.
//! A `--from <harness>` flag overrides detection; an undetectable source →
//! `SourceFormatUnrecognized` (83); a requested-vs-detected **level** mismatch
//! → `Usage` (2). See `research.md §R19`.
//!
//! Populated in Phase 4 (US2).
