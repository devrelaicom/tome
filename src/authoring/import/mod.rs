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
