//! Rules-file integration — block insertion / standalone-file writer.
//!
//! Skeleton only. Production wiring lands in US3.c / US4 alongside the
//! sync algorithm and the summariser, both of which feed the writer.
//!
//! ## Strategies (per `contracts/rules-file-integration.md`)
//!
//! - `BlockInExistingFile` — Tome owns a `<!-- tome:begin --> …
//!   <!-- tome:end -->` block inside an existing developer-authored
//!   rules file. Content outside the markers is preserved verbatim.
//! - `StandaloneFile` — Tome owns a complete file at the harness's
//!   chosen path. Removal deletes the file.
//!
//! ## Block markers (per data-model §11)
//!
//! ```text
//! <!-- tome:begin -->
//! <body>
//! <!-- tome:end -->
//! ```
//!
//! Match regex (line-anchored, trailing-whitespace tolerated):
//! `^<!-- tome:(begin|end) -->\s*$`. Emit format:
//! `<!-- tome:begin -->\n<body>\n<!-- tome:end -->\n`. The regex literal
//! is pinned here so the marker contract stays close to the code that
//! reads it. US4 compiles the regex once at startup.
//!
//! ## Atomic-write discipline
//!
//! Every write follows the Phase 1 atomic-write pattern: read existing
//! content into memory, construct new content, write to a sibling temp
//! file on the same filesystem, fsync, atomic rename onto the target.
//! Symlinks at the target path are refused (security hardening carried
//! from Phase 3 P8 PR-F — `is_symlink()` check → exit 7).

use std::path::Path;

use crate::error::TomeError;
use crate::harness::BlockBodyStyle;

/// Line-anchored regex matching either marker, with trailing whitespace
/// tolerated. Pinned here per data-model §11.
pub const BLOCK_MARKER_REGEX: &str = r"^<!-- tome:(begin|end) -->\s*$";

/// The exact bytes emitted for the begin marker (no trailing newline —
/// the writer adds it).
pub const BLOCK_BEGIN: &str = "<!-- tome:begin -->";

/// The exact bytes emitted for the end marker.
pub const BLOCK_END: &str = "<!-- tome:end -->";

/// Parsed view of an existing Tome block within a rules file.
///
/// US4 fills this in with byte offsets (`begin_line`, `end_line`) plus
/// the body slice. F7 only sketches the shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    pub begin_line: usize,
    pub end_line: usize,
    pub body: String,
}

/// Parse a rules file's contents looking for the canonical Tome block.
///
/// Returns `Ok(Some(_))` when exactly one well-formed block is present,
/// `Ok(None)` when no markers are found, and an error when the file is
/// malformed (e.g. unmatched begin/end, multiple begins). US4 will
/// canonicalise multiple blocks per the contract (collapse to the first;
/// remove subsequent).
#[allow(unused_variables)]
pub fn parse_block(_contents: &str) -> Result<Option<ParsedBlock>, TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Write (or update) the Tome block inside the file at `target`.
///
/// The body is computed from `style` plus the project marker's
/// `RULES.md` path (passed via callers in US4 — F7 keeps the signature
/// minimal). Refuses to write through a symlink (`is_symlink()` check →
/// exit 7 / `TomeError::Io`).
#[allow(unused_variables)]
pub fn write_block(_target: &Path, _body: &str, _style: BlockBodyStyle) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Remove the Tome block from the file at `target` (if present).
///
/// Surrounding content is preserved verbatim. If the file would be left
/// empty after removal, it is kept in place (the developer authored
/// it).
#[allow(unused_variables)]
pub fn remove_block(_target: &Path) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Write the standalone Tome-owned rules file at `target`.
///
/// `contents` is the project marker's `RULES.md` body verbatim. The
/// parent directory is created (mode 0700 on Unix) if missing. Refuses
/// to write through a symlink.
#[allow(unused_variables)]
pub fn write_standalone(_target: &Path, _contents: &str) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Remove the standalone Tome-owned rules file at `target` (if
/// present). The containing directory is untouched.
#[allow(unused_variables)]
pub fn remove_standalone(_target: &Path) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}
