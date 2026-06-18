//! Rules-file integration — block insertion / standalone-file writer.
//!
//! Production wiring for the two strategies declared by
//! [`crate::harness::RulesFileStrategy`]. The sync algorithm (US1.b-3)
//! and the summariser (US4) feed the writer the body content; this
//! module owns the on-disk byte format and the atomic-write discipline.
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
//! `<!-- tome:begin -->\n<body>\n<!-- tome:end -->\n`. The regex is
//! compiled once via `std::sync::OnceLock`.
//!
//! ## Atomic-write discipline
//!
//! Every write follows the Phase 1 atomic-write pattern: read existing
//! content into memory, construct new content, write to a sibling temp
//! file on the same filesystem, fsync, atomic rename onto the target.
//! Symlinks at the target path are refused (security hardening carried
//! from Phase 3 P8 PR-F — `symlink_metadata().is_symlink()` check →
//! `TomeError::Io` / exit 7).
//!
//! ## Idempotence
//!
//! Both block and standalone writers short-circuit when the on-disk
//! bytes already match the desired output (FR-525). This makes
//! `tome workspace use` re-runs zero-syscall on the write path when
//! nothing has changed — required for the sync-idempotence tests in
//! US1.b-3.

use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use tempfile::NamedTempFile;

use crate::error::TomeError;
use crate::harness::{BlockBodyStyle, RulesFrontmatter};

/// Line-anchored regex matching either marker, with trailing whitespace
/// tolerated. Pinned here per data-model §11.
pub const BLOCK_MARKER_REGEX: &str = r"^<!-- tome:(begin|end) -->\s*$";

/// The exact bytes emitted for the begin marker (no trailing newline —
/// the writer adds it).
pub const BLOCK_BEGIN: &str = "<!-- tome:begin -->";

/// The exact bytes emitted for the end marker.
pub const BLOCK_END: &str = "<!-- tome:end -->";

/// Compile-once cache for the marker regex.
fn marker_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(BLOCK_MARKER_REGEX).expect("static marker regex compiles"))
}

/// Parsed view of an existing Tome block within a rules file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    pub begin_line: usize,
    pub end_line: usize,
    pub body: String,
}

/// Internal: classify what each line is.
enum MarkerKind {
    Begin,
    End,
}

fn classify_line(line: &str) -> Option<MarkerKind> {
    let caps = marker_regex().captures(line)?;
    match caps.get(1).map(|m| m.as_str()) {
        Some("begin") => Some(MarkerKind::Begin),
        Some("end") => Some(MarkerKind::End),
        _ => None,
    }
}

/// Find ALL well-formed blocks in `contents`, in document order.
///
/// Returns an empty Vec when zero begin markers exist. Returns an `Err`
/// when markers are mismatched (begin without end, end before begin,
/// nested begins). The classification rules:
///
/// - A `begin` followed (eventually) by an `end` without an intervening
///   second `begin` is a well-formed block.
/// - A second `begin` before the matching `end` is malformed.
/// - An `end` with no preceding `begin` is malformed.
/// - A `begin` with no matching `end` is malformed.
fn find_all_blocks(contents: &str) -> Result<Vec<ParsedBlock>, TomeError> {
    let lines: Vec<&str> = contents.split('\n').collect();
    let mut blocks = Vec::new();
    let mut current_begin: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        match classify_line(line) {
            Some(MarkerKind::Begin) => {
                if current_begin.is_some() {
                    return Err(TomeError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "malformed Tome block: nested <!-- tome:begin --> markers",
                    )));
                }
                current_begin = Some(idx);
            }
            Some(MarkerKind::End) => {
                let begin = current_begin.take().ok_or_else(|| {
                    TomeError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "malformed Tome block: <!-- tome:end --> without matching begin",
                    ))
                })?;
                let body = if idx > begin + 1 {
                    lines[(begin + 1)..idx].join("\n")
                } else {
                    String::new()
                };
                blocks.push(ParsedBlock {
                    begin_line: begin,
                    end_line: idx,
                    body,
                });
            }
            None => {}
        }
    }
    if current_begin.is_some() {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed Tome block: <!-- tome:begin --> without matching end",
        )));
    }
    Ok(blocks)
}

/// Parse a rules file's contents looking for the canonical Tome block.
///
/// Returns `Ok(None)` when no markers are found. Returns
/// `Ok(Some(first))` when one or more well-formed blocks exist — the
/// first block wins. Returns an error when markers are mismatched
/// (nested begins, unmatched end, unterminated begin).
///
/// Per the contract's "Multiple Tome blocks in the same file" edge
/// case, the writer is responsible for collapsing subsequent blocks
/// during the rewrite. `parse_block` itself just surfaces the canonical
/// position.
pub fn parse_block(contents: &str) -> Result<Option<ParsedBlock>, TomeError> {
    Ok(find_all_blocks(contents)?.into_iter().next())
}

