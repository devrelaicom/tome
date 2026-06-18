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
//!    == "mcp"`) are mutated. Preservation of the surrounding file
//!    differs by format:
//!
//!    - **TOML** (`toml_edit`): every other key, value, comment, blank
//!      line, and ordering decision is preserved verbatim.
//!    - **JSON** (`serde_json` with `preserve_order`): key order and
//!      unknown keys are preserved; whitespace and indentation are
//!      normalised by `serde_json`'s pretty-printer (no comment support
//!      in standard JSON, so there is nothing else to preserve).
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
//! order-preserving library → modify the `<parent_key>.tome` node →
//! serialise → write to a sibling temp file on the same filesystem →
//! fsync → atomic rename.
//!
//! ## Dialect-driven wire shapes (Phase 11, G1, FR-008)
//!
//! Every public fn takes a [`McpDialect`] (defined in
//! [`crate::harness`]) rather than the Phase ≤10 `(McpConfigFormat,
//! parent_key)` scalar pair. The dialect carries the file format, the
//! parent key, the entry-body template ([`EntryShape`]), the optional
//! `type` discriminator, the empty-`env` policy, and any always-present
//! mandated fields. `TomeEntry { command, args, env }` stays the uniform
//! in-memory model: on read, a `CommandArray` shape's single `command`
//! array is normalised back to `command = arr[0]`, `args = arr[1..]`, so
//! the ownership predicate ([`is_tome_owned`]) is shape-agnostic. On a
//! rewrite the mandated `type`/`extra_fields` are always re-derived from
//! the dialect (a stale/edited mandated value self-heals); the developer
//! `env` is preserved (FR-503).

use std::io::Write;
use std::path::Path;

use serde_json::{Map as JsonMap, Value as JsonValue};
use tempfile::NamedTempFile;
use toml_edit::{
    Array as TomlArray, DocumentMut, Item as TomlItem, Table as TomlTable, Value as TomlValue,
    value as toml_value,
};

use crate::error::TomeError;
use crate::harness::{EntryShape, ExtraValue, FileFormat, MCP_CONFIG_KEY, McpDialect};

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

/// Refuse to read or write through a symlinked component. Returns Ok if no
/// existing component (below the trusted anchor) is a symlink — including an
/// absent path; Err if a symlinked component is found.
///
/// Delegates to the SSOT guard (`util::symlink_safe`) so the MCP-config sink
/// gets the intermediate-component hardening (FR-007), not just the final-node
/// check it had before. A refusal maps to `TomeError::Io` (exit 7) — the
/// dedicated code this sink already used (code 7 covers IO that is not one of
/// the dedicated Phase 6 sinks).
fn refuse_symlink(target: &Path) -> Result<(), TomeError> {
    crate::util::refuse_symlinked_component(target).map_err(TomeError::Io)
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
///
/// On Unix, when `target` already exists, captures its file mode and
/// chmods the staging tempfile to match before persisting. Preserves
/// developer-set mode bits across the rewrite. If `target` is absent,
/// the tempfile's libc-default mode (typically 0o600) wins.
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

    #[cfg(unix)]
    let target_mode: Option<u32> = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(target)
            .ok()
            .map(|m| m.permissions().mode())
    };

    let mut tmp = NamedTempFile::with_prefix_in(".tome.tmp.", parent).map_err(TomeError::Io)?;
    tmp.write_all(bytes).map_err(TomeError::Io)?;
    tmp.as_file().sync_all().map_err(TomeError::Io)?;

    #[cfg(unix)]
    if let Some(mode) = target_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(mode))
            .map_err(TomeError::Io)?;
    }

    tmp.persist(target).map_err(|e| TomeError::Io(e.error))?;
    Ok(())
}

// =====================================================================
// JSON helpers
// =====================================================================

