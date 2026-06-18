//! Tests for `mcp_config::remove_entry` — Tome-owned entries are
//! removed cleanly; user-owned entries are left alone; missing files
//! and missing entries are no-ops.

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::mcp_config::{read_entry, remove_entry};
use tome::harness::{McpConfigFormat, McpDialect};

const MTIME_TICK: Duration = Duration::from_millis(1500);

#[test]
fn removes_tome_owned_entry_from_json() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "demo"]
    },
    "other": {
      "command": "o",
      "args": []
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    remove_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    let servers = parsed["mcpServers"].as_object().unwrap();
    assert!(!servers.contains_key("tome"), "tome must be removed");
    assert!(servers.contains_key("other"), "other must be preserved");
}

#[test]
fn removes_tome_owned_entry_from_toml() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("mcp.toml");
    let seed = r#"[mcp_servers.tome]
command = "tome"
args = ["mcp", "--workspace", "demo"]

[mcp_servers.other]
command = "o"
args = []
"#;
    std::fs::write(&target, seed).unwrap();

    remove_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Toml, "mcp_servers"),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert!(
        !body.contains("[mcp_servers.tome]"),
        "tome table must be removed, got:\n{body}"
    );
    assert!(
        body.contains("[mcp_servers.other]"),
        "other table must be preserved"
    );

    // And the parsed entry round-trips to absent.
    let read_back = read_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Toml, "mcp_servers"),
    )
    .unwrap();
    assert!(
        read_back.is_none(),
        "tome entry must be absent after remove"
    );
}

#[test]
fn leaves_user_owned_tome_entry_alone() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "other",
      "args": ["foo"]
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();
    let before = std::fs::read(&target).unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);

    remove_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap();

    let after = std::fs::read(&target).unwrap();
    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        before, after,
        "file bytes must be unchanged when entry is user-owned"
    );
    assert_eq!(
        mtime_before, mtime_after,
        "user-owned entry must not advance mtime"
    );
}

#[test]
fn noop_on_missing_file() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("does-not-exist.json");
    assert!(!target.exists());

    remove_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap();

    assert!(
        !target.exists(),
        "remove on missing file must not create it"
    );
}

#[test]
fn noop_on_file_with_no_tome_entry() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "other": {
      "command": "o",
      "args": []
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);

    remove_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap();

    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "file with no tome entry must be left untouched"
    );
}
