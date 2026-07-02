//! `tome tier {set,list,clear}` — per-workspace skill/command routing tiers.
//!
//! Tiers live on the `workspace_skills.tier` column (schema v5). `set` and
//! `clear` perform an UPDATE under the advisory `index.lock` (FR-040); `list`
//! is a read-only projection. After a successful `set`/`clear`, the workspace's
//! `RULES.md` (and every bound project's mirror) is regenerated so the routing
//! directive reflects the new tier immediately.

use crate::cli::TierCommand;
use crate::error::TomeError;
use crate::index::skills::{
    TieredEntry, enabled_plugins_for_workspace, tiered_entries_for_workspace,
};
use crate::output::Mode;
use crate::plugin::PluginId;
use crate::plugin::identity::EntryKind;
use crate::plugin::selector::{self, glob_match, is_glob};
use crate::workspace::ResolvedScope;

mod clear;
mod list;
pub(crate) mod set;

pub fn run(cmd: TierCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        TierCommand::Set(args) => set::run(args, scope, mode),
        TierCommand::List(args) => list::run(args, scope, mode),
        TierCommand::Clear(args) => clear::run(args, scope, mode),
    }
}

/// Shared `<plugin>/<name>` parse.
pub(crate) fn split_id(id: &str) -> Result<(&str, &str), TomeError> {
    match id.split_once('/') {
        Some((p, n)) if !p.is_empty() && !n.is_empty() => Ok((p, n)),
        _ => Err(TomeError::Usage(format!(
            "invalid entry id `{id}` (expected `<plugin>/<name>`)"
        ))),
    }
}

