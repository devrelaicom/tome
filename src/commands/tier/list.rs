//! `tome tier list` — every enabled skill/command grouped by routing tier.

use std::io::Write;

use serde::Serialize;

use crate::cli::TierListArgs;
use crate::error::TomeError;
use crate::index::skills::tiered_entries_for_workspace;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

#[derive(Serialize)]
struct ListRecord<'a> {
    catalog: &'a str,
    plugin: &'a str,
    name: &'a str,
    kind: &'a str,
    tier: u8,
}

pub fn run(_args: TierListArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let ws = scope.scope.name();

    // Read-only — `list` never mutates and so never takes the advisory lock.
    // A genuinely absent index DB means "no enabled entries".
    let entries = if paths.index_db.exists() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        tiered_entries_for_workspace(&conn, ws.as_str())?
    } else {
        Vec::new()
    };

    match mode {
        Mode::Json => {
            // NDJSON: one record per enabled entry, in the byte-stable
            // (tier, catalog, plugin, name) order the query already imposes.
            for e in &entries {
                output::write_json(&ListRecord {
                    catalog: &e.catalog,
                    plugin: &e.plugin,
                    name: &e.name,
                    kind: e.kind.as_str(),
                    tier: e.tier,
                })?;
            }
            Ok(())
        }
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            // `entries` is sorted by tier first, so a single pass that prints a
            // header on each tier transition keeps empty tiers out.
            let mut current: Option<u8> = None;
            for e in &entries {
                if current != Some(e.tier) {
                    writeln!(out, "Tier {}:", e.tier)?;
                    current = Some(e.tier);
                }
                writeln!(
                    out,
                    "  {}/{} ({})  {}",
                    e.plugin,
                    e.name,
                    e.kind.as_str(),
                    e.description,
                )?;
            }
            if entries.is_empty() {
                writeln!(out, "No enabled skills or commands in this workspace.")?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_record_json_shape_is_pinned() {
        // Byte-stable wire-shape pin: field order == struct declaration order
        // (serde_json preserve_order feature is active crate-wide).
        let r = ListRecord {
            catalog: "cat",
            plugin: "plug",
            name: "my-skill",
            kind: "skill",
            tier: 2,
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"catalog":"cat","plugin":"plug","name":"my-skill","kind":"skill","tier":2}"#,
        );
    }
}
