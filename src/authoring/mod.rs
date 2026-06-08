//! Authoring & conversion — the shared core behind `tome {catalog,plugin,skill}
//! {create,convert,lint}` (Phase 8, CLI-only).
//!
//! The three commands are compositions over one normalized in-memory **artifact
//! IR** ([`ir`]), one **emitter** ([`emit`]), one **rule registry**
//! ([`lint`], consumed by both `convert` and `lint`), one **harness-ism
//! rewrite** ([`rewrite`], applied by `convert` and flagged by `lint`), and the
//! existing atomic-landing + symlink-refusal write guards (`util::atomic_dir`,
//! `util::symlink_safe`). Source detection lives in [`detect`], source→IR
//! importers in [`import`], and template scaffolding in [`scaffold`].
//!
//! ```text
//! create  = render(template) → IR → emit
//! convert = detect → import → rewrite → lint → emit
//! lint    = parse → IR → lint   (or rewrite + emit under --autofix)
//! ```
//!
//! This module is **sync** (the constitution's async island is `src/mcp/`
//! only) and never touches the SQLite index or its advisory lock. The MCP tool
//! surface for these commands is an explicit fast-follow, out of scope this
//! phase.

pub mod convert;
pub mod detect;
pub mod emit;
pub mod import;
pub mod ir;
pub mod lint;
pub mod rewrite;
pub mod scaffold;
pub mod untrusted;
