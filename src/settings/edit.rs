//! Order- and comment-preserving editor for the `harnesses` array in
//! Tome-owned settings files.
//!
//! Used by `tome harness use` / `tome harness remove` to append or
//! drop a single harness name from the `harnesses = [...]` array in
//! one of the three settings layers without disturbing other keys,
//! comments, or formatting.
//!
//! The discipline mirrors the FR-349 / FR-503 surface pattern from
//! `src/harness/mcp_config.rs`: read with `toml_edit::DocumentMut` for
//! surgical surface preservation, mutate just the target node, then
//! route the bytes back through [`crate::catalog::store::write_atomic`]
//! for the atomic-write contract (mode preservation + symlink refusal).
//!
//! ## Empty-array semantics
//!
//! When [`remove_harness`] empties the list, the key stays as
//! `harnesses = []`. Per the contract, an empty declared list is
//! semantically distinct from no declaration at all: the resolver's
//! priority walk stops at the first scope where `harnesses` is `Some(_)`
//! regardless of whether the list is empty (FR-441). Leaving the key
//! preserves the "opt out of all harnesses" intent.
//!
//! Removing the key entirely would silently re-enable the next scope's
//! list — exactly the inversion the developer didn't ask for.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::catalog::store::write_atomic;
use crate::error::TomeError;

/// Open `path` as a `toml_edit::DocumentMut`. Missing file → empty
/// document. Parse errors surface as `TomeError::Io(InvalidData)` with
/// the path included.
pub fn open_settings(path: &Path) -> Result<DocumentMut, TomeError> {
    let body = match crate::util::bounded_read_to_string(path, crate::util::TOME_CONFIG_MAX) {
        Ok(s) => s,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DocumentMut::new());
        }
        Err(e) => return Err(e),
    };
    body.parse::<DocumentMut>().map_err(|e| {
        TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("parse settings file {}: {e}", path.display()),
        ))
    })
}

/// Append `harness_name` to the `harnesses` array in `doc`.
///
/// Creates the array (as an inline array, matching the project marker
/// plus workspace settings convention) if absent. Returns `true` iff
/// the document was modified (name wasn't already present).
pub fn add_harness(doc: &mut DocumentMut, harness_name: &str) -> bool {
    let entry = doc.entry("harnesses").or_insert_with(|| {
        let arr = Array::new();
        Item::Value(Value::Array(arr))
    });

    // If the existing item is not an array, replace it with a fresh
    // inline array carrying just our new name. (This case is unreachable
    // for well-formed settings; we tolerate it rather than erroring so
    // the user can recover by rewriting the file.)
    let Some(array) = entry.as_array_mut() else {
        let mut arr = Array::new();
        arr.push(harness_name);
        *entry = Item::Value(Value::Array(arr));
        return true;
    };

    if array_contains(array, harness_name) {
        return false;
    }
    array.push(harness_name);
    true
}

/// Remove `harness_name` from the `harnesses` array in `doc`.
///
/// Returns `true` iff the document was modified. If the array
/// becomes empty, it is left in place as `harnesses = []` — see the
/// module-level doc for the rationale.
///
/// If the key is absent or the name isn't present, the function is
/// a no-op (returns `false`).
pub fn remove_harness(doc: &mut DocumentMut, harness_name: &str) -> bool {
    let Some(item) = doc.get_mut("harnesses") else {
        return false;
    };
    let Some(array) = item.as_array_mut() else {
        return false;
    };
    let original_len = array.len();
    array.retain(|v| v.as_str().map(|s| s != harness_name).unwrap_or(true));
    array.len() != original_len
}