/// Format the canonical block payload (no surrounding text).
fn format_block(body: &str) -> String {
    format!("{BLOCK_BEGIN}\n{body}\n{BLOCK_END}\n")
}

/// Build the new file contents for a block-write operation.
///
/// Handles the four cases:
/// - File is empty/missing → just the block.
/// - File has existing content, no block → append with separator.
/// - File has one block → replace body in place.
/// - File has multiple blocks → replace the first, drop subsequent.
fn compose_block_write(existing: &str, body: &str) -> Result<String, TomeError> {
    let blocks = find_all_blocks(existing)?;
    if blocks.is_empty() {
        if existing.is_empty() {
            return Ok(format_block(body));
        }
        // Separator: existing content + "\n" (if not already ending in
        // one) + "\n" (blank line) + block.
        let mut out = String::with_capacity(existing.len() + body.len() + 64);
        out.push_str(existing);
        if !existing.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&format_block(body));
        return Ok(out);
    }

    // One or more blocks. Splice in the new first block, drop the
    // rest. Build output by walking the line list and tracking which
    // indices to emit.
    let lines: Vec<&str> = existing.split('\n').collect();
    let first = &blocks[0];

    let mut emitted: Vec<String> = Vec::with_capacity(lines.len());
    let mut idx = 0;
    while idx < lines.len() {
        if idx == first.begin_line {
            // The replacement block is emitted as three "lines"
            // (begin marker, body line(s), end marker) so the eventual
            // `\n`-join produces the canonical byte format.
            emitted.push(BLOCK_BEGIN.to_string());
            // Body may itself be multi-line.
            for line in body.split('\n') {
                emitted.push(line.to_string());
            }
            emitted.push(BLOCK_END.to_string());
            idx = first.end_line + 1;
            continue;
        }
        // Skip subsequent blocks entirely (begin..=end inclusive).
        let in_dropped_block = blocks[1..]
            .iter()
            .find(|b| idx >= b.begin_line && idx <= b.end_line);
        if let Some(b) = in_dropped_block {
            idx = b.end_line + 1;
            continue;
        }
        emitted.push(lines[idx].to_string());
        idx += 1;
    }

    Ok(emitted.join("\n"))
}

/// Refuse to write through a symlinked component. Returns Ok if no existing
/// component (below the trusted anchor) is a symlink — including an absent
/// path; Err if a symlinked component is found.
///
/// Delegates to the SSOT guard (`util::symlink_safe`) so the rules-file sink
/// gets the intermediate-component hardening (FR-007), not just the final-node
/// check. A refusal maps to `TomeError::Io` (exit 7) — the dedicated code this
/// sink already used. Promoted to `pub(crate)` so the guardrails writer
/// (`guardrails.rs`) reuses the same discipline; guardrails re-maps the
/// returned error onto exit 46 at its call sites.
pub(crate) fn refuse_symlink(target: &Path) -> Result<(), TomeError> {
    crate::util::refuse_symlinked_component(target).map_err(TomeError::Io)
}

/// Atomic write: temp file in same dir → fsync → rename.
///
/// On Unix, when `target` already exists, captures its file mode and
/// chmods the staging tempfile to match before persisting. Preserves
/// any developer-set mode bits (e.g. group-readable workspaces) across
/// the rewrite. If `target` is absent, the tempfile's libc-default mode
/// (typically 0o600) wins.
///
/// Promoted to `pub(crate)` so the guardrails writer (`guardrails.rs`)
/// reuses the same atomic-rename + mode-preservation discipline.
pub(crate) fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), TomeError> {
    let parent = target
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("rules-file path has no parent")))?;
    std::fs::create_dir_all(parent).map_err(TomeError::Io)?;

    #[cfg(unix)]
    let target_mode: Option<u32> = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(target)
            .ok()
            .map(|m| m.permissions().mode())
    };

    let mut tmp = NamedTempFile::with_prefix_in(".tome.tmp.", parent).map_err(TomeError::Io)?;
    tmp.write_all(bytes).map_err(TomeError::Io)?;
    tmp.as_file().sync_all().map_err(TomeError::Io)?;

    #[cfg(unix)]
    if let Some(mode) = target_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(mode))
            .map_err(TomeError::Io)?;
    }

    tmp.persist(target).map_err(|e| TomeError::Io(e.error))?;
    Ok(())
}

