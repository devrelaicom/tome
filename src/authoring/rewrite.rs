//! Harness-ism rewrite — literal-token swaps over body strings (the
//! `data-model.md §7` table): `${CLAUDE_PLUGIN_ROOT}` → `${TOME_PLUGIN_DIR}`,
//! `${CLAUDE_PROJECT_DIR}` → `${TOME_PROJECT_DIR}`, legacy 1-based `$1..$9` →
//! Tome's 0-based positional form (for Claude Code *command* sources), and
//! warn-unmappable for the rest (`${CLAUDE_SESSION_ID}`, `` !`cmd` ``, `@file`,
//! …).
//!
//! Shared by `convert` (which applies the rewrite) and `lint --autofix` (which
//! applies the rewritable subset). This is a plain regex pass over known
//! tokens — **not** Tome's runtime substitution engine, so there is no
//! single-sweep concern. Bodies must be valid UTF-8 (fail-closed, FR-011a):
//! a token rewrite over a lossily-decoded body would corrupt U+FFFD'd bytes
//! into the deterministic snapshot output.
//!
//! Populated in Phase 2 (Foundational).