/// Read a JSON file and parse it as a `serde_json::Value`. Missing file
/// returns `Ok(None)`; parse errors propagate.
fn read_json_doc(path: &Path) -> Result<Option<JsonValue>, TomeError> {
    let body = match crate::util::bounded_read_to_string(path, crate::util::HARNESS_MCP_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if body.trim().is_empty() {
        // Treat empty file as an empty JSON object.
        return Ok(Some(JsonValue::Object(JsonMap::new())));
    }
    serde_json::from_str::<JsonValue>(&body)
        .map(Some)
        .map_err(|e| parse_err(path, e))
}

/// Parse a JSON value at `parent[MCP_CONFIG_KEY]` into a `TomeEntry`,
/// using the dialect's [`EntryShape`] to extract `(command, args)`.
///
/// - `CommandArgs`: `command` is a string, `args` a string array.
/// - `CommandArray`: `command` is a single array `[launcher, …args]`,
///   normalised back to `command = arr[0]`, `args = arr[1..]`. There is
///   no separate `args` key.
fn json_entry_from_value(
    path: &Path,
    raw: &JsonValue,
    shape: EntryShape,
) -> Result<TomeEntry, TomeError> {
    let obj = raw
        .as_object()
        .ok_or_else(|| parse_err(path, format!("'{MCP_CONFIG_KEY}' entry must be an object")))?;
    let (command, args) = match shape {
        EntryShape::CommandArgs => {
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
            (command, args)
        }
        EntryShape::CommandArray => {
            let arr = obj
                .get("command")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| parse_err(path, "'tome.command' missing or not an array"))?
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        parse_err(path, "'tome.command' must be an array of strings")
                    })
                })
                .collect::<Result<Vec<String>, _>>()?;
            let mut it = arr.into_iter();
            let command = it
                .next()
                .ok_or_else(|| parse_err(path, "'tome.command' array must be non-empty"))?;
            let args = it.collect::<Vec<String>>();
            (command, args)
        }
    };
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

/// Build a JSON object representing a `TomeEntry` under `dialect`.
///
/// Field order (load-bearing for byte-stable pins, insertion order with
/// `preserve_order`): `type` (iff `entry_type` is `Some`) → `command`
/// → `args` (`CommandArgs` only) → `env` (iff developer env present, OR
/// `emit_env` and no env) → each `extra_fields` entry in slice order.
fn json_entry_object(entry: &TomeEntry, dialect: &McpDialect) -> JsonValue {
    let mut obj = JsonMap::new();

    if let Some(ty) = dialect.entry_type {
        obj.insert(
            "type".to_string(),
            JsonValue::String(ty.as_str().to_string()),
        );
    }

    match dialect.entry_shape {
        EntryShape::CommandArgs => {
            obj.insert(
                "command".to_string(),
                JsonValue::String(entry.command.clone()),
            );
            obj.insert(
                "args".to_string(),
                JsonValue::Array(entry.args.iter().cloned().map(JsonValue::String).collect()),
            );
        }
        EntryShape::CommandArray => {
            // `command` carries the launcher AND the args as one array.
            let mut arr = Vec::with_capacity(1 + entry.args.len());
            arr.push(JsonValue::String(entry.command.clone()));
            arr.extend(entry.args.iter().cloned().map(JsonValue::String));
            obj.insert("command".to_string(), JsonValue::Array(arr));
        }
    }

    match &entry.env {
        Some(env) => {
            let mut env_obj = JsonMap::new();
            for (k, v) in env {
                env_obj.insert(k.clone(), JsonValue::String(v.clone()));
            }
            obj.insert("env".to_string(), JsonValue::Object(env_obj));
        }
        None if dialect.emit_env => {
            obj.insert("env".to_string(), JsonValue::Object(JsonMap::new()));
        }
        None => {}
    }

    for field in dialect.extra_fields {
        obj.insert(field.key.to_string(), extra_value_to_json(field.value));
    }

    JsonValue::Object(obj)
}

/// Render an [`ExtraValue`] into its JSON representation.
fn extra_value_to_json(value: ExtraValue) -> JsonValue {
    match value {
        ExtraValue::Bool(b) => JsonValue::Bool(b),
        ExtraValue::StringArray(items) => JsonValue::Array(
            items
                .iter()
                .map(|s| JsonValue::String((*s).to_string()))
                .collect(),
        ),
    }
}

// =====================================================================
// TOML helpers
// =====================================================================