/// Write (or update) the Tome block inside the file at `target`.
///
/// The `_style` parameter is forward-looking: callers in US4 may pick
/// the body composition based on the harness's `BlockBodyStyle`, but
/// the writer itself emits `body` verbatim between the markers.
///
/// Refuses to write through a symlink (security hardening — exit 7 /
/// `TomeError::Io`). Idempotent: when the on-disk first block already
/// has the same body, no write is performed.
pub fn write_block(target: &Path, body: &str, _style: BlockBodyStyle) -> Result<(), TomeError> {
    refuse_symlink(target)?;
    let existing = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX)
    {
        Ok(s) => s,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    // Idempotence: single existing block whose body matches → no-op.
    // C-M6 (US3 review): the multi-block case (`blocks.len() > 1`) is
    // intentionally NOT short-circuited even when `blocks[0].body == body`.
    // Multiple Tome blocks indicate a hand-edit or a prior partial write;
    // the contract requires us to collapse to a single canonical block
    // even when the first one happens to match what we'd write. The
    // collapse IS the convergent action — leaving extra blocks in place
    // would violate FR-525 (byte-for-byte idempotence on the second pass).
    let blocks = find_all_blocks(&existing)?;
    if blocks.len() == 1 && blocks[0].body == body {
        return Ok(());
    }

    let new_contents = compose_block_write(&existing, body)?;
    atomic_write(target, new_contents.as_bytes())
}

/// Remove the Tome block from the file at `target` (if present).
///
/// Surrounding content is preserved verbatim. A single blank-line
/// separator preceding the block (the one inserted by `write_block` when
/// it appended to existing content) is consumed during removal. If the
/// file would be left empty after removal, it is kept in place with
/// empty content — the developer authored it.
///
/// Refuses to write through a symlink. Idempotent: if no block is
/// present, no write is performed.
pub fn remove_block(target: &Path) -> Result<(), TomeError> {
    refuse_symlink(target)?;
    let existing = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX)
    {
        Ok(s) => s,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    let blocks = find_all_blocks(&existing)?;
    if blocks.is_empty() {
        return Ok(());
    }

    // Splice out every block. For each, also consume one preceding
    // blank line (the separator) when present.
    let lines: Vec<&str> = existing.split('\n').collect();
    let mut drop_indices = std::collections::HashSet::new();
    for block in &blocks {
        for i in block.begin_line..=block.end_line {
            drop_indices.insert(i);
        }
        // Consume the single immediately-preceding blank line, if any
        // and if it isn't already part of another block.
        if block.begin_line > 0 {
            let prev = block.begin_line - 1;
            if lines[prev].is_empty() && !drop_indices.contains(&prev) {
                drop_indices.insert(prev);
            }
        }
    }

    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if drop_indices.contains(&i) {
                None
            } else {
                Some(*l)
            }
        })
        .collect();

    let mut new_contents = kept.join("\n");
    // If the only surviving content is a trailing empty string (from a
    // single trailing newline) collapse to empty.
    if kept.iter().all(|l| l.is_empty()) {
        new_contents = String::new();
    }

    atomic_write(target, new_contents.as_bytes())
}

/// Write the standalone Tome-owned rules file at `target`.
///
/// `contents` is written verbatim — no markers, no transformation. The
/// parent directory is created (mode 0700 on Unix) if missing. Refuses
/// to write through a symlink. Idempotent: when the on-disk bytes
/// already match `contents`, no write is performed.
pub fn write_standalone(target: &Path, contents: &str) -> Result<(), TomeError> {
    refuse_symlink(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("standalone path has no parent")))?;
    let parent_existed = parent.exists();
    std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    #[cfg(unix)]
    if !parent_existed {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(TomeError::Io)?;
    }
    #[cfg(not(unix))]
    let _ = parent_existed;

    // Idempotence: same on-disk bytes → no write.
    if let Ok(existing) =
        crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX)
        && existing == contents
    {
        return Ok(());
    }

    atomic_write(target, contents.as_bytes())
}

