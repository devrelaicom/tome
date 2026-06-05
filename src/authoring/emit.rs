//! IR → on-disk Tome format. Writes `tome-catalog.toml` / `tome-plugin.toml` /
//! `SKILL.md` (+ supporting files) via `util::atomic_dir` staging and the
//! `util::symlink_safe` write guard (final node **and** intermediate
//! component). Field/format order is deterministic so emitted manifests and
//! rewritten bodies are byte-stable and snapshot-pinnable (FR-027).
//!
//! `catalog convert` stages the entire tree and lands it atomically —
//! all-or-nothing (FR-014a). Populated in Phase 2 (Foundational).
