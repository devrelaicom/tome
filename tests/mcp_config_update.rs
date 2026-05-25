//! Tests for `mcp_config::write_entry` updating an existing Tome-owned
//! entry — verifies `env` is preserved on rewrite (FR-503) and that
//! identical command+args do not advance mtime (FR-525).

use std::thread::sleep;
use std::time::Duration;

use tempfile::TempDir;
use tome::harness::McpConfigFormat;
use tome::harness::mcp_config::{TomeEntry, read_entry, write_entry};

const MTIME_TICK: Duration = Duration::from_millis(1500);

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

#[test]
fn rebind_rewrites_args_preserving_env_json() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    // Seed a Tome-owned entry with developer-added env.
    let seed = r#"{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "alpha"],
      "env": {
        "MY_FEATURE_FLAG": "1"
      }
    }
  }
}
"#;
    std::fs::write(&target, seed).unwrap();

    // Re-bind to `beta`. Caller passes env=None; on-disk env must
    // survive (FR-503).
    write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry("beta")).unwrap();

    let read_back = read_entry(&target, McpConfigFormat::Json, "mcpServers")
        .unwrap()
        .expect("entry must exist after rewrite");
    assert_eq!(read_back.args, vec!["mcp", "--workspace", "beta"]);
    assert_eq!(
        read_back.env.as_deref(),
        Some(&[("MY_FEATURE_FLAG".to_string(), "1".to_string())][..]),
        "developer-added env must be preserved on rewrite (FR-503)"
    );
}

#[test]
fn rebind_rewrites_args_preserving_env_toml() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("mcp.toml");
    let seed = r#"[mcp_servers.tome]
command = "tome"
args = ["mcp", "--workspace", "alpha"]

[mcp_servers.tome.env]
MY_FEATURE_FLAG = "1"
"#;
    std::fs::write(&target, seed).unwrap();

    write_entry(
        &target,
        McpConfigFormat::Toml,
        "mcp_servers",
        &entry("beta"),
    )
    .unwrap();

    let read_back = read_entry(&target, McpConfigFormat::Toml, "mcp_servers")
        .unwrap()
        .expect("entry must exist after rewrite");
    assert_eq!(read_back.args, vec!["mcp", "--workspace", "beta"]);
    assert_eq!(
        read_back.env.as_deref(),
        Some(&[("MY_FEATURE_FLAG".to_string(), "1".to_string())][..]),
    );
}

#[test]
fn idempotent_rewrite_no_mtime_advance() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry("demo")).unwrap();

    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);

    write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry("demo")).unwrap();

    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "second write with identical command+args must not touch the file"
    );
}

#[test]
fn idempotence_ignores_env_for_comparison() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");

    // Write entry with no env.
    write_entry(&target, McpConfigFormat::Json, "mcpServers", &entry("demo")).unwrap();
    let mtime_before = std::fs::metadata(&target).unwrap().modified().unwrap();
    sleep(MTIME_TICK);

    // Call again with the same command+args but a caller-supplied env.
    // FR-525 corollary says env is opaque to comparison — so this MUST
    // be a no-op (mtime unchanged).
    let mut with_env = entry("demo");
    with_env.env = Some(vec![("ADDED".to_string(), "yes".to_string())]);
    write_entry(&target, McpConfigFormat::Json, "mcpServers", &with_env).unwrap();

    let mtime_after = std::fs::metadata(&target).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "env differences must not trigger a rewrite (FR-525 corollary)"
    );
}
