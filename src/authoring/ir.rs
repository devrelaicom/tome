//! The normalized artifact IR — the in-memory model every authoring command
//! produces and consumes (catalog → plugins → entries → diagnostics).
//!
//! Not serialized to disk: [`super::emit`] writes the on-disk Tome format
//! (`tome-catalog.toml` / `tome-plugin.toml` / `SKILL.md`), and diagnostics
//! flow to the command report. With native-`SKILL.md`-only conversion the IR
//! is near-identical to the emitted format, so the per-harness importer code
//! stays a thin source→IR parser. See `data-model.md §4`.
//!
//! Populated in Phase 2 (Foundational).
