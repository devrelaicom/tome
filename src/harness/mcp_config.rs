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

use std::io::Write;
use std::path::Path;

use serde_json::{Map as JsonMap, Value as JsonValue};
use tempfile::NamedTempFile;
use toml_edit::{
    Array as TomlArray, DocumentMut, Item as TomlItem, Table as TomlTable, Value as TomlValue,
    value as toml_value,
};

use crate::error::TomeError;
use crate::harness::{MCP_CONFIG_KEY, McpConfigFormat};

/// Parsed view of the existing Tome-owned entry in a harness MCP
/// config. `env` is preserved on rewrite per FR-503 and is never
/// consulted by the ownership marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomeEntry {
    pub command: String,
    pub args: Vec<String>,
    /// Developer-added env vars. Preserved on rewrite per FR-503; never
    /// consulted by the ownership marker. `Vec` (not `HashMap`) so
    /// insertion order from the source file round-trips.
    pub env: Option<Vec<(String, String)>>,
}

impl TomeEntry {
    /// Convenience constructor for the common (command, args) case.
    /// Sets `env = None`.
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self {
            command,
            args,
            env: None,
        }
    }
}

/// Refuse to write through a symlink. Returns Ok if the path is absent
/// or a regular file/dir; Err if it's a symlink. Mirrors the
/// `rules_file.rs` discipline (FR-7 carry-over from Phase 3 P8 PR-F).
fn refuse_symlink(target: &Path) -> Result<(), TomeError> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "refusing to read or write through symlink: {}",
                target.display()
            ),
        ))),
        Ok(_) | Err(_) => Ok(()),
    }
}

/// Map an arbitrary error into `TomeError::Io(InvalidData)` with the
/// path included for diagnosis.
fn parse_err(path: &Path, err: impl std::fmt::Display) -> TomeError {
    TomeError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("parse mcp config {}: {err}", path.display()),
    ))
}

/// Atomic write: temp file in same dir → fsync → rename.
fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), TomeError> {
    let parent = target
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("mcp-config path has no parent")))?;
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

    let mut tmp = NamedTempFile::with_prefix_in(".tome.tmp.", parent).map_err(TomeError::Io)?;
    tmp.write_all(bytes).map_err(TomeError::Io)?;
    tmp.as_file().sync_all().map_err(TomeError::Io)?;
    tmp.persist(target).map_err(|e| TomeError::Io(e.error))?;
    Ok(())
}

// =====================================================================
// JSON helpers
// =====================================================================

/// Read a JSON file and parse it as a `serde_json::Value`. Missing file
/// returns `Ok(None)`; parse errors propagate.
fn read_json_doc(path: &Path) -> Result<Option<JsonValue>, TomeError> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TomeError::Io(e)),
    };
    if body.trim().is_empty() {
        // Treat empty file as an empty JSON object.
        return Ok(Some(JsonValue::Object(JsonMap::new())));
    }
    serde_json::from_str::<JsonValue>(&body)
        .map(Some)
        .map_err(|e| parse_err(path, e))
}

/// Parse a JSON value at `parent[MCP_CONFIG_KEY]` into a `TomeEntry`.
fn json_entry_from_value(path: &Path, raw: &JsonValue) -> Result<TomeEntry, TomeError> {
    let obj = raw
        .as_object()
        .ok_or_else(|| parse_err(path, format!("'{MCP_CONFIG_KEY}' entry must be an object")))?;
    let command = obj
        .get("command")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| parse_err(path, "'tome.command' missing or not a string"))?
        .to_string();
    let args = obj
        .get("args")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| parse_err(path, "'tome.args' missing or not an array"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| parse_err(path, "'tome.args' must be an array of strings"))
        })
        .collect::<Result<Vec<String>, _>>()?;
    let env = match obj.get("env") {
        None => None,
        Some(JsonValue::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map.iter() {
                let value = v
                    .as_str()
                    .ok_or_else(|| parse_err(path, format!("'tome.env.{k}' must be a string")))?;
                out.push((k.clone(), value.to_string()));
            }
            Some(out)
        }
        Some(_) => {
            return Err(parse_err(path, "'tome.env' must be an object of strings"));
        }
    };
    Ok(TomeEntry { command, args, env })
}

