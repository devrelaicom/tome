//! `tome tier set <plugin>/<name> <1|2|3>`.

use std::io::Write;

use serde::Serialize;

use crate::cli::{TierKindArg, TierSetArgs};
use crate::error::TomeError;
use crate::index::skills::{TieredEntry, set_tier_for_entry, tiered_entries_for_workspace};
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::plugin::identity::EntryKind;
use crate::workspace::ResolvedScope;

pub fn run(args: TierSetArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let (plugin, name) = super::split_id(&args.id)?;
    let paths = Paths::resolve()?;
    let ws = scope.scope.name();

    // The UPDATE is a mutation, so it MUST run under the advisory write lock on
    // a writable connection (FR-040). Mirror the lifecycle write commands:
    // open the central DB read-write via `index::open`, then take `index.lock`.
    let (embedder, reranker, summariser) = crate::commands::plugin::registry_seeds();
    let conn = crate::index::open(
        &paths.index_db,
        &crate::index::OpenOptions {
            embedder,
            reranker,
            summariser,
            profile: None,
        },
    )?;
    let lock = crate::index::acquire_lock(&paths.index_lock)?;

    let result = (|| -> Result<TieredEntry, TomeError> {
        let target = resolve_target(
            &conn,
            ws.as_str(),
            plugin,
            name,
            args.catalog.as_deref(),
            args.kind.map(kind_of),
        )?;
        set_tier_for_entry(
            &conn,
            ws.as_str(),
            &target.catalog,
            &target.plugin,
            &target.kind,
            &target.name,
            args.tier,
        )?;
        Ok(target)
    })();

    let target = match result {
        Ok(t) => {
            lock.release()?;
            t
        }
        Err(e) => {
            drop(lock);
            return Err(e);
        }
    };

    // Refresh the workspace's RULES.md + every bound project's mirror so the
    // new tier takes effect immediately. Cheap (no LLM).
    crate::harness::routing::write_workspace_rules(&paths, ws)?;

    let record = TierRecord {
        catalog: target.catalog,
        plugin: target.plugin,
        name: target.name,
        kind: target.kind.as_str(),
        tier: args.tier,
    };
    emit(&record, mode)
}

/// Resolve a `(plugin, name)` (+ optional catalog / kind disambiguators) to a
/// single enabled, tierable entry in `workspace_name`.
///
/// Shared by `set` and `clear`. Resolution reads the same set the routing
/// directive sees (`tiered_entries_for_workspace`, which excludes agents).
/// 0 matches → `EntryNotFound` (exit 27); >1 → `Usage` (exit 2) nudging the
/// caller to pass `--catalog` and/or `--kind`.
pub(crate) fn resolve_target(
    conn: &rusqlite::Connection,
    workspace_name: &str,
    plugin: &str,
    name: &str,
    catalog: Option<&str>,
    kind: Option<EntryKind>,
) -> Result<TieredEntry, TomeError> {
    let mut matches: Vec<TieredEntry> = tiered_entries_for_workspace(conn, workspace_name)?
        .into_iter()
        .filter(|e| e.plugin == plugin && e.name == name)
        .filter(|e| catalog.is_none_or(|c| e.catalog == c))
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .collect();

    match matches.len() {
        0 => Err(TomeError::EntryNotFound {
            catalog: catalog.unwrap_or("*").to_owned(),
            plugin: plugin.to_owned(),
            name: name.to_owned(),
            kind: kind
                .map(|k| k.as_str().to_owned())
                .unwrap_or_else(|| "*".into()),
        }),
        1 => Ok(matches.pop().expect("len == 1")),
        _ => Err(TomeError::Usage(format!(
            "ambiguous entry `{plugin}/{name}` matches {} enabled entries; \
             pass --catalog and/or --kind to disambiguate",
            matches.len()
        ))),
    }
}

pub(crate) fn kind_of(arg: TierKindArg) -> EntryKind {
    match arg {
        TierKindArg::Skill => EntryKind::Skill,
        TierKindArg::Command => EntryKind::Command,
    }
}

/// Shared JSON / human emit record for `set` and `clear`.
#[derive(Serialize)]
pub(crate) struct TierRecord {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: &'static str,
    pub tier: u8,
}

// NOTE: `index::open` is called BEFORE `acquire_lock` so that schema-version
// checks and migrations can run on the connection; the first actual write
// (`set_tier_for_entry`) runs only after `acquire_lock` is held.

pub(crate) fn emit(record: &TierRecord, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "{}/{} ({}) → tier {}",
                record.plugin, record.name, record.kind, record.tier,
            )?;
            Ok(())
        }
        Mode::Json => output::write_json(record),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_record_json_shape_is_pinned() {
        // Byte-stable wire-shape pin: field order == struct declaration order
        // (serde_json preserve_order feature is active crate-wide).
        let r = TierRecord {
            catalog: "cat".into(),
            plugin: "plug".into(),
            name: "my-skill".into(),
            kind: "skill",
            tier: 1,
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"catalog":"cat","plugin":"plug","name":"my-skill","kind":"skill","tier":1}"#,
        );
    }
}