/// Render the byte-stable file contents for a frontmatter-fronted standalone
/// rules file (G3, FR-026).
///
/// Format (deterministic — key order is the `fields` slice order):
///
/// ```text
/// ---
/// <key>: <value>
/// …
/// ---
/// <body>
/// ```
///
/// A trailing newline always follows the closing `---`; `body` is emitted
/// verbatim after it. Tome owns every key/value (they are `&'static`
/// constants), so they are NOT scanned for marker collisions — only
/// third-party content gets the verbatim-collision guard.
fn render_frontmatter(frontmatter: &RulesFrontmatter, body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 32);
    out.push_str("---\n");
    for (key, value) in frontmatter.fields {
        out.push_str(key);
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// Write the standalone Tome-owned rules file at `target` with a Tome-owned
/// YAML front-matter header above the verbatim `body` (G3, FR-026).
///
/// The emitted bytes are `render_frontmatter(frontmatter, body)` — a
/// `---`-fenced header whose key order is the `frontmatter.fields` slice
/// order, then `body` verbatim. Same symlink-refusal + atomic-write +
/// idempotence discipline as [`write_standalone`]; the only difference is the
/// composed payload. Used by harnesses whose standalone sink requires a
/// front-matter directive (kiro `inclusion: always`, jetbrains-ai apply-mode).
pub fn write_standalone_with_frontmatter(
    target: &Path,
    frontmatter: &RulesFrontmatter,
    body: &str,
) -> Result<(), TomeError> {
    let contents = render_frontmatter(frontmatter, body);
    write_standalone(target, &contents)
}

/// Remove the standalone Tome-owned rules file at `target` (if
/// present). The containing directory is untouched.
pub fn remove_standalone(target: &Path) -> Result<(), TomeError> {
    refuse_symlink(target)?;
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TomeError::Io(e)),
    }
}

// =====================================================================
// Parameterised marker regions (Phase 6 / US3, R-5, R-19)
//
// The `tome:begin/end` block above is the single Tome rules-include
// block. Guardrails (US3) need MANY managed regions on the same file —
// one per plugin — each delimited by its own keyed marker pair:
//
//   <!-- START GUARDRAILS: <catalog>:<plugin> -->
//   <verbatim body>
//   <!-- END GUARDRAILS: <catalog>:<plugin> -->
//
// This section generalises the block find/replace to a parameterised
// marker pair so guardrails regions coexist with the `tome:begin/end`
// block without collision. The guardrails module (`guardrails.rs`) owns
// the GUARDRAILS-specific regex strings + ordering; this engine is the
// reusable line-anchored find / replace / remove machinery.
// =====================================================================

/// A marker-delimited region keyed by an opaque provenance string (for
/// guardrails, the `<catalog>:<plugin>` text captured from the START
/// marker). Line indices are into the `\n`-split contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerRegion {
    pub key: String,
    pub begin_line: usize,
    pub end_line: usize,
    pub body: String,
}

/// The compiled START / END regex pair for one family of keyed regions.
///
/// `start` MUST expose a capture group named `key` whose value is the
/// region's provenance key; `end` is matched by reconstructing the exact
/// END marker for that key (so a START for key A followed by an END for
/// key B is malformed — markers cannot interleave).
pub struct MarkerSpec {
    start: Regex,
    /// Builds the canonical END-marker line for `key` (no trailing
    /// newline), used both to emit and to verify a matched END belongs to
    /// the currently-open START.
    end_for: fn(&str) -> String,
    /// Builds the canonical START-marker line for `key`.
    begin_for: fn(&str) -> String,
    /// Matches any END marker line (key-agnostic) so a stray / mismatched
    /// END is detected rather than silently treated as body text.
    end_any: Regex,
}

impl MarkerSpec {
    /// Construct a spec from the START regex (must capture `key`), an
    /// END-any regex, and two closures rendering the canonical
    /// begin/end marker lines for a key.
    pub fn new(
        start: Regex,
        end_any: Regex,
        begin_for: fn(&str) -> String,
        end_for: fn(&str) -> String,
    ) -> Self {
        Self {
            start,
            end_for,
            begin_for,
            end_any,
        }
    }

    fn start_key(&self, line: &str) -> Option<String> {
        self.start
            .captures(line)
            .and_then(|c| c.name("key").map(|m| m.as_str().to_string()))
    }
}

/// Find ALL well-formed keyed regions in `contents`, in document order.
///
/// Mismatched markers (a START whose matching END never appears, an END
/// with no open START, a START before the prior region's END, or an END
/// whose key differs from the open START) are malformed → `Err`. This
/// mirrors `find_all_blocks`'s strictness so a hand-mangled region fails
/// loudly rather than silently dropping content.
pub fn find_marker_regions(
    spec: &MarkerSpec,
    contents: &str,
) -> Result<Vec<MarkerRegion>, TomeError> {
    let lines: Vec<&str> = contents.split('\n').collect();
    let mut regions = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(key) = spec.start_key(line) {
            if open.is_some() {
                return Err(malformed_region("nested START marker"));
            }
            open = Some((idx, key));
            continue;
        }
        if spec.end_any.is_match(line) {
            let (begin, key) = open
                .take()
                .ok_or_else(|| malformed_region("END marker without matching START"))?;
            // The matched END must be exactly the canonical END for the open
            // key — an interleaved/mismatched key is malformed.
            if *line != (spec.end_for)(&key) && line.trim_end() != (spec.end_for)(&key) {
                return Err(malformed_region("END marker key does not match open START"));
            }
            let body = if idx > begin + 1 {
                lines[(begin + 1)..idx].join("\n")
            } else {
                String::new()
            };
            regions.push(MarkerRegion {
                key,
                begin_line: begin,
                end_line: idx,
                body,
            });
        }
    }
    if open.is_some() {
        return Err(malformed_region("START marker without matching END"));
    }
    Ok(regions)
}

