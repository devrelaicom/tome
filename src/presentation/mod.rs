//! Thin wrappers around the Phase 2 presentation crates so individual
//! commands don't carry knowledge of `comfy-table`, `indicatif`, `owo-colors`,
//! or `inquire` directly. Each submodule encapsulates one capability:
//!
//! - [`tables`] — renders human-readable tables (`comfy-table`). Plain-text
//!   fallback when stdout is not a terminal.
//! - [`progress`] — progress bars and spinners (`indicatif`) that auto-hide
//!   when stderr is not a terminal (FR-042 / FR-043 / FR-046).
//! - [`colour`] — colour primitives (`owo-colors`) gated on `NO_COLOR`,
//!   `--no-color`, and TTY detection (FR-045 / FR-046).
//! - [`prompt`] — interactive prompts (`inquire`) that refuse to run without
//!   a connected terminal, returning [`TomeError::NotATerminal`] (FR-051).
//!
//! The public surface is intentionally small. Callers go through these
//! wrappers rather than pulling the underlying crates into command modules,
//! so swapping a renderer (e.g. dropping `comfy-table` per the binary-size
//! contingency ladder in research §R1) is a one-file change.

pub mod colour;
pub mod format;
pub mod progress;
pub mod prompt;
pub mod tables;
