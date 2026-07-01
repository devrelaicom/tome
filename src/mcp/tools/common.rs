//! Shared lookup helpers for the read-side MCP tools (`get_skill`,
//! `get_skill_info`).
//!
//! Both tools need to answer the SAME question when a `(catalog, plugin, name)`
//! lookup fails to resolve an enabled entry: *which* not-found case is it —
//! `unknown_catalog`, `unknown_plugin`, or `unknown_skill`? #295 aligned
//! `get_skill_info` onto `get_skill`'s three-code surface so an agent never
//! pays a second round-trip to learn whether the catalog, the plugin, or just
//! the entry name was wrong.
//!
//! The catalog/plugin existence guards live here as the single source of truth
//! (the "single-source-of-truth promotion at the second consumer" pattern):
//! `get_skill` used to own them inline, and `get_skill_info` would otherwise
//! have had to copy-paste the identical `workspace_catalogs::find` +
//! `list_for_plugin`-is-empty logic. Both now call [`classify_not_found`].

use serde_json::{Value, json};

use crate::error::{ErrorCategory, TomeError};
use crate::index::{skills, workspace_catalogs};

/// Build the MCP tool error `data` payload for a closed [`ErrorCategory`].
///
/// #296: the single source of truth for the MCP `data` object — `code` (the
/// wire-stable category slug), plus the `retryable` bool and optional
/// `remediation` command hint derived from the SAME accessors the CLI `--json`
/// error envelope uses ([`ErrorCategory::retryable`] / [`remediation`]). Every
/// MCP surface that attaches a category-driven `code` routes through this so an
/// agent branches on structured data instead of regexing the English message,
/// and the CLI and MCP can never disagree on `code`/`retryable`/`remediation`
/// for the same failure.
///
/// `remediation` is omitted when the category has no single fix command, so the
/// payload gains only the always-present `retryable` field over the historical
/// `{ "code": … }` shape (plus `remediation` when one exists).
///
/// [`remediation`]: ErrorCategory::remediation
pub fn error_data(category: ErrorCategory) -> Value {
    error_data_with_code(category.as_str(), category, &[])
}

/// Like [`error_data`] but keeps a **custom** `code` slug (one that is not a
/// bare `ErrorCategory::as_str()` — e.g. `embedder_drift`, `unknown_catalog`,
/// `query_too_long`) while still deriving `retryable`/`remediation` from the
/// representative `category`, and merges any `extra` `(key, value)` fields the
/// call site already surfaced (e.g. `catalog`, `harness`, `max`).
///
/// This lets the input-validation and drift error sites keep their byte-stable
/// custom `code` + extra fields yet gain the same structured `retryable` /
/// `remediation` as every other error — one SSOT, no divergence.
pub fn error_data_with_code(code: &str, category: ErrorCategory, extra: &[(&str, Value)]) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".to_string(), json!(code));
    obj.insert("retryable".to_string(), json!(category.retryable()));
    if let Some(remediation) = category.remediation() {
        obj.insert("remediation".to_string(), json!(remediation));
    }
    for (key, value) in extra {
        obj.insert((*key).to_string(), value.clone());
    }
    Value::Object(obj)
}

/// The three not-found classifications the read-side tools distinguish, in the
/// contract's precedence order (catalog, then plugin, then skill). Each maps to
/// a stable `data.code` slug on the wire (`unknown_catalog` / `unknown_plugin`
/// / `unknown_skill`) — identical across `get_skill` and `get_skill_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotFound {
    /// The catalog is not enrolled in the resolved workspace's
    /// `workspace_catalogs`.
    UnknownCatalog,
    /// The catalog is enrolled but no rows exist for `(catalog, plugin)` — the
    /// plugin isn't part of this workspace at all.
    UnknownPlugin,
    /// The `(catalog, plugin)` pair has rows, but none matched the requested
    /// entry (absent, or present only as a disabled row).
    UnknownSkill,
}

/// Is `catalog` enrolled in `workspace_name`'s `workspace_catalogs`?
///
/// The single existence gate both read-side tools use — catalog enrolment is
/// resolved from the DB (`workspace_catalogs`), never `config.toml [catalogs]`
/// (FF3: the latter is never written in production, so reading it returned
/// `unknown_catalog` for every enrolled catalog on a fresh install).
pub fn catalog_enrolled(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    catalog: &str,
) -> Result<bool, TomeError> {
    Ok(workspace_catalogs::find(conn, workspace_name, catalog)?.is_some())
}

/// Classify a not-found lookup as `unknown_catalog` / `unknown_plugin` /
/// `unknown_skill`, in that precedence order.
///
/// Call this ONLY once a `(catalog, plugin, name)` lookup has failed to resolve
/// an enabled entry — it re-derives which layer was actually missing:
///
/// 1. Catalog not enrolled in `workspace_name` → [`NotFound::UnknownCatalog`].
/// 2. Enrolled, but zero rows for `(catalog, plugin)` → [`NotFound::UnknownPlugin`].
/// 3. Otherwise (the plugin has rows, just not this enabled entry) →
///    [`NotFound::UnknownSkill`].
///
/// This is the shared classifier `get_skill` and `get_skill_info` both route
/// through, so the two surfaces emit byte-identical codes + precedence for the
/// same failure. `list_for_plugin` returns every row for the pair regardless of
/// per-row enablement, so "empty" means the plugin genuinely has no entries in
/// this workspace — the same `unknown_plugin` vs `unknown_skill` split
/// `get_skill` has always drawn.
pub fn classify_not_found(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    catalog: &str,
    plugin: &str,
) -> Result<NotFound, TomeError> {
    if !catalog_enrolled(conn, workspace_name, catalog)? {
        return Ok(NotFound::UnknownCatalog);
    }
    let any = skills::list_for_plugin(conn, workspace_name, catalog, plugin)?;
    if any.is_empty() {
        Ok(NotFound::UnknownPlugin)
    } else {
        Ok(NotFound::UnknownSkill)
    }
}