/// Append `harness_name` to the `[harness].enabled` array in a unified
/// config doc (`config.toml`). Used when writing at global scope.
///
/// Creates the `[harness]` table and the `enabled` inline array when absent.
/// Returns `true` iff the document was modified.
///
/// Returns `false` without modifying `doc` if `[harness]` exists but is not
/// a table (e.g. `harness = "a string"` after a hand-edit). The caller can
/// treat the no-op as "not modified" — the file stays unchanged and the user
/// can correct the hand-edited value themselves.
pub fn add_harness_to_config(doc: &mut DocumentMut, harness_name: &str) -> bool {
    let harness_tbl = {
        let item = doc
            .entry("harness")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        let Some(tbl) = item.as_table_mut() else {
            // Defensive: user hand-edited `harness` to a non-table value.
            // Return false (no change) so the process does not abort.
            return false;
        };
        tbl
    };

    let entry = harness_tbl.entry("enabled").or_insert_with(|| {
        let arr = Array::new();
        Item::Value(Value::Array(arr))
    });

    let Some(array) = entry.as_array_mut() else {
        let mut arr = Array::new();
        arr.push(harness_name);
        *entry = Item::Value(Value::Array(arr));
        return true;
    };

    if array_contains(array, harness_name) {
        return false;
    }
    array.push(harness_name);
    true
}

/// Remove `harness_name` from the `[harness].enabled` array in a unified
/// config doc. Returns `true` iff the document was modified.
pub fn remove_harness_from_config(doc: &mut DocumentMut, harness_name: &str) -> bool {
    let Some(harness_item) = doc.get_mut("harness") else {
        return false;
    };
    let Some(harness_tbl) = harness_item.as_table_mut() else {
        return false;
    };
    let Some(item) = harness_tbl.get_mut("enabled") else {
        return false;
    };
    let Some(array) = item.as_array_mut() else {
        return false;
    };
    let original_len = array.len();
    array.retain(|v| v.as_str().map(|s| s != harness_name).unwrap_or(true));
    array.len() != original_len
}

fn array_contains(array: &Array, needle: &str) -> bool {
    array
        .iter()
        .any(|v| v.as_str().map(|s| s == needle).unwrap_or(false))
}

