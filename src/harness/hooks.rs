//! Real Claude Code hooks: read a plugin's `hooks/hooks.json`, rewrite its
//! two path variables, and merge / remove the rewritten entries into the
//! project's machine-local `.claude/settings.local.json` (Phase 6 / US2).
//!
//! Contract: `specs/006-phase-6-hooks-agents/contracts/hooks-integration.md`.
//!
//! ## Scope
//!
//! Only the Claude Code harness drives this module (`hooks_strategy() ==
//! RealJson`). Every other harness is `GuardrailsOnly` and falls back to the
//! prose `GUARDRAILS.md` region (US3) — this module is never reached for them.
//!
//! ## Two-variable rewrite (FR-003, R-4)
//!
//! A **targeted** textual substitution over the JSON string leaves only —
//! NOT the Phase 5 substitution pipeline (NFR-007: no parallel substitution
//! path). Exactly two tokens are rewritten:
//!
//! - `${CLAUDE_PLUGIN_ROOT}` → the absolute installed-plugin root.
//! - `${CLAUDE_PLUGIN_DATA}` → the plugin-data dir
//!   (`~/.tome/plugin-data/<catalog>/<plugin>/`).
//!
//! Every other `${CLAUDE_*}` token (e.g. `${CLAUDE_PROJECT_DIR}`,
//! `${CLAUDE_SESSION_ID}`) is left **verbatim** — Claude Code resolves those
//! natively at runtime. Keys and non-string scalars are never touched.
//!
//! ## Merge ownership (FR-004, FR-005, FR-006, NFR-003)
//!
//! Ownership is established **solely** by re-derivation + deep
//! `serde_json::Value` equality — no sidecar, no provenance marker. A hook
//! the user hand-edited after Tome wrote it no longer matches and is left in
//! place; Tome never deletes a hook it cannot prove it owns.
//!
//! The committed `.claude/settings.json` is **never** written — only the
//! local, gitignored `settings.local.json` (rewritten hooks carry
//! machine-specific absolute paths and must not be committed).

use std::io::Write;
use std::path::Path;

use serde_json::{Map as JsonMap, Value as JsonValue};
use tempfile::NamedTempFile;

use crate::error::TomeError;

/// Read and rewrite a plugin's `hooks/hooks.json` into its post-rewrite
/// per-event entries.
///
/// Returns `Ok(None)` when the plugin ships no `hooks/hooks.json` (a benign
/// fall-through to guardrails for Claude Code, FR-001). A malformed /
/// unparsable file → [`TomeError::HookSpecParseError`] (exit 43), naming the
/// file. The two-variable rewrite (FR-003) is applied to every string leaf.
///
/// `plugin_root` is the absolute installed-plugin root
/// (`${CLAUDE_PLUGIN_ROOT}`); `plugin_data` is the plugin-data dir
/// (`${CLAUDE_PLUGIN_DATA}`).
pub fn read_rewritten_entries(
    plugin_root: &Path,
    plugin_data: &Path,
) -> Result<Option<RewrittenHooks>, TomeError> {
    let source = plugin_root.join("hooks").join("hooks.json");

    // A symlinked source is refused like every other harness read/write.
    refuse_symlink(&source)?;

    let body = match crate::util::bounded_read_to_string(&source, crate::util::HARNESS_MCP_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // A read failure other than "absent" is a hook spec failure (exit 43)
        // naming the file — the contract's "malformed or unparsable" covers an
        // unreadable source.
        Err(_) => return Err(TomeError::HookSpecParseError { path: source }),
    };
    if body.trim().is_empty() {
        return Ok(None);
    }

    let mut doc: JsonValue =
        serde_json::from_str(&body).map_err(|_| TomeError::HookSpecParseError {
            path: source.clone(),
        })?;

    // The top-level shape is an object keyed by event name; each value is an
    // array of hook entries. Anything else is malformed.
    let Some(obj) = doc.as_object_mut() else {
        return Err(TomeError::HookSpecParseError { path: source });
    };

    // Fail closed on a non-UTF-8 rewrite target. These values become
    // LOAD-BEARING text inside an executed hook command; `to_string_lossy`
    // would substitute a U+FFFD-corrupted path, emitting a silently-broken
    // command rather than refusing. Surface exit 44 instead (R2-2).
    let plugin_root_str = non_utf8_guard(plugin_root, plugin_root)?;
    let plugin_data_str = non_utf8_guard(plugin_data, plugin_root)?;

    let mut events: Vec<(String, Vec<JsonValue>)> = Vec::with_capacity(obj.len());
    for (event, value) in obj.iter() {
        let Some(arr) = value.as_array() else {
            return Err(TomeError::HookSpecParseError { path: source });
        };
        let mut entries = Vec::with_capacity(arr.len());
        for entry in arr {
            let mut rewritten = entry.clone();
            rewrite_string_leaves(&mut rewritten, plugin_root_str, plugin_data_str);
            entries.push(rewritten);
        }
        events.push((event.clone(), entries));
    }

    Ok(Some(RewrittenHooks { events }))
}

