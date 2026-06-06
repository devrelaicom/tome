//! Consolidated integration-test binary for the Phase 8 `authoring` surface
//! (manifest cutover, convert, lint, create). Each former top-level file is a
//! submodule under `tests/authoring/`, sharing ONE compiled + linked binary.
//! `cargo test --test authoring` runs the group; `cargo test cutover::` filters
//! by member.

#[path = "authoring/cutover.rs"]
mod cutover;