/// Serialise `doc` and atomic-write to `path`. Routes through
/// [`crate::catalog::store::write_atomic`] for mode preservation +
/// symlink refusal.
pub fn save_settings(path: &Path, doc: &DocumentMut) -> Result<(), TomeError> {
    write_atomic(path, doc.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_to_empty_document_creates_array() {
        let mut doc = DocumentMut::new();
        assert!(add_harness(&mut doc, "claude-code"));
        let s = doc.to_string();
        assert!(s.contains("harnesses"));
        assert!(s.contains("claude-code"));
    }

    #[test]
    fn add_to_existing_array_appends() {
        let mut doc: DocumentMut = "harnesses = [\"codex\"]\n".parse().unwrap();
        assert!(add_harness(&mut doc, "claude-code"));
        let s = doc.to_string();
        assert!(s.contains("codex"));
        assert!(s.contains("claude-code"));
    }

    #[test]
    fn add_already_present_is_noop() {
        let mut doc: DocumentMut = "harnesses = [\"codex\"]\n".parse().unwrap();
        assert!(!add_harness(&mut doc, "codex"));
    }

    #[test]
    fn remove_existing_entry_drops_it() {
        let mut doc: DocumentMut = "harnesses = [\"codex\", \"gemini\"]\n".parse().unwrap();
        assert!(remove_harness(&mut doc, "codex"));
        let s = doc.to_string();
        assert!(!s.contains("codex"));
        assert!(s.contains("gemini"));
    }

    #[test]
    fn remove_absent_entry_is_noop() {
        let mut doc: DocumentMut = "harnesses = [\"codex\"]\n".parse().unwrap();
        assert!(!remove_harness(&mut doc, "gemini"));
    }

    #[test]
    fn remove_last_leaves_empty_array() {
        let mut doc: DocumentMut = "harnesses = [\"codex\"]\n".parse().unwrap();
        assert!(remove_harness(&mut doc, "codex"));
        let s = doc.to_string();
        // Empty array key MUST remain — see module-level doc.
        assert!(s.contains("harnesses"));
        assert!(s.contains("[]") || s.contains("[ ]"));
    }

    #[test]
    fn add_preserves_other_top_level_keys() {
        let src = "name = \"demo\"\nharnesses = [\"codex\"]\n";
        let mut doc: DocumentMut = src.parse().unwrap();
        assert!(add_harness(&mut doc, "claude-code"));
        let s = doc.to_string();
        assert!(s.contains("name = \"demo\""));
        assert!(s.contains("claude-code"));
    }

    // ---------------------------------------------------------------------------
    // Tests for config-doc helpers (`add_harness_to_config` / `remove_harness_from_config`)
    // ---------------------------------------------------------------------------

    #[test]
    fn add_to_config_creates_harness_table_and_enabled_array() {
        // From an empty doc, the function must create `[harness]` + `enabled`.
        let mut doc = DocumentMut::new();
        assert!(
            add_harness_to_config(&mut doc, "claude-code"),
            "empty doc → modified"
        );
        let s = doc.to_string();
        assert!(s.contains("harness"), "section header must be written");
        assert!(s.contains("claude-code"), "harness name must appear");
    }

    #[test]
    fn add_to_config_idempotent_second_add_is_noop() {
        // Adding the same name twice must return false on the second call.
        let mut doc = DocumentMut::new();
        assert!(
            add_harness_to_config(&mut doc, "codex"),
            "first add modified"
        );
        assert!(
            !add_harness_to_config(&mut doc, "codex"),
            "second add must be no-op"
        );
        // The name must appear exactly once.
        let s = doc.to_string();
        assert_eq!(
            s.matches("codex").count(),
            1,
            "name must appear exactly once: {s}"
        );
    }

    #[test]
    fn add_to_config_returns_false_when_harness_is_non_table() {
        // A hand-edited `harness = "a string"` must NOT panic — returns false.
        let mut doc: DocumentMut = "harness = \"oops\"\n".parse().unwrap();
        assert!(
            !add_harness_to_config(&mut doc, "stub"),
            "non-table [harness] must return false, not panic"
        );
        // The doc must remain unchanged.
        let s = doc.to_string();
        assert!(s.contains("\"oops\""), "original value must be preserved");
    }

    #[test]
    fn remove_from_config_absent_entry_is_noop() {
        // Removing a name that isn't present must return false.
        let mut doc: DocumentMut = "[harness]\nenabled = [\"codex\"]\n".parse().unwrap();
        assert!(
            !remove_harness_from_config(&mut doc, "gemini"),
            "absent entry must be no-op"
        );
    }

    #[test]
    fn remove_from_config_last_entry_leaves_empty_array() {
        // Removing the only entry must leave `enabled = []` (not delete the key).
        let mut doc: DocumentMut = "[harness]\nenabled = [\"codex\"]\n".parse().unwrap();
        assert!(
            remove_harness_from_config(&mut doc, "codex"),
            "present entry must be removed"
        );
        let s = doc.to_string();
        assert!(!s.contains("codex"), "removed name must be gone");
        // The `enabled` key must still be present (semantics: opt-out-of-all).
        assert!(
            s.contains("enabled"),
            "enabled key must remain as empty array: {s}"
        );
    }

    #[test]
    fn add_remove_config_round_trips_via_toml_parse() {
        // After add + save (simulated via to_string), config::HarnessConfig
        // must parse back the name in enabled.
        let mut doc = DocumentMut::new();
        add_harness_to_config(&mut doc, "stub");
        let toml_str = doc.to_string();
        // Parse as the [harness] section by wrapping in [harness] — the doc
        // already has a [harness] table, so parse the full config form.
        let cfg: crate::config::Config = toml::from_str(&toml_str).expect("must round-trip");
        assert_eq!(
            cfg.harness.enabled.as_deref(),
            Some(&["stub".to_string()][..]),
            "round-trip must preserve the enabled list"
        );
    }
}