/// Build a JSON object representing a `TomeEntry`, preserving the
/// `command → args → env` key order (insertion order with
/// `preserve_order`).
fn json_entry_object(entry: &TomeEntry) -> JsonValue {
    let mut obj = JsonMap::new();
    obj.insert(
        "command".to_string(),
        JsonValue::String(entry.command.clone()),
    );
    obj.insert(
        "args".to_string(),
        JsonValue::Array(entry.args.iter().cloned().map(JsonValue::String).collect()),
    );
    if let Some(env) = &entry.env {
        let mut env_obj = JsonMap::new();
        for (k, v) in env {
            env_obj.insert(k.clone(), JsonValue::String(v.clone()));
        }
        obj.insert("env".to_string(), JsonValue::Object(env_obj));
    }
    JsonValue::Object(obj)
}

// =====================================================================
// TOML helpers
// =====================================================================

/// Read a TOML file and parse it as a `DocumentMut`. Missing file
/// returns `Ok(None)`; parse errors propagate.
fn read_toml_doc(path: &Path) -> Result<Option<DocumentMut>, TomeError> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TomeError::Io(e)),
    };
    body.parse::<DocumentMut>()
        .map(Some)
        .map_err(|e| parse_err(path, e))
}

/// Extract a `TomeEntry` from a `TableLike` view at
/// `parent[MCP_CONFIG_KEY]`.
fn toml_entry_from_table(
    path: &Path,
    entry: &dyn toml_edit::TableLike,
) -> Result<TomeEntry, TomeError> {
    let command = entry
        .get("command")
        .and_then(TomlItem::as_str)
        .ok_or_else(|| parse_err(path, "'tome.command' missing or not a string"))?
        .to_string();
    let args_item = entry
        .get("args")
        .ok_or_else(|| parse_err(path, "'tome.args' missing"))?;
    let args = args_item
        .as_array()
        .ok_or_else(|| parse_err(path, "'tome.args' must be an array"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| parse_err(path, "'tome.args' must be an array of strings"))
        })
        .collect::<Result<Vec<String>, _>>()?;
    let env = match entry.get("env") {
        None => None,
        Some(env_item) => {
            let env_table = env_item
                .as_table_like()
                .ok_or_else(|| parse_err(path, "'tome.env' must be a table of strings"))?;
            let mut out = Vec::new();
            for (k, v) in env_table.iter() {
                let s = v
                    .as_str()
                    .ok_or_else(|| parse_err(path, format!("'tome.env.{k}' must be a string")))?;
                out.push((k.to_string(), s.to_string()));
            }
            Some(out)
        }
    };
    Ok(TomeEntry { command, args, env })
}

/// Construct a fresh standard `[parent.tome]` table populated from
/// `entry`. The `env` table is added (and standalone) only when
/// `entry.env` is `Some`.
fn toml_new_entry_table(entry: &TomeEntry) -> TomlTable {
    let mut table = TomlTable::new();
    table.insert("command", toml_value(entry.command.as_str()));
    let mut args = TomlArray::new();
    for a in &entry.args {
        args.push(a.as_str());
    }
    table.insert("args", toml_value(args));
    if let Some(env) = &entry.env {
        let mut env_table = TomlTable::new();
        env_table.set_implicit(false);
        for (k, v) in env {
            env_table.insert(k, toml_value(v.as_str()));
        }
        table.insert("env", TomlItem::Table(env_table));
    }
    table
}

// =====================================================================
// Public API
// =====================================================================

/// Read the existing entry under `mcpServers.tome` (or
/// `mcp_servers.tome`) from the harness MCP config at `path`.
///
/// Returns `Ok(None)` when the file or the entry is absent. Returns the
/// parsed entry otherwise. Lenient parse — unknown sibling keys are
/// preserved through the underlying document model (`serde_json` with
/// `preserve_order`, or `toml_edit`).
pub fn read_entry(
    path: &Path,
    format: McpConfigFormat,
    parent_key: &str,
) -> Result<Option<TomeEntry>, TomeError> {
    refuse_symlink(path)?;
    match format {
        McpConfigFormat::Json => {
            let Some(doc) = read_json_doc(path)? else {
                return Ok(None);
            };
            let Some(parent) = doc.get(parent_key).and_then(JsonValue::as_object) else {
                return Ok(None);
            };
            let Some(raw) = parent.get(MCP_CONFIG_KEY) else {
                return Ok(None);
            };
            Ok(Some(json_entry_from_value(path, raw)?))
        }
        McpConfigFormat::Toml => {
            let Some(doc) = read_toml_doc(path)? else {
                return Ok(None);
            };
            let Some(parent) = doc.get(parent_key).and_then(TomlItem::as_table_like) else {
                return Ok(None);
            };
            let Some(entry_item) = parent.get(MCP_CONFIG_KEY) else {
                return Ok(None);
            };
            let entry_table = entry_item
                .as_table_like()
                .ok_or_else(|| parse_err(path, format!("'{MCP_CONFIG_KEY}' must be a table")))?;
            Ok(Some(toml_entry_from_table(path, entry_table)?))
        }
    }
}

