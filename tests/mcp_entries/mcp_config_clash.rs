//! Tests for user-owned vs Tome-owned entries.
//!
//! The contract reserves exit 19 (`HarnessClash`) for the sync
//! orchestrator (US1.b-3). The `mcp_config` primitives surface the
//! parsed entry; the orchestrator inspects `is_tome_owned` to decide
//! between refusing the rewrite and forcing it. These tests pin that
//! primitive-level behaviour.

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::mcp_config::{TomeEntry, is_tome_owned, read_entry, write_entry};
use tome::harness::{McpConfigFormat, McpDialect};

const MTIME_TICK: Duration = Duration::from_millis(1500);

#[test]
fn user_owned_entry_with_different_command_returns_value_not_error() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    // User has hand-authored a "tome" entry pointing at a different
    // binary. Tome must surface this faithfully via `read_entry` so the
    // sync orchestrator can raise HarnessClash (exit 19, US1.b-3 wiring).
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "other-binary",
      "args": ["mcp"]
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    let parsed = read_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap()
    .expect("entry must be returned");
    assert_eq!(parsed.command, "other-binary");
    assert!(
        !is_tome_owned(&parsed),
        "command != \"tome\" must read as user-owned"
    );
}

#[test]
fn user_owned_entry_with_different_first_arg_returns_value_not_error() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["other-cmd"]
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    let parsed = read_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
    )
    .unwrap()
    .expect("entry must be returned");
    assert_eq!(parsed.args, vec!["other-cmd"]);
    assert!(
        !is_tome_owned(&parsed),
        "args[0] != \"mcp\" must read as user-owned"
    );
}

#[test]
fn tome_owned_entry_is_idempotent_overwrite() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "demo"]
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);

    let same = TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            "demo".to_string(),
        ],
    );
    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
        &same,
    )
    .unwrap();

    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "rewrite of identical Tome-owned entry must be a no-op"
    );
}