/// Read a TOML file and parse it as a `DocumentMut`. Missing file
/// returns `Ok(None)`; parse errors propagate.
fn read_toml_doc(path: &Path) -> Result<Option<DocumentMut>, TomeError> {
    let body = match crate::util::bounded_read_to_string(path, crate::util::HARNESS_MCP_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    body.parse::<DocumentMut>()
        .map(Some)
        .map_err(|e| parse_err(path, e))
}

/// Extract a `TomeEntry` from a `TableLike` view at
/// `parent[MCP_CONFIG_KEY]`, using the dialect's [`EntryShape`].
fn toml_entry_from_table(
    path: &Path,
    entry: &dyn toml_edit::TableLike,
    shape: EntryShape,
) -> Result<TomeEntry, TomeError> {
    let (command, args) = match shape {
        EntryShape::CommandArgs => {
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
            (command, args)
        }
        EntryShape::CommandArray => {
            let arr = entry
                .get("command")
                .and_then(TomlItem::as_array)
                .ok_or_else(|| parse_err(path, "'tome.command' missing or not an array"))?
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        parse_err(path, "'tome.command' must be an array of strings")
                    })
                })
                .collect::<Result<Vec<String>, _>>()?;
            let mut it = arr.into_iter();
            let command = it
                .next()
                .ok_or_else(|| parse_err(path, "'tome.command' array must be non-empty"))?;
            (command, it.collect::<Vec<String>>())
        }
    };
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
/// `entry`, in the dialect's field order: `type` → `command` →
/// `args` (`CommandArgs` only) → `env` (iff present, OR `emit_env`) →
/// `extra_fields`. The `env` sub-table is standalone (non-implicit).
fn toml_new_entry_table(entry: &TomeEntry, dialect: &McpDialect) -> TomlTable {
    let mut table = TomlTable::new();

    if let Some(ty) = dialect.entry_type {
        table.insert("type", toml_value(ty.as_str()));
    }

    match dialect.entry_shape {
        EntryShape::CommandArgs => {
            table.insert("command", toml_value(entry.command.as_str()));
            let mut args = TomlArray::new();
            for a in &entry.args {
                args.push(a.as_str());
            }
            table.insert("args", toml_value(args));
        }
        EntryShape::CommandArray => {
            let mut arr = TomlArray::new();
            arr.push(entry.command.as_str());
            for a in &entry.args {
                arr.push(a.as_str());
            }
            table.insert("command", toml_value(arr));
        }
    }

    match &entry.env {
        Some(env) => {
            let mut env_table = TomlTable::new();
            env_table.set_implicit(false);
            for (k, v) in env {
                env_table.insert(k, toml_value(v.as_str()));
            }
            table.insert("env", TomlItem::Table(env_table));
        }
        None if dialect.emit_env => {
            let mut env_table = TomlTable::new();
            env_table.set_implicit(false);
            table.insert("env", TomlItem::Table(env_table));
        }
        None => {}
    }

    for field in dialect.extra_fields {
        match field.value {
            ExtraValue::Bool(b) => {
                table.insert(field.key, toml_value(b));
            }
            ExtraValue::StringArray(items) => {
                let mut arr = TomlArray::new();
                for s in items {
                    arr.push(*s);
                }
                table.insert(field.key, toml_value(arr));
            }
        }
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
pub fn read_entry(path: &Path, dialect: &McpDialect) -> Result<Option<TomeEntry>, TomeError> {
    refuse_symlink(path)?;
    let parent_key = dialect.parent_key;
    match dialect.file_format {
        FileFormat::Json | FileFormat::Jsonc => {
            let Some(doc) = read_json_doc(path)? else {
                return Ok(None);
            };
            let Some(parent) = doc.get(parent_key).and_then(JsonValue::as_object) else {
                return Ok(None);
            };
            let Some(raw) = parent.get(MCP_CONFIG_KEY) else {
                return Ok(None);
            };
            Ok(Some(json_entry_from_value(path, raw, dialect.entry_shape)?))
        }
        FileFormat::Toml => {
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
            Ok(Some(toml_entry_from_table(
                path,
                entry_table,
                dialect.entry_shape,
            )?))
        }
    }
}

/// Write the Tome-owned entry at `mcpServers.tome` (or
/// `mcp_servers.tome`) in the harness MCP config at `path`.
///
/// Surrounding-content preservation differs by format:
/// - **TOML** preserves every other key, value, comment, blank line,
///   and ordering decision verbatim (via `toml_edit`).
/// - **JSON** preserves key order and unknown keys; whitespace and
///   indentation are normalised by `serde_json`'s pretty-printer.
///
/// Preserves the existing entry's `env` field on rewrite per FR-503
/// (only when the existing entry was already Tome-owned; a user-owned
/// entry's `env` is discarded on `--force` rewrite — see
/// `contracts/mcp-config-integration.md` § "Ownership marker (FR-501)"
/// for the safety rationale). Creates parent directory (mode 0700 on
/// Unix) and the file itself if missing. Atomic rename onto `path`.
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
pub fn write_entry(path: &Path, dialect: &McpDialect, entry: &TomeEntry) -> Result<(), TomeError> {
    refuse_symlink(path)?;

    // Idempotence pre-check: same command+args means no write. The
    // mandated `type`/`extra_fields` are re-derived from the dialect on
    // every rewrite (R3/m6), so a *stale* mandated field is NOT covered
    // by this idempotence check — but that only matters when the rest of
    // the entry also matches, in which case the on-disk file is already
    // correct (a freshly-written entry always carries the current
    // dialect's mandated fields).
    if let Some(current) = read_entry(path, dialect)?
        && super::mcp_config::is_tome_owned(&current)
        && current.command == entry.command
        && current.args == entry.args
    {
        return Ok(());
    }

    match dialect.file_format {
        FileFormat::Json | FileFormat::Jsonc => write_entry_json(path, dialect, entry),
        FileFormat::Toml => write_entry_toml(path, dialect, entry),
    }
}

fn write_entry_json(path: &Path, dialect: &McpDialect, entry: &TomeEntry) -> Result<(), TomeError> {
    let parent_key = dialect.parent_key;
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
        && let Ok(parsed) = json_entry_from_value(path, existing, dialect.entry_shape)
        && is_tome_owned(&parsed)
    {
        new_entry.env = parsed.env;
    }

    parent_obj.insert(
        MCP_CONFIG_KEY.to_string(),
        json_entry_object(&new_entry, dialect),
    );

    let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|e| parse_err(path, e))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn write_entry_toml(path: &Path, dialect: &McpDialect, entry: &TomeEntry) -> Result<(), TomeError> {
    let parent_key = dialect.parent_key;
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
            && let Ok(parsed) = toml_entry_from_table(path, existing_table, dialect.entry_shape)
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
        // Build an inline table replacement to keep shape. Field order
        // mirrors `toml_new_entry_table`: type → command → args
        // (CommandArgs only) → env → extra_fields.
        let mut inline = toml_edit::InlineTable::new();
        if let Some(ty) = dialect.entry_type {
            inline.insert("type", ty.as_str().into());
        }
        match dialect.entry_shape {
            EntryShape::CommandArgs => {
                inline.insert("command", new_entry.command.as_str().into());
                let mut args = TomlArray::new();
                for a in &new_entry.args {
                    args.push(a.as_str());
                }
                inline.insert("args", TomlValue::Array(args));
            }
            EntryShape::CommandArray => {
                let mut arr = TomlArray::new();
                arr.push(new_entry.command.as_str());
                for a in &new_entry.args {
                    arr.push(a.as_str());
                }
                inline.insert("command", TomlValue::Array(arr));
            }
        }
        match &new_entry.env {
            Some(env) => {
                let mut env_inline = toml_edit::InlineTable::new();
                for (k, v) in env {
                    env_inline.insert(k, v.as_str().into());
                }
                inline.insert("env", TomlValue::InlineTable(env_inline));
            }
            None if dialect.emit_env => {
                inline.insert("env", TomlValue::InlineTable(toml_edit::InlineTable::new()));
            }
            None => {}
        }
        for field in dialect.extra_fields {
            match field.value {
                ExtraValue::Bool(b) => {
                    inline.insert(field.key, b.into());
                }
                ExtraValue::StringArray(items) => {
                    let mut arr = TomlArray::new();
                    for s in items {
                        arr.push(*s);
                    }
                    inline.insert(field.key, TomlValue::Array(arr));
                }
            }
        }
        parent_table.insert(
            MCP_CONFIG_KEY,
            TomlItem::Value(TomlValue::InlineTable(inline)),
        );
    } else {
        let new_table = toml_new_entry_table(&new_entry, dialect);
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
pub fn remove_entry(path: &Path, dialect: &McpDialect) -> Result<(), TomeError> {
    refuse_symlink(path)?;
    let parent_key = dialect.parent_key;

    // Pre-check via `read_entry`: if the entry is absent or user-owned,
    // no write — idempotence preserved (mtime unchanged).
    let current = read_entry(path, dialect)?;
    let Some(current) = current else {
        return Ok(());
    };
    if !is_tome_owned(&current) {
        return Ok(());
    }

    match dialect.file_format {
        FileFormat::Json | FileFormat::Jsonc => {
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
        FileFormat::Toml => {
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

// =====================================================================
// Phase 11 — G1 dialect wire-shape pins + round-trips (T013).
//
// These pin the EXACT serialized bytes each dialect produces and assert
// every shape round-trips through `write_entry → read_entry →
// is_tome_owned`. The default + opencode + copilot-cli-shape full byte
// strings are pinned verbatim. The copilot-cli-like dialect is built
// inline (its harness lands in US1) purely to exercise the
// `extra_fields` + `emit_env` ordering before it ships.
// =====================================================================
#[cfg(test)]
mod dialect_pin_tests {
    use super::*;
    use crate::harness::{EntryShape, ExtraField, ExtraValue, FileFormat, McpDialect, ServerType};
    use tempfile::TempDir;

    /// The legacy JSON `mcpServers` + `CommandArgs` dialect (default).
    const DEFAULT_DIALECT: McpDialect = McpDialect::LEGACY;

    /// Codex's TOML `mcp_servers` + `CommandArgs` dialect.
    const CODEX_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Toml,
        parent_key: "mcp_servers",
        entry_shape: EntryShape::CommandArgs,
        entry_type: None,
        emit_env: false,
        extra_fields: &[],
    };

    /// OpenCode's `mcp` + `CommandArray` + `type:local` + `enabled:true`.
    const OPENCODE_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Jsonc,
        parent_key: "mcp",
        entry_shape: EntryShape::CommandArray,
        entry_type: Some(ServerType::Local),
        emit_env: false,
        extra_fields: &[ExtraField {
            key: "enabled",
            value: ExtraValue::Bool(true),
        }],
    };

    /// A copilot-cli-like dialect: `mcpServers`, `CommandArgs`,
    /// `type:local`, `emit_env:true` (→ `env:{}`), and a `tools:["*"]`
    /// extra field. Exercises `extra_fields` + `emit_env` ordering. The
    /// real harness lands in US1.
    const COPILOT_CLI_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Json,
        parent_key: "mcpServers",
        entry_shape: EntryShape::CommandArgs,
        entry_type: Some(ServerType::Local),
        emit_env: true,
        extra_fields: &[ExtraField {
            key: "tools",
            value: ExtraValue::StringArray(&["*"]),
        }],
    };

    /// Copilot (VS Code): `servers` parent key + `type:stdio`, no env.
    const COPILOT_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Json,
        parent_key: "servers",
        entry_shape: EntryShape::CommandArgs,
        entry_type: Some(ServerType::Stdio),
        emit_env: false,
        extra_fields: &[],
    };

    /// Zed: `context_servers` parent key + `CommandArgs` + `emit_env`.
    const ZED_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Json,
        parent_key: "context_servers",
        entry_shape: EntryShape::CommandArgs,
        entry_type: None,
        emit_env: true,
        extra_fields: &[],
    };

    /// Crush: `mcp` parent key + `CommandArgs` + per-entry `type:stdio`, no env.
    const CRUSH_DIALECT: McpDialect = McpDialect {
        file_format: FileFormat::Json,
        parent_key: "mcp",
        entry_shape: EntryShape::CommandArgs,
        entry_type: Some(ServerType::Stdio),
        emit_env: false,
        extra_fields: &[],
    };

    fn tome_entry() -> TomeEntry {
        TomeEntry::new(
            "tome".to_string(),
            vec![
                "mcp".to_string(),
                "--workspace".to_string(),
                "demo".to_string(),
            ],
        )
    }

    // ---- byte-stable wire pins ----------------------------------------

    #[test]
    fn default_dialect_pins_exact_bytes() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("settings.json");
        write_entry(&target, &DEFAULT_DIALECT, &tome_entry()).unwrap();
        let body = std::fs::read_to_string(&target).unwrap();
        assert_eq!(
            body,
            "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ]\n    }\n  }\n}\n",
        );
    }

    #[test]
    fn codex_dialect_pins_exact_toml_bytes() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("config.toml");
        write_entry(&target, &CODEX_DIALECT, &tome_entry()).unwrap();
        let body = std::fs::read_to_string(&target).unwrap();
        assert_eq!(
            body,
            "[mcp_servers.tome]\ncommand = \"tome\"\nargs = [\"mcp\", \"--workspace\", \"demo\"]\n",
        );
    }

    #[test]
    fn opencode_dialect_pins_exact_bytes() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("opencode.json");
        write_entry(&target, &OPENCODE_DIALECT, &tome_entry()).unwrap();
        let body = std::fs::read_to_string(&target).unwrap();
        assert_eq!(
            body,
            "{\n  \"mcp\": {\n    \"tome\": {\n      \"type\": \"local\",\n      \"command\": [\n        \"tome\",\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"enabled\": true\n    }\n  }\n}\n",
        );
    }

    #[test]
    fn copilot_cli_shape_pins_exact_bytes() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("mcp-config.json");
        write_entry(&target, &COPILOT_CLI_DIALECT, &tome_entry()).unwrap();
        let body = std::fs::read_to_string(&target).unwrap();
        // type → command → args → env:{} (emit_env) → tools (extra) order.
        assert_eq!(
            body,
            "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"type\": \"local\",\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"env\": {},\n      \"tools\": [\n        \"*\"\n      ]\n    }\n  }\n}\n",
        );
    }

    // ---- write → read-back → is_tome_owned round-trips ----------------

    fn round_trip(dialect: &McpDialect, file: &str) {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join(file);
        let entry = tome_entry();
        write_entry(&target, dialect, &entry).unwrap();
        let read = read_entry(&target, dialect)
            .unwrap()
            .expect("entry should be present after write");
        // The uniform in-memory model round-trips regardless of shape.
        assert_eq!(read.command, "tome");
        assert_eq!(read.args, vec!["mcp", "--workspace", "demo"]);
        assert!(is_tome_owned(&read), "round-tripped entry must be owned");
    }

    #[test]
    fn default_dialect_round_trips_and_is_owned() {
        round_trip(&DEFAULT_DIALECT, "settings.json");
    }

    #[test]
    fn codex_dialect_round_trips_and_is_owned() {
        round_trip(&CODEX_DIALECT, "config.toml");
    }

    #[test]
    fn opencode_command_array_round_trips_and_is_owned() {
        // CommandArray specifically: the single `command` array is
        // normalised back to command=arr[0], args=arr[1..], so the SAME
        // `is_tome_owned` predicate (command=="tome" && args[0]=="mcp")
        // applies — here arr[0]=="tome" && arr[1]=="mcp".
        round_trip(&OPENCODE_DIALECT, "opencode.json");
    }

    #[test]
    fn copilot_cli_shape_round_trips_and_is_owned() {
        round_trip(&COPILOT_CLI_DIALECT, "mcp-config.json");
    }

    // ---- clash: a foreign entry named `tome` is NOT owned -------------

    #[test]
    fn foreign_entry_named_tome_is_not_owned_commandargs() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("settings.json");
        std::fs::write(
            &target,
            "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"not-tome\",\n      \"args\": [\"serve\"]\n    }\n  }\n}\n",
        )
        .unwrap();
        let read = read_entry(&target, &DEFAULT_DIALECT).unwrap().unwrap();
        assert!(!is_tome_owned(&read));
    }

    #[test]
    fn foreign_entry_named_tome_is_not_owned_commandarray() {
        // CommandArray clash: a foreign `tome` whose command[0] != "tome".
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("opencode.json");
        std::fs::write(
            &target,
            "{\n  \"mcp\": {\n    \"tome\": {\n      \"type\": \"local\",\n      \"command\": [\"not-tome\", \"serve\"],\n      \"enabled\": true\n    }\n  }\n}\n",
        )
        .unwrap();
        let read = read_entry(&target, &OPENCODE_DIALECT).unwrap().unwrap();
        assert_eq!(read.command, "not-tome");
        assert_eq!(read.args, vec!["serve"]);
        assert!(!is_tome_owned(&read));
    }

    // ---- mandated fields self-heal on rewrite (R3/m6) -----------------

    #[test]
    fn opencode_rewrite_reasserts_type_and_enabled() {
        // A developer edits away the mandated `type`/`enabled` AND changes
        // the workspace arg (so the idempotence pre-check doesn't short-
        // circuit). The rewrite must re-derive both mandated fields.
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("opencode.json");
        std::fs::write(
            &target,
            "{\n  \"mcp\": {\n    \"tome\": {\n      \"command\": [\"tome\", \"mcp\", \"--workspace\", \"stale\"]\n    }\n  }\n}\n",
        )
        .unwrap();
        write_entry(&target, &OPENCODE_DIALECT, &tome_entry()).unwrap();
        let body = std::fs::read_to_string(&target).unwrap();
        assert!(
            body.contains("\"type\": \"local\""),
            "type re-derived:\n{body}"
        );
        assert!(
            body.contains("\"enabled\": true"),
            "enabled re-derived:\n{body}"
        );
        assert!(body.contains("\"--workspace\""));
        assert!(body.contains("\"demo\""));
    }

    // ---- developer env is preserved across a rewrite ------------------

    #[test]
    fn default_dialect_preserves_developer_env_on_rewrite() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("settings.json");
        // Seed an owned entry WITH a developer env and a stale workspace.
        std::fs::write(
            &target,
            "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\"mcp\", \"--workspace\", \"stale\"],\n      \"env\": { \"MY_FLAG\": \"1\" }\n    }\n  }\n}\n",
        )
        .unwrap();
        write_entry(&target, &DEFAULT_DIALECT, &tome_entry()).unwrap();
        let read = read_entry(&target, &DEFAULT_DIALECT).unwrap().unwrap();
        assert_eq!(
            read.env,
            Some(vec![("MY_FLAG".to_string(), "1".to_string())]),
            "developer env must survive the rewrite",
        );
    }

    // ---- remove_entry under the new dialect parent keys (the disable path) ----
    //
    // Each: seed a Tome-owned entry + a FOREIGN sibling server under the SAME
    // parent key, `remove_entry`, then assert the `tome` key is gone and the
    // foreign sibling survives byte-for-byte (value-equality, since serde_json's
    // pretty-printer normalises whitespace).

    /// Seed `{ "<parent_key>": { "tome": {<owned>}, "other": {<foreign>} } }`,
    /// remove the Tome entry, and return the re-read parent object so the caller
    /// can assert on the surviving siblings.
    fn remove_entry_preserves_sibling(
        dialect: &McpDialect,
        file: &str,
        owned: &str,
        foreign: &str,
    ) -> JsonValue {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join(file);
        let seed = format!(
            "{{\n  \"{key}\": {{\n    \"tome\": {owned},\n    \"other\": {foreign}\n  }}\n}}\n",
            key = dialect.parent_key,
        );
        std::fs::write(&target, seed).unwrap();
        // Sanity: the seeded Tome entry IS owned before removal.
        assert!(is_tome_owned(
            &read_entry(&target, dialect).unwrap().unwrap()
        ));

        remove_entry(&target, dialect).unwrap();

        let body = std::fs::read_to_string(&target).unwrap();
        let doc: JsonValue = serde_json::from_str(&body).unwrap();
        let parent = doc
            .get(dialect.parent_key)
            .and_then(JsonValue::as_object)
            .expect("parent object survives removal");
        assert!(
            parent.get("tome").is_none(),
            "tome key must be gone after remove_entry:\n{body}",
        );
        assert!(
            parent.get("other").is_some(),
            "foreign sibling must survive remove_entry:\n{body}",
        );
        doc.get(dialect.parent_key).cloned().unwrap()
    }

    #[test]
    fn remove_entry_servers_key_preserves_foreign_sibling() {
        // copilot (VS Code): `servers` + type:stdio.
        let parent = remove_entry_preserves_sibling(
            &COPILOT_DIALECT,
            "mcp.json",
            "{ \"type\": \"stdio\", \"command\": \"tome\", \"args\": [\"mcp\"] }",
            "{ \"type\": \"stdio\", \"command\": \"other-bin\", \"args\": [\"run\"] }",
        );
        let other = &parent["other"];
        assert_eq!(other["command"], "other-bin");
        assert_eq!(other["args"][0], "run");
    }

    #[test]
    fn remove_entry_context_servers_key_preserves_foreign_sibling() {
        // zed: `context_servers` + CommandArgs.
        let parent = remove_entry_preserves_sibling(
            &ZED_DIALECT,
            "settings.json",
            "{ \"command\": \"tome\", \"args\": [\"mcp\"] }",
            "{ \"command\": \"other-bin\", \"args\": [\"run\"] }",
        );
        assert_eq!(parent["other"]["command"], "other-bin");
    }

    #[test]
    fn remove_entry_mcp_key_type_stdio_preserves_foreign_sibling() {
        // crush: `mcp` parent key + per-entry type:stdio.
        let parent = remove_entry_preserves_sibling(
            &CRUSH_DIALECT,
            "crush.json",
            "{ \"type\": \"stdio\", \"command\": \"tome\", \"args\": [\"mcp\"] }",
            "{ \"type\": \"stdio\", \"command\": \"other-bin\", \"args\": [\"run\"] }",
        );
        assert_eq!(parent["other"]["command"], "other-bin");
        assert_eq!(parent["other"]["type"], "stdio");
    }

    #[test]
    fn remove_entry_mcpservers_tools_extra_preserves_foreign_sibling() {
        // copilot-cli: `mcpServers` + type:local + env:{} + tools:["*"].
        let parent = remove_entry_preserves_sibling(
            &COPILOT_CLI_DIALECT,
            "mcp-config.json",
            "{ \"type\": \"local\", \"command\": \"tome\", \"args\": [\"mcp\"], \"env\": {}, \"tools\": [\"*\"] }",
            "{ \"type\": \"local\", \"command\": \"other-bin\", \"args\": [\"run\"], \"tools\": [\"x\"] }",
        );
        assert_eq!(parent["other"]["command"], "other-bin");
        assert_eq!(parent["other"]["tools"][0], "x");
    }

    // ---- write_entry preserves a foreign sibling under the new parent keys ----

    #[test]
    fn write_entry_servers_key_preserves_foreign_sibling() {
        // Seed ONLY a foreign sibling under `servers`; writing the Tome entry
        // must ADD `tome` and leave `other` intact.
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("mcp.json");
        std::fs::write(
            &target,
            "{\n  \"servers\": {\n    \"other\": { \"type\": \"stdio\", \"command\": \"other-bin\", \"args\": [\"run\"] }\n  }\n}\n",
        )
        .unwrap();
        write_entry(&target, &COPILOT_DIALECT, &tome_entry()).unwrap();

        let body = std::fs::read_to_string(&target).unwrap();
        let doc: JsonValue = serde_json::from_str(&body).unwrap();
        let servers = doc.get("servers").and_then(JsonValue::as_object).unwrap();
        assert!(
            servers.get("other").is_some(),
            "foreign sibling must survive the write:\n{body}",
        );
        assert_eq!(servers["other"]["command"], "other-bin");
        // The Tome entry was added and is owned.
        let read = read_entry(&target, &COPILOT_DIALECT).unwrap().unwrap();
        assert!(is_tome_owned(&read));
    }

    #[test]
    fn write_entry_context_servers_key_preserves_foreign_sibling() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("settings.json");
        std::fs::write(
            &target,
            "{\n  \"context_servers\": {\n    \"other\": { \"command\": \"other-bin\", \"args\": [\"run\"] }\n  }\n}\n",
        )
        .unwrap();
        write_entry(&target, &ZED_DIALECT, &tome_entry()).unwrap();

        let body = std::fs::read_to_string(&target).unwrap();
        let doc: JsonValue = serde_json::from_str(&body).unwrap();
        let parent = doc
            .get("context_servers")
            .and_then(JsonValue::as_object)
            .unwrap();
        assert!(
            parent.get("other").is_some(),
            "foreign sibling must survive the write:\n{body}",
        );
        assert_eq!(parent["other"]["command"], "other-bin");
        let read = read_entry(&target, &ZED_DIALECT).unwrap().unwrap();
        assert!(is_tome_owned(&read));
    }
}
