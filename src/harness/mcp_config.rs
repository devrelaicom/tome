//! Strict-vs-lenient boundary for harness MCP configuration files.
//!
//! Tome treats harness MCP configs as **third-party data** with two
//! consequences:
//!
//! 1. **Lenient parse**: unknown fields are preserved on round-trip, not
//!    rejected. `serde_json` (with the project-wide `preserve_order`
//!    feature) for JSON; `toml_edit` (comment- and order-preserving)
//!    for TOML. Tome-owned manifests (`config.toml`, `settings.toml`,
//!    `manifest.json`) use the strict `#[serde(deny_unknown_fields)]`
//!    boundary instead.
//!
//! 2. **Read-modify-write**: only Tome-owned entries (under the `"tome"`
//!    key, matching the ownership marker `command == "tome" && args[0]
//!    == "mcp"`) are mutated. Every other key, value, comment, and
//!    ordering decision in the file is preserved verbatim.
//!
//! F5 audit (PR #63) confirmed `serde_json/preserve_order` is
//! behaviourally neutral on the rest of Tome's `serde_json` usage;
//! `toml_edit` is unused outside this module. See
//! `specs/004-phase-4-refactor-harnesses/retro/P2.md`
//! § Workarounds & Solutions for the audit details.
//!
//! ## Ownership marker (FR-501)
//!
//! An existing entry under key `"tome"` is **Tome-owned** if and only
//! if:
//!
//! - `command == "tome"`, AND
//! - `args[0] == "mcp"`.
//!
//! Any other content under the `"tome"` key is **user-owned** and
//! refuses rewrite without `--force` (exit 19 / `HarnessClash`). The
//! `env` field is preserved on rewrite (FR-503) and is NOT consulted by
//! the ownership marker.
//!
//! ## Atomic-write discipline (FR-349)
//!
//! Every read-modify-write follows: read → parse with the
//! order-preserving library → modify the `mcpServers.tome` (or
//! `mcp_servers.tome`) node → serialise → write to a sibling temp file
//! on the same filesystem → fsync → atomic rename.

use std::path::Path;

use crate::error::TomeError;
use crate::harness::McpConfigFormat;

/// Parsed view of the existing Tome-owned entry in a harness MCP
/// config. US4 will fill out the fields (`command`, `args`, optional
/// `env`). F7 only sketches the shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomeEntry {
    pub command: String,
    pub args: Vec<String>,
    /// Developer-added env vars. Preserved on rewrite per FR-503; never
    /// consulted by the ownership marker.
    pub env: Option<Vec<(String, String)>>,
}

/// Read the existing entry under `mcpServers.tome` (or
/// `mcp_servers.tome`) from the harness MCP config at `path`.
///
/// Returns `Ok(None)` when the file or the entry is absent. Returns the
/// parsed entry otherwise. Lenient parse — unknown sibling keys are
/// preserved through the underlying document model (`serde_json` with
/// `preserve_order`, or `toml_edit`).
#[allow(unused_variables)]
pub fn read_entry(
    _path: &Path,
    _format: McpConfigFormat,
    _parent_key: &str,
) -> Result<Option<TomeEntry>, TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Write the Tome-owned entry at `mcpServers.tome` (or
/// `mcp_servers.tome`) in the harness MCP config at `path`.
///
/// Preserves every other key, value, comment, and ordering decision in
/// the file. Preserves the existing entry's `env` field on rewrite per
/// FR-503. Creates parent directory (mode 0700 on Unix) and the file
/// itself if missing. Atomic rename onto `path`.
#[allow(unused_variables)]
pub fn write_entry(
    _path: &Path,
    _format: McpConfigFormat,
    _parent_key: &str,
    _entry: &TomeEntry,
) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Remove the Tome-owned entry from the harness MCP config at `path`.
///
/// Leaves the file alone if the entry is absent, user-owned, or the
/// file itself is missing. After removal the parent object (`mcpServers`
/// / `mcp_servers`) is left in place even if empty — other entries are
/// unaffected.
#[allow(unused_variables)]
pub fn remove_entry(
    _path: &Path,
    _format: McpConfigFormat,
    _parent_key: &str,
) -> Result<(), TomeError> {
    unimplemented!("F7 skeleton; production wiring lands in US3.c / US4")
}

/// Predicate matching the ownership marker (FR-501).
///
/// An entry is Tome-owned iff `command == "tome"` and `args[0] ==
/// "mcp"`. The `env` field is ignored. Returns `false` for any
/// shape-mismatch (e.g. missing `args`, `args[0] != "mcp"`).
pub fn is_tome_owned(entry: &TomeEntry) -> bool {
    entry.command == "tome" && entry.args.first().map(String::as_str) == Some("mcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(command: &str, args: &[&str]) -> TomeEntry {
        TomeEntry {
            command: command.to_string(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            env: None,
        }
    }

    #[test]
    fn is_tome_owned_matches_canonical_shape() {
        let e = entry("tome", &["mcp", "--workspace", "demo"]);
        assert!(is_tome_owned(&e));
    }

    #[test]
    fn is_tome_owned_rejects_wrong_command() {
        let e = entry("other-binary", &["mcp"]);
        assert!(!is_tome_owned(&e));
    }

    #[test]
    fn is_tome_owned_rejects_wrong_first_arg() {
        let e = entry("tome", &["serve"]);
        assert!(!is_tome_owned(&e));
    }

    #[test]
    fn is_tome_owned_rejects_empty_args() {
        let e = entry("tome", &[]);
        assert!(!is_tome_owned(&e));
    }

    #[test]
    fn is_tome_owned_ignores_env() {
        let mut e = entry("tome", &["mcp"]);
        e.env = Some(vec![("MY_FLAG".to_string(), "1".to_string())]);
        assert!(is_tome_owned(&e));
    }
}
