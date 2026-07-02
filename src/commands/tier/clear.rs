//! `tome tier clear <plugin>/<name>|--plugin <sel>|--all` — reset tier(s) to the
//! default (3).
//!
//! Issue #317 widened the single-entry `clear` with a `*` name-glob on the
//! positional id, a repeatable `--plugin` selector, and `--all` (reset every
//! enabled tierable entry in the workspace). Selection resolves ONCE into a
//! deduped target set (or the whole-workspace set for `--all`), then applies
//! under a single advisory-lock batch. A single literal `<plugin>/<name>` id
//! behaves EXACTLY as before: one JSON record, the same human line, the same
//! exit codes, one RULES.md regen.

use std::io::Write;

use crate::cli::TierClearArgs;
use crate::error::TomeError;
use crate::index::skills::{TieredEntry, reset_all_tiers_for_workspace, set_tier_for_entry};
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::set::{TierRecord, kind_of};

/// The default routing tier — searchable on demand (Tier 3).
const DEFAULT_TIER: u8 = 3;

pub fn run(args: TierClearArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let ws = scope.scope.name();

    // Same writable-open-under-lock discipline as `set` — `clear` is an UPDATE.
    // NOTE: `index::open` is called BEFORE `acquire_lock` so that schema-version
    // checks and migrations can run on the connection; the first actual write
    // runs only after `acquire_lock` is held.
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

    let apply = (|| -> Result<(Vec<TieredEntry>, Option<TomeError>), TomeError> {
        if args.all {
            // Whole-workspace reset: one atomic UPDATE, no per-entry loop. The
            // returned rows (already at the default tier) drive the emit; there
            // is no per-entry failure path, so `first_error` is always None.
            let affected = reset_all_tiers_for_workspace(&conn, ws.as_str())?;
            return Ok((affected, None));
        }

        // Positional glob/literal or `--plugin` fan-out → deduped target set.
        let targets = super::resolve_targets(
            &conn,
            ws.as_str(),
            args.id.as_deref(),
            &args.plugin,
            args.catalog.as_deref(),
            args.kind.map(kind_of),
        )?;

        // Per-entry forward-progress loop (mirrors `set`): reset each target to
        // the default tier, warn+skip on a per-entry failure, capture the first.
        let mut applied: Vec<TieredEntry> = Vec::with_capacity(targets.len());
        let mut first_error: Option<TomeError> = None;
        for target in targets {
            match set_tier_for_entry(
                &conn,
                ws.as_str(),
                &target.catalog,
                &target.plugin,
                &target.kind,
                &target.name,
                DEFAULT_TIER,
            ) {
                Ok(()) => applied.push(target),
                Err(e) => {
                    tracing::warn!(
                        entry = %format!("{}/{}", target.plugin, target.name),
                        error = %e,
                        "tier clear: entry failed; continuing",
                    );
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        Ok((applied, first_error))
    })();

    let (applied, first_error) = match apply {
        Ok(t) => {
            lock.release()?;
            t
        }
        Err(e) => {
            drop(lock);
            return Err(e);
        }
    };

    // Refresh RULES.md ONCE after the whole batch.
    crate::harness::routing::write_workspace_rules(&paths, ws)?;

    // NDJSON: one record per affected entry. A single affected entry ⇒ exactly
    // one object ⇒ byte-identical to the pre-#317 record.
    for target in &applied {
        let record = TierRecord {
            catalog: target.catalog.clone(),
            plugin: target.plugin.clone(),
            name: target.name.clone(),
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
            }
            Mode::Json => output::write_json(&record)?,
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