/// A plugin's post-rewrite hook entries, grouped by event name in source
/// order. Each entry is the fully-rewritten `serde_json::Value` object that
/// merges into / removes from `settings.local.json` under its event key by
/// deep structural equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenHooks {
    pub events: Vec<(String, Vec<JsonValue>)>,
}

impl RewrittenHooks {
    /// `true` when the plugin contributed no hook entries at all.
    pub fn is_empty(&self) -> bool {
        self.events.iter().all(|(_, entries)| entries.is_empty())
    }
}

/// Return `path` as `&str`, or fail closed with exit 44 when it is not valid
/// UTF-8. The rewritten value is injected into an executed hook command, so a
/// non-UTF-8 install path must be refused rather than `to_string_lossy`'d into
/// a U+FFFD-corrupted command (R2-2). `error_path` names the offending plugin
/// root in the surfaced [`TomeError::HookSettingsWriteFailed`].
fn non_utf8_guard<'a>(path: &'a Path, error_path: &Path) -> Result<&'a str, TomeError> {
    path.to_str().ok_or_else(|| {
        settings_write_failed(
            error_path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("non-UTF-8 hook rewrite target path: {}", path.display()),
            ),
        )
    })
}

/// Recursively rewrite the two recognised `${CLAUDE_*}` tokens in every
/// JSON **string leaf**. Keys, numbers, booleans, and nulls are untouched.
///
/// A literal textual replace of exactly the two tokens — no regex engine is
/// needed for two fixed needles, and a fixed-needle `replace` cannot
/// accidentally match a longer variable name (e.g. `${CLAUDE_PLUGIN_ROOTX}`
/// is not a token Claude Code defines, and the replace only fires on the
/// exact `${CLAUDE_PLUGIN_ROOT}` byte sequence — a following `X` survives as
/// trailing text, which is harmless because no such variable exists). All
/// other `${CLAUDE_*}` tokens are left verbatim by construction: they are
/// never in the needle set.
fn rewrite_string_leaves(value: &mut JsonValue, plugin_root: &str, plugin_data: &str) {
    match value {
        JsonValue::String(s) => {
            if s.contains("${CLAUDE_PLUGIN_ROOT}") {
                *s = s.replace("${CLAUDE_PLUGIN_ROOT}", plugin_root);
            }
            if s.contains("${CLAUDE_PLUGIN_DATA}") {
                *s = s.replace("${CLAUDE_PLUGIN_DATA}", plugin_data);
            }
        }
        JsonValue::Array(arr) => {
            for item in arr {
                rewrite_string_leaves(item, plugin_root, plugin_data);
            }
        }
        JsonValue::Object(map) => {
            // Only the VALUES are rewritten; keys stay verbatim.
            for (_k, v) in map.iter_mut() {
                rewrite_string_leaves(v, plugin_root, plugin_data);
            }
        }
        // Numbers / booleans / null carry no rewritable text.
        _ => {}
    }
}

// =====================================================================
// settings.local.json merge / remove
// =====================================================================

