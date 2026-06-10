//! Tests that key/table order in the MCP config file is preserved
//! across `write_entry` calls — required by FR-349 read-modify-write
//! and FR-525 idempotence guarantees.

use tempfile::TempDir;
use tome::harness::McpConfigFormat;
use tome::harness::mcp_config::{TomeEntry, write_entry};

fn entry(workspace: &str) -> TomeEntry {
    TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            workspace.to_string(),
        ],
    )
}

/// Helper: get the key indices of `targets` in the order they appear in
/// `haystack`. Panics on any missing key.
fn key_positions(haystack: &str, targets: &[&str]) -> Vec<usize> {
    targets
        .iter()
        .map(|t| {
            haystack
                .find(t)
                .unwrap_or_else(|| panic!("key {t:?} not found in:\n{haystack}"))
        })
        .collect()
}

#[test]
fn inserts_tome_in_middle_position_preserves_existing_order_json() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    // Three existing entries in alphabetical order. After insertion,
    // `tome` is alphabetically between `bbb` and `ccc`, but
    // `preserve_order` (an IndexMap-backed JSON map) keeps insertion
    // order — so tome lands at the end.
    let seed = r#"{
  "mcpServers": {
    "aaa": { "command": "a", "args": [] },
    "bbb": { "command": "b", "args": [] },
    "ccc": { "command": "c", "args": [] }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry("demo")).unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    let positions = key_positions(&body, &["\"aaa\"", "\"bbb\"", "\"ccc\"", "\"tome\""]);
    let sorted = {
        let mut v = positions.clone();
        v.sort();
        v
    };
    assert_eq!(
        positions, sorted,
        "expected insertion order aaa, bbb, ccc, tome — got positions {positions:?}"
    );
}

#[test]
fn rewrites_tome_preserves_surrounding_entry_order_json() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "aaa": { "command": "a", "args": [] },
    "bbb": { "command": "b", "args": [] },
    "ccc": { "command": "c", "args": [] }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    write_entry(
        &target,
        McpConfigFormat::Json,
        "mcpServers",
        &entry("first"),
    )
    .unwrap();
    write_entry(
        &target,
        McpConfigFormat::Json,
        "mcpServers",
        &entry("second"),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    let positions = key_positions(&body, &["\"aaa\"", "\"bbb\"", "\"ccc\"", "\"tome\""]);
    let sorted = {
        let mut v = positions.clone();
        v.sort();
        v
    };
    assert_eq!(
        positions, sorted,
        "rewrite must preserve surrounding entry order"
    );
    // Confirm the rewrite landed.
    assert!(
        body.contains("\"second\""),
        "rewrite must have updated args"
    );
}

#[test]
fn inserts_tome_preserves_existing_order_toml() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("mcp.toml");
    let seed = r#"[mcp_servers.aaa]
command = "a"
args = []

[mcp_servers.bbb]
command = "b"
args = []
"#;
    std::fs::write(&target, seed).unwrap();

    write_entry(
        &target,
        McpConfigFormat::Toml,
        "mcp_servers",
        &entry("demo"),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    let positions = key_positions(
        &body,
        &[
            "[mcp_servers.aaa]",
            "[mcp_servers.bbb]",
            "[mcp_servers.tome]",
        ],
    );
    let sorted = {
        let mut v = positions.clone();
        v.sort();
        v
    };
    assert_eq!(
        positions, sorted,
        "expected appended order aaa, bbb, tome — got positions {positions:?}"
    );
}