/// Write the Tome-owned entry at `mcpServers.tome` (or
/// `mcp_servers.tome`) in the harness MCP config at `path`.
///
/// Preserves every other key, value, comment, and ordering decision in
/// the file. Preserves the existing entry's `env` field on rewrite per
/// FR-503. Creates parent directory (mode 0700 on Unix) and the file
/// itself if missing. Atomic rename onto `path`.
///
/// **Idempotence (FR-525 corollary)**: when the on-disk Tome-owned
/// entry already has the same `command` + `args` as `entry`, no write
/// is performed. `env` is treated as opaque in the comparison.
///
/// **Note on user-owned clashes**: this primitive does NOT raise
/// `HarnessClash` (exit 19). Callers (the sync orchestrator in US1.b-3)
/// must call `read_entry` first, inspect the result with
/// `is_tome_owned`, and decide between exit-19 vs `--force` rewrite.
/// `write_entry` itself always rewrites whatever's there — when
/// invoked, the caller has already decided overwriting is safe.
pub fn write_entry(
    path: &Path,
    format: McpConfigFormat,
    parent_key: &str,
    entry: &TomeEntry,
) -> Result<(), TomeError> {
    refuse_symlink(path)?;

    // Idempotence pre-check: same command+args means no write.
    if let Some(current) = read_entry(path, format, parent_key)?
        && super::mcp_config::is_tome_owned(&current)
        && current.command == entry.command
        && current.args == entry.args
    {
        return Ok(());
    }

    match format {
        McpConfigFormat::Json => write_entry_json(path, parent_key, entry),
        McpConfigFormat::Toml => write_entry_toml(path, parent_key, entry),
    }
}

