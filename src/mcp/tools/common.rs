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

use crate::error::TomeError;
use crate::index::{skills, workspace_catalogs};

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
