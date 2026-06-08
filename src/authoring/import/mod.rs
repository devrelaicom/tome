//! Source → IR importers (the per-harness parsers behind `convert`).
//!
//! Every importer treats the source tree as **untrusted**: reads/copies stay
//! within the resolved source root, symlinked components and `..`/absolute
//! escapes are refused, copied filenames pass safe-segment validation, reads
//! are bounded by the per-class cap, and bodies must be valid UTF-8
//! (fail-closed). Third-party inputs (Codex `config.toml`, source frontmatter)
//! are parsed **leniently** — unknown fields warn, never abort (the strictness
//! boundary, principle IV).
//!
//! Tier 1 (port closely): Claude Code marketplaces/plugins + native `SKILL.md`
//! from Cursor/OpenCode/Cline/generic Agent Skills. Tier 2 (best-effort
//! synthesis): Codex projects.
//!
//! Populated in Phase 4 (US2): `claude_code`, `agent_skills` (+ thin
//! `cursor`/`opencode`/`cline`), `codex`.

pub mod claude_code;
pub mod codex;
pub mod native_skill;

/// Shared `convert` diagnostic rule ids — the single source of truth for the
/// per-importer diagnostics, so the `--strict` blocking-set
/// ([`crate::authoring::convert`]) and every importer agree on one stable
/// vocabulary. Promoted here when Codex became the second consumer of the
/// originally-CC-local set.
pub mod rule {
    // Manifest / project level.
    pub const MISSING_VERSION: &str = "convert/missing-version";
    pub const DROPPED_MANIFEST_FIELD: &str = "convert/dropped-manifest-field";
    pub const UNSUPPORTED_MANIFEST_FIELD: &str = "convert/unsupported-manifest-field";
    pub const UNSUPPORTED_COMPONENT: &str = "convert/unsupported-component";
    // Entry / frontmatter level.
    pub const DROPPED_FRONTMATTER: &str = "convert/dropped-frontmatter";
    pub const TOOL_RESTRICTION_DROPPED: &str = "convert/tool-restriction-dropped";
    pub const AGENT_LOSSY: &str = "convert/agent-lossy";
    pub const SKIPPED_ENTRY: &str = "convert/skipped-entry";
    pub const MALFORMED_MCP: &str = "convert/malformed-mcp-server";
    // Codex (Tier-2 synthesis) specific.
    pub const CODEX_SYNTHESIZED_VERSION: &str = "convert/codex-synthesized-version";
    pub const CODEX_UNSUPPORTED: &str = "convert/codex-unsupported";
}