/// Merge `hooks`'s rewritten entries into `settings.local.json` at
/// `target`, appending each entry under its event only when no deep-equal
/// entry already exists there (idempotent, never duplicates a user-authored
/// identical entry — FR-004).
///
/// Creates the file (with a single `{"hooks": {}}` object) and its parent
/// `.claude/` (0700 on Unix) when absent. Atomic, mode-preserving,
/// symlink-refusing write. Any read / merge / write failure surfaces
/// [`TomeError::HookSettingsWriteFailed`] (exit 44), naming the file.
///
/// Returns `true` when the file was changed on disk (so the caller can
/// classify Created/Updated vs LeftAlone); `false` on an idempotent no-op.
pub fn merge_into_settings(target: &Path, hooks: &RewrittenHooks) -> Result<bool, TomeError> {
    refuse_symlink_settings(target)?;

    let (mut doc, existed) = load_settings(target)?;
    let mut changed = false;

    {
        let hooks_obj = ensure_hooks_object(&mut doc, target)?;
        for (event, entries) in &hooks.events {
            for entry in entries {
                if append_if_absent(hooks_obj, event, entry) {
                    changed = true;
                }
            }
        }
    }

    // Create-if-absent: even when the plugin contributes nothing, the
    // contract creates `{"hooks": {}}` (FR-002). But when the file already
    // exists and nothing changed, do not rewrite (idempotence).
    if !existed {
        write_settings(target, &doc)?;
        return Ok(true);
    }
    if changed {
        write_settings(target, &doc)?;
    }
    Ok(changed)
}

/// Remove `hooks`'s rewritten entries from `settings.local.json` at
/// `target` by deep structural equality, then prune any now-empty event
/// array (FR-005, FR-006). Non-matching / user-edited entries are left in
/// place — ownership is re-derivation + structural match only (NFR-003).
///
/// A missing file is a no-op. The otherwise-empty `hooks` object and the
/// file itself are left in place (FR-006). Returns `true` when the file was
/// changed on disk.
pub fn remove_from_settings(target: &Path, hooks: &RewrittenHooks) -> Result<bool, TomeError> {
    refuse_symlink_settings(target)?;

    let (mut doc, existed) = load_settings(target)?;
    if !existed {
        return Ok(false);
    }

    let mut changed = false;
    {
        // Only touch an existing `hooks` object; do not create one on removal.
        if let Some(hooks_obj) = doc
            .as_object_mut()
            .and_then(|o| o.get_mut("hooks"))
            .and_then(JsonValue::as_object_mut)
        {
            for (event, entries) in &hooks.events {
                for entry in entries {
                    if remove_if_present(hooks_obj, event, entry) {
                        changed = true;
                    }
                }
                prune_empty_event(hooks_obj, event);
            }
        }
    }

    if changed {
        write_settings(target, &doc)?;
    }
    Ok(changed)
}

/// Append `entry` under `event` in `hooks_obj` unless a deep-equal entry is
/// already present there. Returns `true` when the entry was appended.
fn append_if_absent(
    hooks_obj: &mut JsonMap<String, JsonValue>,
    event: &str,
    entry: &JsonValue,
) -> bool {
    let arr = hooks_obj
        .entry(event.to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    // If the event key existed but was not an array, replace it with one.
    if !arr.is_array() {
        *arr = JsonValue::Array(Vec::new());
    }
    let Some(items) = arr.as_array_mut() else {
        return false;
    };
    if items.iter().any(|existing| existing == entry) {
        return false;
    }
    items.push(entry.clone());
    true
}

/// Remove the deep-equal `entry` under `event` in `hooks_obj`. Returns
/// `true` when an entry was removed. A non-matching entry is left in place.
fn remove_if_present(
    hooks_obj: &mut JsonMap<String, JsonValue>,
    event: &str,
    entry: &JsonValue,
) -> bool {
    let Some(items) = hooks_obj.get_mut(event).and_then(JsonValue::as_array_mut) else {
        return false;
    };
    let before = items.len();
    items.retain(|existing| existing != entry);
    before != items.len()
}

/// Drop `event`'s key from `hooks_obj` when its array is now empty (FR-006).
/// A non-array or absent value is left untouched.
fn prune_empty_event(hooks_obj: &mut JsonMap<String, JsonValue>, event: &str) {
    let is_empty_array = hooks_obj
        .get(event)
        .and_then(JsonValue::as_array)
        .is_some_and(|a| a.is_empty());
    if is_empty_array {
        hooks_obj.shift_remove(event);
    }
}

/// Load `settings.local.json`, returning `(value, existed)`. An absent file
/// yields a fresh `{"hooks": {}}` object with `existed = false`. A malformed
/// existing file → [`TomeError::HookSettingsWriteFailed`] (exit 44).
fn load_settings(target: &Path) -> Result<(JsonValue, bool), TomeError> {
    let body = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_MCP_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut obj = JsonMap::new();
            obj.insert("hooks".to_string(), JsonValue::Object(JsonMap::new()));
            return Ok((JsonValue::Object(obj), false));
        }
        // Any other read failure (permissions, oversize cap, non-UTF-8) maps
        // to the exit-44 settings failure naming the file.
        Err(TomeError::Io(e)) => return Err(settings_write_failed(target, e)),
        Err(other) => return Err(other),
    };
    if body.trim().is_empty() {
        let mut obj = JsonMap::new();
        obj.insert("hooks".to_string(), JsonValue::Object(JsonMap::new()));
        return Ok((JsonValue::Object(obj), true));
    }
    let value = serde_json::from_str::<JsonValue>(&body).map_err(|e| {
        settings_write_failed(
            target,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
        )
    })?;
    if !value.is_object() {
        return Err(settings_write_failed(
            target,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "settings.local.json top-level value must be an object",
            ),
        ));
    }
    Ok((value, true))
}

