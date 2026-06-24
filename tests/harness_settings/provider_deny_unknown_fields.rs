//! File-scoped `deny_unknown_fields` gate for the per-kind provider response
//! modules (FR-021).
//!
//! The four per-kind wire-shape modules — `openai`, `anthropic`, `gemini`,
//! `voyage` — parse THIRD-PARTY response bodies, which must be LENIENT: a
//! provider adding a field to its response must not break Tome's parse. The
//! `deny_unknown_fields` attribute is reserved for Tome-OWNED inputs (config,
//! manifests); applying it to a response struct would be a forward-compat
//! footgun.
//!
//! This test mirrors `sync_boundary.rs`'s file-scoped grep: it reads each of
//! the four files and asserts none contains the literal `deny_unknown_fields`.
//! The gate is intentionally scoped to ONLY these four files — Tome-owned
//! structs elsewhere (incl. `provider/config.rs` via the `[providers]` entries)
//! stay strict.

use std::fs;
use std::path::Path;

/// The per-kind response modules that MUST stay lenient. Relative to the crate
/// root (`CARGO_MANIFEST_DIR`).
const LENIENT_FILES: &[&str] = &[
    "src/provider/openai.rs",
    "src/provider/anthropic.rs",
    "src/provider/gemini.rs",
    "src/provider/voyage.rs",
];

#[test]
fn per_kind_response_modules_are_lenient() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations: Vec<String> = Vec::new();

    for rel in LENIENT_FILES {
        let path = root.join(rel);
        let contents =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if contents.contains("deny_unknown_fields") {
            violations.push(format!(
                "  {rel}: contains `deny_unknown_fields` (per-kind response structs must be lenient — FR-021)"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "per-kind provider response modules must NOT use deny_unknown_fields:\n{}",
        violations.join("\n"),
    );
}