/// Resolve the `--plugin <sel>` selectors and/or the positional `<plugin>/<name>`
/// id into a deduped target set of [`TieredEntry`] (issue #317). This is the
/// SSOT shared by `set` and `clear`, computed ONCE against a single snapshot of
/// the workspace's tierable entries (`tiered_entries_for_workspace`, which is
/// the same set `tier list` and the routing directive see; agents excluded).
///
/// Exactly one of `id` / `plugins` is non-empty at the call site — the XOR is
/// enforced upstream (clap-level `conflicts_with`/`required_unless_present` for
/// `clear`; the `set` positional splitter for `set`, whose trailing tier makes a
/// clap-level XOR ambiguous). Both sources honour `--catalog` / `--kind`:
///
/// * **`--plugin <sel>`** (repeatable): each selector is resolved via
///   [`selector::resolve`] against the ENABLED-plugin candidate set. This is the
///   right reuse — a bare `--plugin` token means a PLUGIN name, matching the
///   selector's bare-token semantics exactly. Each resolved [`PluginId`] then
///   collects its enabled tierable entries (catalog + plugin match, `--kind`
///   honoured). A `--plugin` naming a plugin with zero tierable entries →
///   [`TomeError::EntryNotFound`] (exit 27). Selector failures map through
///   [`selector::SelectorError::into_tome_error`] (Usage/2, PluginNotFound/20).
///
/// * **positional `<plugin>/<name>`**: parsed via [`split_id`]. If the name (or
///   plugin) segment is a `*` glob, the entries are filtered by plugin (+
///   `--catalog`/`--kind`) keeping every entry whose name [`glob_match`]es the
///   pattern; zero matches → `EntryNotFound` (exit 27, echoing
///   `<plugin>/<name-pattern>`). A literal name defers to the existing
///   single-entry [`set::resolve_target`] (0 → `EntryNotFound`/27, >1 → ambiguous
///   `Usage`/2) — the BYTE-IDENTICAL back-compat path.
///
/// The returned set is deduped on the `(catalog, plugin, kind, name)` identity
/// preserving first-seen order (a plugin matched by two `--plugin` tokens, or an
/// entry matched by an overlapping glob, appears once).
fn resolve_targets(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    id: Option<&str>,
    plugins: &[String],
    catalog: Option<&str>,
    kind: Option<EntryKind>,
) -> Result<Vec<TieredEntry>, TomeError> {
    let all_entries = tiered_entries_for_workspace(conn, workspace_name)?;
    let mut out: Vec<TieredEntry> = Vec::new();

    // ---- `--plugin` selectors ---------------------------------------------
    if !plugins.is_empty() {
        // The candidate universe is the ENABLED plugins of the workspace,
        // mapped to `PluginId` — exactly what `selector::resolve` expects.
        let candidates: Vec<PluginId> = enabled_plugins_for_workspace(conn, workspace_name)?
            .into_iter()
            .map(|(catalog, plugin)| PluginId { catalog, plugin })
            .collect();
        let resolution = selector::resolve(plugins, &candidates, catalog);

        // A selector batch that matched nothing fails loud with the first mapped
        // error (bad glob / ambiguous / not-found), never a silent success.
        if resolution.matched.is_empty() {
            let first = resolution
                .errors
                .into_iter()
                .next()
                .map(selector::SelectorError::into_tome_error)
                // Unreachable: a non-empty `plugins` with no matches always has
                // ≥1 selector error. Defensive usage fallback.
                .unwrap_or_else(|| TomeError::Usage("no plugins selected".to_owned()));
            return Err(first);
        }
        // Any resolved-but-with-errors batch surfaces the first selector error
        // (a mistyped token in a batch is a hard usage failure here, not a
        // forward-progress warning — the target set must be unambiguous before
        // the write batch runs).
        if let Some(err) = resolution.errors.into_iter().next() {
            return Err(err.into_tome_error());
        }

        for pid in &resolution.matched {
            let mut plugin_hits: Vec<TieredEntry> = all_entries
                .iter()
                .filter(|e| e.catalog == pid.catalog && e.plugin == pid.plugin)
                .filter(|e| kind.is_none_or(|k| e.kind == k))
                .cloned()
                .collect();
            if plugin_hits.is_empty() {
                // A resolved plugin with no enabled tierable entries (all agents,
                // disabled, or filtered out by `--kind`) is the same user-facing
                // "nothing to tier" as a missing entry → EntryNotFound (27).
                return Err(TomeError::EntryNotFound {
                    catalog: pid.catalog.clone(),
                    plugin: pid.plugin.clone(),
                    name: "*".to_owned(),
                    kind: kind
                        .map(|k| k.as_str().to_owned())
                        .unwrap_or_else(|| "*".into()),
                });
            }
            for hit in plugin_hits.drain(..) {
                push_unique(&mut out, hit);
            }
        }
        return Ok(out);
    }

    // ---- positional `<plugin>/<name>` -------------------------------------
    // `id` is Some here (clap's XOR guarantees it when `plugins` is empty).
    let id = id.expect("clap XOR: id present when --plugin absent");
    let (plugin, name) = split_id(id)?;

    if is_glob(name) || is_glob(plugin) {
        // A glob in either segment fans out over the plugin's tierable entries.
        let mut hits: Vec<TieredEntry> = all_entries
            .iter()
            .filter(|e| glob_match(plugin, &e.plugin) && glob_match(name, &e.name))
            .filter(|e| catalog.is_none_or(|c| e.catalog == c))
            .filter(|e| kind.is_none_or(|k| e.kind == k))
            .cloned()
            .collect();
        if hits.is_empty() {
            // A zero-match glob is an ERROR, never a silent no-op (mirrors the
            // selector's `NoGlobMatch` policy). Reuse EntryNotFound (27) — the
            // same code a literal miss produces — echoing the pattern.
            return Err(TomeError::EntryNotFound {
                catalog: catalog.unwrap_or("*").to_owned(),
                plugin: plugin.to_owned(),
                name: name.to_owned(),
                kind: kind
                    .map(|k| k.as_str().to_owned())
                    .unwrap_or_else(|| "*".into()),
            });
        }
        for hit in hits.drain(..) {
            push_unique(&mut out, hit);
        }
        return Ok(out);
    }

    // Literal name → the existing single-entry resolver, BYTE-IDENTICAL to the
    // pre-#317 path (0 → EntryNotFound/27, >1 → ambiguous Usage/2).
    let target = set::resolve_target(conn, workspace_name, plugin, name, catalog, kind)?;
    out.push(target);
    Ok(out)
}

/// Order-preserving dedupe on the `(catalog, plugin, kind, name)` identity: only
/// push an entry not already present.
fn push_unique(out: &mut Vec<TieredEntry>, entry: TieredEntry) {
    if !out.iter().any(|e| {
        e.catalog == entry.catalog
            && e.plugin == entry.plugin
            && e.kind == entry.kind
            && e.name == entry.name
    }) {
        out.push(entry);
    }
}