fn malformed_region(reason: &str) -> TomeError {
    TomeError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("malformed marker region: {reason}"),
    ))
}

/// Render the canonical region payload for `key` with `body` between the
/// markers (no trailing newline — the composer adds line joins).
pub fn format_marker_region(spec: &MarkerSpec, key: &str, body: &str) -> String {
    format!(
        "{}\n{}\n{}",
        (spec.begin_for)(key),
        body,
        (spec.end_for)(key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_block_returns_none_for_empty() {
        assert_eq!(parse_block("").unwrap(), None);
    }

    #[test]
    fn parse_block_returns_none_when_no_markers() {
        assert_eq!(parse_block("hello\nworld\n").unwrap(), None);
    }

    #[test]
    fn parse_block_returns_first_well_formed_block() {
        let s = "top\n<!-- tome:begin -->\nbody\n<!-- tome:end -->\nbottom\n";
        let block = parse_block(s).unwrap().unwrap();
        assert_eq!(block.body, "body");
        assert_eq!(block.begin_line, 1);
        assert_eq!(block.end_line, 3);
    }

    #[test]
    fn parse_block_tolerates_trailing_whitespace_on_markers() {
        let s = "<!-- tome:begin -->   \nx\n<!-- tome:end -->\n";
        let block = parse_block(s).unwrap().unwrap();
        assert_eq!(block.body, "x");
    }

    #[test]
    fn parse_block_errors_on_nested_begin() {
        let s = "<!-- tome:begin -->\n<!-- tome:begin -->\n<!-- tome:end -->\n";
        assert!(parse_block(s).is_err());
    }

    #[test]
    fn parse_block_errors_on_unmatched_end() {
        let s = "<!-- tome:end -->\n";
        assert!(parse_block(s).is_err());
    }

    #[test]
    fn parse_block_errors_on_unterminated_begin() {
        let s = "<!-- tome:begin -->\nbody\n";
        assert!(parse_block(s).is_err());
    }

    #[test]
    fn parse_block_returns_first_when_multiple_present() {
        let s = "<!-- tome:begin -->\nfirst\n<!-- tome:end -->\n<!-- tome:begin -->\nsecond\n<!-- tome:end -->\n";
        let block = parse_block(s).unwrap().unwrap();
        assert_eq!(block.body, "first");
    }

    #[test]
    fn render_frontmatter_kiro_shape_pins_exact_bytes() {
        let fm = RulesFrontmatter {
            fields: &[("inclusion", "always")],
        };
        assert_eq!(
            render_frontmatter(&fm, "the directive\n"),
            "---\ninclusion: always\n---\nthe directive\n",
        );
    }

    #[test]
    fn render_frontmatter_jetbrains_shape_pins_exact_bytes() {
        let fm = RulesFrontmatter {
            fields: &[("apply", "always")],
        };
        assert_eq!(
            render_frontmatter(&fm, "body"),
            "---\napply: always\n---\nbody",
        );
    }

    #[test]
    fn render_frontmatter_preserves_field_slice_order() {
        let fm = RulesFrontmatter {
            fields: &[("b", "2"), ("a", "1")],
        };
        assert_eq!(render_frontmatter(&fm, "x"), "---\nb: 2\na: 1\n---\nx");
    }

    #[test]
    fn write_standalone_with_frontmatter_round_trips_and_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join(".kiro/steering/tome.md");
        let fm = RulesFrontmatter {
            fields: &[("inclusion", "always")],
        };
        write_standalone_with_frontmatter(&target, &fm, "rules body\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "---\ninclusion: always\n---\nrules body\n",
        );
        // Second write with identical inputs is a no-op (idempotence inherited
        // from `write_standalone`).
        write_standalone_with_frontmatter(&target, &fm, "rules body\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "---\ninclusion: always\n---\nrules body\n",
        );
    }
}