/// Borrow (creating if needed) the `hooks` object inside the loaded
/// settings document. A `hooks` value of the wrong type → exit 44.
fn ensure_hooks_object<'a>(
    doc: &'a mut JsonValue,
    target: &Path,
) -> Result<&'a mut JsonMap<String, JsonValue>, TomeError> {
    let obj = doc.as_object_mut().ok_or_else(|| {
        settings_write_failed(
            target,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "settings.local.json top-level value must be an object",
            ),
        )
    })?;
    let entry = obj
        .entry("hooks".to_string())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    if !entry.is_object() {
        return Err(settings_write_failed(
            target,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "settings.local.json `hooks` value must be an object",
            ),
        ));
    }
    Ok(entry
        .as_object_mut()
        .expect("hooks ensured to be an object"))
}

// =====================================================================
// Atomic write + symlink refusal
// =====================================================================

/// Refuse to read through a symlinked hook source (a plugin's `hooks.json`).
/// Delegates to the SSOT guard (`util::symlink_safe`) for intermediate-component
/// hardening (FR-007); a symlink surfaces as `Io` (exit 7), the dedicated code
/// for reading non-sink third-party content.
fn refuse_symlink(target: &Path) -> Result<(), TomeError> {
    crate::util::refuse_symlinked_component(target).map_err(TomeError::Io)
}

/// Refuse to write through a symlinked settings file. `settings.local.json` is a
/// dedicated Phase 6 sink, so a symlinked component here surfaces as exit 44
/// (`HookSettingsWriteFailed`), reconciled with exit-codes-p6.md and the parallel
/// guardrails-target → 46 decision (code 7 is reserved for IO that is *not* the
/// local Claude settings file). Delegates to the SSOT guard so the intermediate-
/// component hardening (FR-007) lands here too, then re-maps the refusal onto
/// this sink's dedicated exit-44 variant — never a regression to generic `Io`.
fn refuse_symlink_settings(target: &Path) -> Result<(), TomeError> {
    crate::util::refuse_symlinked_component(target).map_err(|e| settings_write_failed(target, e))
}

/// Map a write-path IO failure to the exit-44 variant naming the file.
fn settings_write_failed(target: &Path, source: std::io::Error) -> TomeError {
    TomeError::HookSettingsWriteFailed {
        path: target.to_path_buf(),
        source,
    }
}

