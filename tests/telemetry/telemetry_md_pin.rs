//! Phase 10 / US5 (T069, FR-062, R-15) — the `TELEMETRY.md` wire-shape pin.
//!
//! `TELEMETRY.md` (tracked, repo root) documents EXACTLY what Tome emits. Its §8
//! "Worked examples" embeds two copy-pasteable JSON lines — one `tome.install`,
//! one `catalog.midnight.entry_invoked` — each marked by an HTML comment
//! (`<!-- TELEMETRY_PIN: <event_type> -->`) and followed by a fenced ```json
//! block. This test `include_str!`s the document, extracts those two blocks, and
//! asserts each equals the matching typed-constructor output BYTE-FOR-BYTE.
//!
//! This is the FR-062 drift guard: if the document's worked example diverges from
//! what the event constructors actually serialize — a field reorder, a renamed
//! enum token, a new/dropped field, a version that isn't `0.6.0` — CI breaks here.
//! Because `fixed_envelope_for_tests` pins `tome_version` to a FIXED test
//! constant (not `env!("CARGO_PKG_VERSION")`), a crate-version bump does NOT
//! break this schema pin: the version is data on the wire, not schema.
//!
//! It also asserts both markers exist and that EXACTLY ONE ```json block follows
//! each, so a doc edit that drops a marker or its example fails CI too.

use tome::telemetry::event::{
    AttributedEntryInvoked, EntryKind, Harness, Install, InstallMethod,
    fixed_attributed_envelope_for_tests, fixed_envelope_for_tests, to_line,
};

/// The authored document, embedded at compile time. The relative path is from
/// THIS file (`tests/telemetry/telemetry_md_pin.rs`) to the repo root.
const TELEMETRY_MD: &str = include_str!("../../TELEMETRY.md");

const MARKER_INSTALL: &str = "<!-- TELEMETRY_PIN: tome.install -->";
const MARKER_ATTRIBUTED: &str = "<!-- TELEMETRY_PIN: catalog.midnight.entry_invoked -->";

/// Extract the single JSON line of the ```json fenced block that immediately
/// FOLLOWS `marker` in `doc`. Panics with a clear message if the marker is
/// missing, no ```json fence follows it, the fence is unterminated, or the block
/// does not hold exactly one non-blank line.
fn extract_pinned_json(doc: &str, marker: &str) -> String {
    let after_marker = doc
        .split_once(marker)
        .unwrap_or_else(|| panic!("TELEMETRY.md is missing the marker `{marker}`"))
        .1;

    // Find the opening ```json fence after the marker.
    let fence_open = "```json";
    let rest = after_marker.split_once(fence_open).unwrap_or_else(|| {
        panic!("no ```json block follows the marker `{marker}` in TELEMETRY.md")
    });
    // Everything between the opening fence and the next ``` is the block body.
    let body_and_after = rest.1.trim_start_matches('\n');
    let block = body_and_after
        .split_once("```")
        .unwrap_or_else(|| panic!("unterminated ```json block after marker `{marker}`"))
        .0;

    let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly ONE JSON line in the block after `{marker}`, found {}:\n{block}",
        lines.len(),
    );
    lines[0].to_string()
}

/// Count how many ```json blocks follow `marker` up to the NEXT marker (or EOF).
/// Used to assert exactly one example block is attached to each marker.
fn count_json_blocks_after(doc: &str, marker: &str, next_marker: Option<&str>) -> usize {
    let after = doc
        .split_once(marker)
        .unwrap_or_else(|| panic!("marker `{marker}` missing"))
        .1;
    // Bound the search region to before the next marker so each marker's blocks
    // are counted independently.
    let region = match next_marker.and_then(|nm| after.split_once(nm)) {
        Some((before, _)) => before,
        None => after,
    };
    region.matches("```json").count()
}

#[test]
fn both_markers_present_each_with_exactly_one_json_block() {
    assert!(
        TELEMETRY_MD.contains(MARKER_INSTALL),
        "TELEMETRY.md must carry the `{MARKER_INSTALL}` marker"
    );
    assert!(
        TELEMETRY_MD.contains(MARKER_ATTRIBUTED),
        "TELEMETRY.md must carry the `{MARKER_ATTRIBUTED}` marker"
    );
    // The install marker precedes the attributed one in §8, so bound the
    // install marker's region at the attributed marker.
    assert_eq!(
        count_json_blocks_after(TELEMETRY_MD, MARKER_INSTALL, Some(MARKER_ATTRIBUTED)),
        1,
        "exactly one ```json block must follow the tome.install marker"
    );
    assert_eq!(
        count_json_blocks_after(TELEMETRY_MD, MARKER_ATTRIBUTED, None),
        1,
        "exactly one ```json block must follow the catalog.midnight.entry_invoked marker"
    );
}

#[test]
fn install_worked_example_matches_constructor_byte_for_byte() {
    // The EXACT event the doc's §8 example shows: a `tome.install` with the brew
    // install method, behind the canonical fixed envelope.
    let envelope = fixed_envelope_for_tests("tome.install");
    let event = Install {
        install_method: InstallMethod::Brew,
    };
    let constructed = to_line(&envelope, &event).expect("install event serialises");

    let documented = extract_pinned_json(TELEMETRY_MD, MARKER_INSTALL);

    assert_eq!(
        documented, constructed,
        "TELEMETRY.md tome.install worked example drifted from the constructor.\n\
         documented : {documented}\n\
         constructed: {constructed}"
    );
}

#[test]
fn attributed_entry_invoked_worked_example_matches_constructor_byte_for_byte() {
    // The EXACT event the doc's §8 example 2 shows (data-model §10 example-2
    // values): a `catalog.midnight.entry_invoked` behind the attributed envelope
    // (NO sample_rate — attributed events are never sampled).
    let envelope =
        fixed_attributed_envelope_for_tests("catalog.midnight.entry_invoked".to_string());
    let event = AttributedEntryInvoked {
        entry_name: "midnight-compact-debug".into(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".into(),
        plugin_version: "1.2.0".into(),
        catalog_id: "midnight",
        calling_harness: Some(Harness::ClaudeCode),
    };
    let constructed = to_line(&envelope, &event).expect("attributed event serialises");

    let documented = extract_pinned_json(TELEMETRY_MD, MARKER_ATTRIBUTED);

    assert_eq!(
        documented, constructed,
        "TELEMETRY.md catalog.midnight.entry_invoked worked example drifted from the constructor.\n\
         documented : {documented}\n\
         constructed: {constructed}"
    );
}
