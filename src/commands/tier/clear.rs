//! `tome tier clear <plugin>/<name>` — reset an entry's tier to the default (3).

use std::io::Write;

use crate::cli::TierClearArgs;
use crate::error::TomeError;
use crate::index::skills::{TieredEntry, set_tier_for_entry};
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::set::{TierRecord, kind_of, resolve_target};

/// The default routing tier — searchable on demand (Tier 3).
const DEFAULT_TIER: u8 = 3;

pub fn run(args: TierClearArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let (plugin, name) = super::split_id(&args.id)?;
    let paths = Paths::resolve()?;
    let ws = scope.scope.name();

    // Same writable-open-under-lock discipline as `set` — `clear` is an UPDATE.
    // NOTE: `index::open` is called BEFORE `acquire_lock` so that schema-version
    // checks and migrations can run on the connection; the first actual write
    // (`set_tier_for_entry`) runs only after `acquire_lock` is held.
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
            DEFAULT_TIER,
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

    crate::harness::routing::write_workspace_rules(&paths, ws)?;

    let record = TierRecord {
        catalog: target.catalog,
        plugin: target.plugin,
        name: target.name,
        kind: target.kind.as_str(),
        tier: DEFAULT_TIER,
    };

    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "{}/{} ({}) → tier {} (default)",
                record.plugin, record.name, record.kind, record.tier,
            )?;
            Ok(())
        }
        Mode::Json => output::write_json(&record),
    }
}
