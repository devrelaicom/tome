//! `tome tier set <plugin>/<name>|--plugin <sel> <1|2|3>`.
//!
//! Issue #317 widened the single-entry `set` to a bulk retiering command: the
//! positional id may carry a `*` name-glob (`<plugin>/*`, `<plugin>/foo-*`), and
//! a repeatable `--plugin <catalog/plugin>` selector fans out across a plugin's
//! enabled tierable entries. Selection is resolved ONCE into a deduped target
//! set (`super::resolve_targets`), then applied under a single advisory-lock
//! batch. A single literal `<plugin>/<name>` id behaves EXACTLY as before: one
//! JSON record, the same human line, the same exit codes, one RULES.md regen.

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
    // Split the two clap-optional trailing positionals into (id, tier). The
    // positional/`--plugin` XOR and the always-required tier are validated here
    // (usage exit 2) so the tier can be the SOLE positional under `--plugin`
    // while the single-id form `tier set <plugin>/<name> <tier>` is unchanged.
    let (id, tier) = parse_positionals(&args.positionals, !args.plugin.is_empty())?;

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

    // Resolve the target set ONCE (positional glob/literal or `--plugin` fan-out)
    // inside the lock so the snapshot the writes run against can't shift.
    let apply = (|| -> Result<(Vec<TieredEntry>, Option<TomeError>), TomeError> {
        let targets = super::resolve_targets(
            &conn,
            ws.as_str(),
            id.as_deref(),
            &args.plugin,
            args.catalog.as_deref(),
            args.kind.map(kind_of),
        )?;

        // Per-entry forward-progress loop (mirrors `harness use`): a per-entry
        // UPDATE failure is warned + skipped, the FIRST error captured and
        // surfaced after emitting the successes.
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
                tier,
            ) {
                Ok(()) => applied.push(target),
                Err(e) => {
                    tracing::warn!(
                        entry = %format!("{}/{}", target.plugin, target.name),
                        error = %e,
                        "tier set: entry failed; continuing",
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

    // Refresh the workspace's RULES.md + every bound project's mirror ONCE after
    // the whole batch so the new tiers take effect immediately. Cheap (no LLM).
    crate::harness::routing::write_workspace_rules(&paths, ws)?;

    // NDJSON: one record per affected entry. A single affected entry ⇒ exactly
    // one object ⇒ byte-identical to the pre-#317 record (the pinned shape).
    for target in &applied {
        let record = TierRecord {
            catalog: target.catalog.clone(),
            plugin: target.plugin.clone(),
            name: target.name.clone(),
            kind: target.kind.as_str(),
            tier,
        };
        emit(&record, mode)?;
    }

    // Forward-progress: successes emitted, now surface the first failure's code.
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Split the clap-optional trailing positionals into `(id, tier)`, validating
/// the `id`/`--plugin` XOR and the always-required, 1..=3 tier at runtime (all
/// as `Usage`, exit 2). `has_plugin` is whether `--plugin` was passed.
///
/// Layouts:
/// * `--plugin` absent: `[<id>, <tier>]` — both required. Fewer than 2 → the
///   missing selection source or tier; the id form needs both.
/// * `--plugin` present: `[<tier>]` — the id positional is forbidden (XOR), so
///   the sole positional is the tier. Two positionals with `--plugin` means an
///   id was also given → the XOR violation.
///
/// The tier is always the LAST positional and is range-checked here (the clap
/// value_parser can't run on a `Vec<String>` positional).
fn parse_positionals(
    positionals: &[String],
    has_plugin: bool,
) -> Result<(Option<String>, u8), TomeError> {
    let parse_tier = |s: &str| -> Result<u8, TomeError> {
        match s.parse::<u8>() {
            Ok(t @ 1..=3) => Ok(t),
            _ => Err(TomeError::Usage(format!(
                "invalid tier `{s}` (expected 1, 2, or 3)"
            ))),
        }
    };

    if has_plugin {
        match positionals {
            // `--plugin sel <tier>` — the id positional is forbidden.
            [tier] => Ok((None, parse_tier(tier)?)),
            [] => Err(TomeError::Usage(
                "missing required tier argument (expected 1, 2, or 3)".to_owned(),
            )),
            // Two positionals with `--plugin` = an id was also given → XOR.
            _ => Err(TomeError::Usage(
                "the entry id positional cannot be used with `--plugin`".to_owned(),
            )),
        }
    } else {
        match positionals {
            // `tier set <plugin>/<name> <tier>` — the single-id form.
            [id, tier] => Ok((Some(id.clone()), parse_tier(tier)?)),
            // A lone positional with no `--plugin` is missing either the id or
            // the tier; the id form needs both. (A bare tier without a selection
            // source is exactly the "neither id nor --plugin" case.)
            [_] => Err(TomeError::Usage(
                "provide an entry id and a tier (`tome tier set <plugin>/<name> <1|2|3>`), \
                 or select plugins with `--plugin <sel> <1|2|3>`"
                    .to_owned(),
            )),
            _ => Err(TomeError::Usage(
                "no selection source: pass an entry id `<plugin>/<name>` or `--plugin <sel>`"
                    .to_owned(),
            )),
        }
    }
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
        TierKindArg::Agent => EntryKind::Agent,
    }
}

/// The reverse of [`kind_of`]: an [`EntryKind`] → its CLI [`TierKindArg`]
/// selector. The MCP `search_skills` tool (#320) accepts a single-value
/// `kind` filter typed as `EntryKind` and threads it through the shared
/// `QueryArgs.kind: Vec<TierKindArg>` slot, so it needs this symmetric
/// direction. Kept beside `kind_of` so the two mappings stay in lockstep —
/// a new `EntryKind` / `TierKindArg` variant fails to compile until both
/// arms are added.
pub(crate) fn tierkind_of(kind: EntryKind) -> TierKindArg {
    match kind {
        EntryKind::Skill => TierKindArg::Skill,
        EntryKind::Command => TierKindArg::Command,
        EntryKind::Agent => TierKindArg::Agent,
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

    fn s(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parse_positionals_single_id_form() {
        let (id, tier) = parse_positionals(&s(&["plug/name", "2"]), false).expect("ok");
        assert_eq!(id.as_deref(), Some("plug/name"));
        assert_eq!(tier, 2);
    }

    #[test]
    fn parse_positionals_plugin_form_sole_positional_is_tier() {
        // With `--plugin` present the sole positional is the tier, NOT the id.
        let (id, tier) = parse_positionals(&s(&["3"]), true).expect("ok");
        assert_eq!(id, None);
        assert_eq!(tier, 3);
    }

    #[test]
    fn parse_positionals_out_of_range_is_usage() {
        for (args, has_plugin) in [(s(&["plug/name", "4"]), false), (s(&["0"]), true)] {
            let err = parse_positionals(&args, has_plugin).expect_err("out of range");
            assert_eq!(err.exit_code(), 2, "out-of-range tier → usage 2");
        }
    }

    #[test]
    fn parse_positionals_missing_tier_is_usage() {
        // id form with only the id (no tier).
        let err = parse_positionals(&s(&["plug/name"]), false).expect_err("missing tier");
        assert_eq!(err.exit_code(), 2);
        // --plugin form with no positional at all.
        let err = parse_positionals(&s(&[]), true).expect_err("missing tier");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn parse_positionals_both_id_and_plugin_is_usage() {
        // `--plugin` present AND two positionals means an id was also given.
        let err = parse_positionals(&s(&["plug/name", "2"]), true).expect_err("both");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn parse_positionals_no_selection_is_usage() {
        // No `--plugin` and no positionals at all.
        let err = parse_positionals(&s(&[]), false).expect_err("neither");
        assert_eq!(err.exit_code(), 2);
    }
}