/// Serialise `doc` and atomically replace `target`, creating the parent
/// `.claude/` (0700 on Unix) when absent and preserving the existing file's
/// mode (new files get the tempfile default, typically 0600). Mirrors the
/// `mcp_config::atomic_write` discipline; every failure maps to exit 44.
fn write_settings(target: &Path, doc: &JsonValue) -> Result<(), TomeError> {
    let mut bytes = serde_json::to_vec_pretty(doc)
        .map_err(|e| settings_write_failed(target, std::io::Error::other(e)))?;
    bytes.push(b'\n');

    let parent = target.parent().ok_or_else(|| {
        settings_write_failed(target, std::io::Error::other("path has no parent"))
    })?;
    let parent_existed = parent.exists();
    std::fs::create_dir_all(parent).map_err(|e| settings_write_failed(target, e))?;
    #[cfg(unix)]
    if !parent_existed {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| settings_write_failed(target, e))?;
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

    let mut tmp = NamedTempFile::with_prefix_in(".tome.tmp.", parent)
        .map_err(|e| settings_write_failed(target, e))?;
    tmp.write_all(&bytes)
        .map_err(|e| settings_write_failed(target, e))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| settings_write_failed(target, e))?;

    #[cfg(unix)]
    if let Some(mode) = target_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(mode))
            .map_err(|e| settings_write_failed(target, e))?;
    }

    tmp.persist(target)
        .map_err(|e| settings_write_failed(target, e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cmd: &str) -> JsonValue {
        serde_json::json!({
            "matcher": "Bash",
            "hooks": [ { "type": "command", "command": cmd } ]
        })
    }

    #[test]
    fn rewrite_resolves_only_the_two_tokens() {
        let mut v = serde_json::json!({
            "command": "${CLAUDE_PLUGIN_ROOT}/g.sh --root ${CLAUDE_PROJECT_DIR} --data ${CLAUDE_PLUGIN_DATA} --sess ${CLAUDE_SESSION_ID}"
        });
        rewrite_string_leaves(&mut v, "/abs/root", "/abs/data");
        let s = v["command"].as_str().unwrap();
        assert_eq!(
            s,
            "/abs/root/g.sh --root ${CLAUDE_PROJECT_DIR} --data /abs/data --sess ${CLAUDE_SESSION_ID}"
        );
    }

    #[test]
    fn rewrite_leaves_keys_untouched() {
        // A key that looks like a token must NOT be rewritten — only values.
        let mut v = serde_json::json!({ "${CLAUDE_PLUGIN_ROOT}": "x" });
        rewrite_string_leaves(&mut v, "/abs/root", "/abs/data");
        assert!(
            v.as_object().unwrap().contains_key("${CLAUDE_PLUGIN_ROOT}"),
            "key must stay verbatim: {v}"
        );
    }

    #[test]
    fn append_if_absent_is_idempotent() {
        let mut hooks = JsonMap::new();
        let e = entry("/x/g.sh");
        assert!(append_if_absent(&mut hooks, "PreToolUse", &e));
        // Second identical append is a no-op.
        assert!(!append_if_absent(&mut hooks, "PreToolUse", &e));
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn remove_then_prune_drops_empty_event() {
        let mut hooks = JsonMap::new();
        let e = entry("/x/g.sh");
        append_if_absent(&mut hooks, "PreToolUse", &e);
        assert!(remove_if_present(&mut hooks, "PreToolUse", &e));
        prune_empty_event(&mut hooks, "PreToolUse");
        assert!(
            !hooks.contains_key("PreToolUse"),
            "empty event array must be pruned"
        );
    }

    // A non-UTF-8 path can only be constructed from raw bytes on Unix; gate
    // the construction on Linux per project convention.
    #[test]
    #[cfg(target_os = "linux")]
    fn non_utf8_rewrite_target_is_refused_exit_44() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // 0xFF is never valid UTF-8.
        let bad = Path::new(OsStr::from_bytes(b"/tmp/\xff/plugin"));
        let good = Path::new("/tmp/data");

        // Non-UTF-8 plugin_root → refused.
        let err = non_utf8_guard(bad, bad).expect_err("non-UTF-8 root must refuse");
        assert_eq!(
            err.exit_code(),
            44,
            "non-UTF-8 target → exit 44; got {err:?}"
        );
        match &err {
            TomeError::HookSettingsWriteFailed { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::InvalidData);
            }
            other => panic!("expected HookSettingsWriteFailed, got {other:?}"),
        }

        // A valid UTF-8 path passes through unchanged.
        assert_eq!(non_utf8_guard(good, good).expect("utf-8 ok"), "/tmp/data");
    }

    #[test]
    fn remove_skips_non_matching_entry() {
        let mut hooks = JsonMap::new();
        let user_edited = entry("/x/g.sh --extra-flag");
        append_if_absent(&mut hooks, "PreToolUse", &user_edited);
        // Tome's re-derived entry differs → not removed.
        let derived = entry("/x/g.sh");
        assert!(!remove_if_present(&mut hooks, "PreToolUse", &derived));
        assert_eq!(hooks["PreToolUse"].as_array().unwrap().len(), 1);
    }
}