fn write_entry_json(path: &Path, parent_key: &str, entry: &TomeEntry) -> Result<(), TomeError> {
    // Load the existing document or start with an empty object.
    let mut doc = read_json_doc(path)?.unwrap_or_else(|| JsonValue::Object(JsonMap::new()));

    let doc_obj = match doc.as_object_mut() {
        Some(o) => o,
        None => {
            return Err(parse_err(path, "top-level JSON value must be an object"));
        }
    };

    // Ensure the parent object exists.
    if !doc_obj
        .get(parent_key)
        .map(JsonValue::is_object)
        .unwrap_or(false)
    {
        doc_obj.insert(parent_key.to_string(), JsonValue::Object(JsonMap::new()));
    }
    let parent_obj = doc_obj
        .get_mut(parent_key)
        .and_then(JsonValue::as_object_mut)
        .expect("parent_key was just ensured to be an object");

    // Preserve developer-added `env` when the existing entry is
    // Tome-owned (FR-503). Caller-supplied env wins when set; otherwise
    // fall back to the on-disk env.
    let mut new_entry = entry.clone();
    if new_entry.env.is_none()
        && let Some(existing) = parent_obj.get(MCP_CONFIG_KEY)
        && let Ok(parsed) = json_entry_from_value(path, existing)
        && is_tome_owned(&parsed)
    {
        new_entry.env = parsed.env;
    }

    parent_obj.insert(MCP_CONFIG_KEY.to_string(), json_entry_object(&new_entry));

    let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|e| parse_err(path, e))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn write_entry_toml(path: &Path, parent_key: &str, entry: &TomeEntry) -> Result<(), TomeError> {
    let mut doc = read_toml_doc(path)?.unwrap_or_else(DocumentMut::new);

    // Ensure the parent table exists. Use standard (non-implicit) so
    // serialisation produces `[mcp_servers.tome]` style headers.
    {
        let parent_item = doc.entry(parent_key).or_insert_with(|| {
            let mut t = TomlTable::new();
            t.set_implicit(true);
            TomlItem::Table(t)
        });
        if !parent_item.is_table_like() {
            return Err(parse_err(path, format!("'{parent_key}' must be a table")));
        }
    }

    // Compose the new entry value, preserving existing env if not
    // supplied by the caller (FR-503).
    let mut new_entry = entry.clone();
    if new_entry.env.is_none() {
        let parent_view = doc.get(parent_key).and_then(TomlItem::as_table_like);
        if let Some(parent_view) = parent_view
            && let Some(existing_item) = parent_view.get(MCP_CONFIG_KEY)
            && let Some(existing_table) = existing_item.as_table_like()
            && let Ok(parsed) = toml_entry_from_table(path, existing_table)
            && is_tome_owned(&parsed)
        {
            new_entry.env = parsed.env;
        }
    }

    // Now mutate. We choose between inline-table and standard table
    // shapes based on the existing entry (or default to standard when
    // creating).
    let parent_item = doc.get_mut(parent_key).expect("ensured above");
    let parent_table = parent_item.as_table_like_mut().expect("ensured table-like");

    let existing_is_inline = matches!(
        parent_table.get(MCP_CONFIG_KEY),
        Some(TomlItem::Value(TomlValue::InlineTable(_)))
    );

    if existing_is_inline {
        // Build an inline table replacement to keep shape.
        let mut inline = toml_edit::InlineTable::new();
        inline.insert("command", new_entry.command.as_str().into());
        let mut args = TomlArray::new();
        for a in &new_entry.args {
            args.push(a.as_str());
        }
        inline.insert("args", TomlValue::Array(args));
        if let Some(env) = &new_entry.env {
            let mut env_inline = toml_edit::InlineTable::new();
            for (k, v) in env {
                env_inline.insert(k, v.as_str().into());
            }
            inline.insert("env", TomlValue::InlineTable(env_inline));
        }
        parent_table.insert(
            MCP_CONFIG_KEY,
            TomlItem::Value(TomlValue::InlineTable(inline)),
        );
    } else {
        let new_table = toml_new_entry_table(&new_entry);
        parent_table.insert(MCP_CONFIG_KEY, TomlItem::Table(new_table));
    }

    atomic_write(path, doc.to_string().as_bytes())
}

/// Remove the Tome-owned entry from the harness MCP config at `path`.
///
/// Leaves the file alone if the entry is absent, user-owned, or the
/// file itself is missing. After removal the parent object (`mcpServers`
/// / `mcp_servers`) is left in place even if empty — other entries are
/// unaffected.
pub fn remove_entry(
    path: &Path,
    format: McpConfigFormat,
    parent_key: &str,
) -> Result<(), TomeError> {
    refuse_symlink(path)?;

    // Pre-check via `read_entry`: if the entry is absent or user-owned,
    // no write — idempotence preserved (mtime unchanged).
    let current = read_entry(path, format, parent_key)?;
    let Some(current) = current else {
        return Ok(());
    };
    if !is_tome_owned(&current) {
        return Ok(());
    }

    match format {
        McpConfigFormat::Json => {
            let Some(mut doc) = read_json_doc(path)? else {
                return Ok(());
            };
            let Some(doc_obj) = doc.as_object_mut() else {
                return Ok(());
            };
            let Some(parent_obj) = doc_obj
                .get_mut(parent_key)
                .and_then(JsonValue::as_object_mut)
            else {
                return Ok(());
            };
            parent_obj.shift_remove(MCP_CONFIG_KEY);
            let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|e| parse_err(path, e))?;
            bytes.push(b'\n');
            atomic_write(path, &bytes)
        }
        McpConfigFormat::Toml => {
            let Some(mut doc) = read_toml_doc(path)? else {
                return Ok(());
            };
            let Some(parent_item) = doc.get_mut(parent_key) else {
                return Ok(());
            };
            let Some(parent_table) = parent_item.as_table_like_mut() else {
                return Ok(());
            };
            parent_table.remove(MCP_CONFIG_KEY);
            atomic_write(path, doc.to_string().as_bytes())
        }
    }
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

    #[test]
    fn new_constructor_sets_env_to_none() {
        let e = TomeEntry::new("tome".to_string(), vec!["mcp".to_string()]);
        assert!(e.env.is_none());
    }
}
