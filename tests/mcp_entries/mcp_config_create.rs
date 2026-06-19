//! Tests for `mcp_config::write_entry` creating entries against
//! missing-file, populated-file, and commented-TOML inputs.

use tempfile::TempDir;
use tome::harness::mcp_config::{TomeEntry, write_entry};
use tome::harness::{McpConfigFormat, McpDialect};

fn demo_entry() -> TomeEntry {
    TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            "demo".to_string(),
        ],
    )
}

#[test]
fn creates_json_scaffold_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".claude").join("settings.json");
    assert!(!target.exists());

    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
        &demo_entry(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let tome_entry = &parsed["mcpServers"]["tome"];
    assert_eq!(tome_entry["command"], "tome");
    assert_eq!(
        tome_entry["args"],
        serde_json::json!(["mcp", "--workspace", "demo"])
    );
    assert!(body.ends_with('\n'));
}

#[test]
fn creates_toml_scaffold_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".codex").join("mcp.toml");
    assert!(!target.exists());

    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Toml, "mcp_servers"),
        &demo_entry(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    // The standard table header should appear.
    assert!(
        body.contains("[mcp_servers.tome]"),
        "expected [mcp_servers.tome] header, got:\n{body}"
    );
    assert!(body.contains("command = \"tome\""));
    assert!(body.contains("args = [\"mcp\", \"--workspace\", \"demo\"]"));
}

#[test]
fn inserts_tome_entry_into_existing_json_with_other_entries() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("settings.json");
    let existing = r#"{
  "mcpServers": {
    "other": {
      "command": "o",
      "args": []
    }
  }
}
"#;
    std::fs::write(&target, existing).unwrap();

    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
        &demo_entry(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let servers = parsed["mcpServers"].as_object().unwrap();
    assert!(
        servers.contains_key("other"),
        "other entry must be preserved"
    );
    assert!(servers.contains_key("tome"), "tome entry must be inserted");

    // Verify insertion order: `other` first, then `tome` (preserve_order
    // keeps insertion order, not alphabetical).
    let keys: Vec<&String> = servers.keys().collect();
    assert_eq!(keys, vec![&"other".to_string(), &"tome".to_string()]);
}

#[test]
fn inserts_tome_entry_into_existing_toml_with_comments() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("config.toml");
    let existing = r#"# Top-level comment we authored.

# Existing other entry.
[mcp_servers.other]
command = "o"
args = []
"#;
    std::fs::write(&target, existing).unwrap();

    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Toml, "mcp_servers"),
        &demo_entry(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    // Comments preserved verbatim.
    assert!(body.contains("# Top-level comment we authored."));
    assert!(body.contains("# Existing other entry."));
    // Existing entry preserved.
    assert!(body.contains("[mcp_servers.other]"));
    // Tome entry added.
    assert!(body.contains("[mcp_servers.tome]"));
}

#[cfg(unix)]
#[test]
fn parent_dir_is_0700_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join(".claude");
    let target = parent.join("settings.json");
    assert!(!parent.exists());

    write_entry(
        &target,
        &McpDialect::from_format(McpConfigFormat::Json, "mcpServers"),
        &demo_entry(),
    )
    .unwrap();

    let mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "parent dir mode must be 0700, got {mode:o}");
}
