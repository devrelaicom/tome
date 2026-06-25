//! `tome reindex [<scope>] [--force]`.
//!
//! Explicit re-embedding outside the `tome catalog update` schedule.
//! Used for embedder upgrades (FR-016 recovery path) and integrity recovery.
//! See `contracts/reindex.md`.
//!
//! The scope grammar is:
//!
//! * omitted — every enabled plugin across every registered catalog;
//! * `<catalog>` — every enabled plugin in one catalog;
//! * `<catalog>/<plugin>` — exactly one plugin.

use std::io::Write;
use std::str::FromStr;
use std::time::Instant;

use serde::Serialize;

use crate::cli::ReindexArgs;
use crate::error::TomeError;
use crate::index::skills::ReindexSummary;
use crate::index::{self, OpenOptions, enabled_plugins_for_catalog};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::colour;
use crate::workspace::ResolvedScope;

use crate::commands::plugin::{open_index_for_read, registry_seeds};
use crate::index::meta::{self, MetaKey, ModelIdent};

// NOTE: this module's local `Scope` enum is the reindex *target* (all /
// catalog / plugin). To avoid a name collision with the Phase 3
// `workspace::Scope`, the workspace scope is always referenced as
// `&ResolvedScope` (or `&crate::workspace::Scope`) at function boundaries.

pub fn run(args: ReindexArgs, ws: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let forced = args.force;
    // Derive the telemetry scope structurally from the raw arg (before
    // validation) so a failure during `parse_scope` still carries the right
    // dimension. omitted→All, `<catalog>/<plugin>`→Plugin, `<catalog>`→Catalog.
    let tele_scope = reindex_scope_of(args.scope.as_deref());

    let result = run_inner(args, ws, mode);

    // OUTCOME-bearing: emit on BOTH success and failure. A failed reindex emits
    // `Reindex{outcome:Failed}` here AND the boundary emits `tome.error` — two
    // distinct signals (intentional). One infallible `enqueue`.
    crate::telemetry::enqueue(crate::telemetry::event::Reindex {
        scope: tele_scope,
        forced,
        outcome: if result.is_ok() {
            crate::telemetry::event::Outcome::Ok
        } else {
            crate::telemetry::event::Outcome::Failed
        },
    });

    result
}

/// Structurally map the raw `<scope>` arg to the telemetry
/// [`ReindexScope`](crate::telemetry::event::ReindexScope) — no validation, so
/// it is meaningful even when `parse_scope` later rejects the value.
fn reindex_scope_of(raw: Option<&str>) -> crate::telemetry::event::ReindexScope {
    use crate::telemetry::event::ReindexScope;
    match raw {
        None => ReindexScope::All,
        Some(s) if s.contains('/') => ReindexScope::Plugin,
        Some(_) => ReindexScope::Catalog,
    }
}

fn run_inner(args: ReindexArgs, ws: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    // Phase 12 / US2: load the global config strictly so the embedder resolves
    // remote-vs-bundled, the policy gate compares the right identity, and (on a
    // remote whole-index run) the established dimension can be persisted.
    let cfg = crate::config::load(&paths)?;

    let scope = parse_scope(args.scope.as_deref(), &paths, &ws.scope)?;
    let plugins = resolve_targets(&scope, &paths, &ws.scope)?;

    if plugins.is_empty() {
        // No enabled plugins anywhere in scope: nothing to reindex. Exit 0
        // with a small notice so the user knows this wasn't a silent failure.
        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(out, "Nothing to reindex (no enabled plugins in scope).")?;
        }
        return Ok(());
    }

    // B1: a profile-driven embedder change requires a WHOLE-INDEX re-embed; the
    // GLOBAL `meta` embedder stamp is gated on it. Open one writable handle for
    // the active-embedder read + the (post-commit) stamp. `Scope::All` is the
    // "no catalog/plugin scope" discriminant from `parse_scope` above.
    let policy_conn = {
        let (e_seed, r_seed, s_seed) = registry_seeds();
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: e_seed,
                reranker: r_seed,
                summariser: s_seed,
                profile: None,
            },
        )?
    };
    let whole_index = matches!(scope, Scope::All);
    let configured = meta::active_embedder(&policy_conn)?;
    // Phase 12: the configured identity is the ACTIVE (remote-or-bundled)
    // embedder. The policy gate compares THIS against the stored `meta` stamp,
    // so a remote-embedder switch forces a whole-index re-embed exactly as a
    // profile change does.
    let active_embedder_seed = crate::embedding::embedder_seed(&cfg, configured)?;
    let configured_ident = ModelIdent {
        name: active_embedder_seed.name.clone(),
        version: active_embedder_seed.version.clone(),
    };
    // Refuses a scoped reindex under embedder drift; otherwise returns the
    // effective force flag (args.force || embedder_changed).
    let force = embedder_change_policy(&policy_conn, whole_index, args.force, &configured_ident)?;

    // Phase 12: is the embedder remote? Drives both the embedder construction
    // and (on a whole-index run) the persisted-dimension write.
    let remote_embedding =
        crate::provider::resolve(&cfg, crate::provider::Capability::Embedding)?.is_some();

    let embedder = load_embedder(&cfg, &paths)?;
    let (_e_seed, reranker_seed, summariser_seed) = registry_seeds();
    // `LifecycleDeps.config` is vestigial since the catalog-enrolment
    // migration to the DB — `resolve_plugin_dir` reads `workspace_catalogs`
    // and nothing in the lifecycle consults `config`. Pass an empty default.
    let config = crate::config::Config::default();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &ws.scope,
        config: &config,
        embedder: embedder.as_ref(),
        // Phase 12: stamp `meta` with the ACTIVE (remote-or-bundled) identity.
        embedder_seed: active_embedder_seed.clone(),
        reranker_seed,
        summariser_seed,
        allow_model_download: false,
    };

    let aggregate = execute(&scope, &plugins, &deps, force)?;

    // B1: stamp the GLOBAL `meta` embedder rows ONLY after a WHOLE-INDEX
    // re-embed commits. Never stamp after a partial (scoped) re-embed — the
    // `meta` table is a single global key/value store describing the entire
    // index, and a partial stamp would advertise a dimension the out-of-scope
    // rows do not carry. `force` is true here whenever the embedder changed.
    if whole_index && force {
        stamp_embedder_after_whole_index(&policy_conn, &configured_ident)?;
    }

    // Phase 12 / US2 (FR-015a): on a REMOTE whole-index reindex, persist the
    // active embedder's expected output dimension — `[embedding] dimensions` if
    // the user pinned one (authoritative), else the dimension established from
    // the first successful embed of this run. Written ONLY here (the remote
    // reindex path) and ONLY for a remote embedder; the bundled path NEVER
    // writes the key (NFR-006: a new meta row would change stored artefacts).
    // Gated on `whole_index` so a partial reindex can't stamp a dimension the
    // out-of-scope rows may not share. A run that re-embedded nothing (e.g. an
    // unchanged tree with no `--force`) leaves any prior value untouched.
    if remote_embedding && whole_index {
        let persisted = cfg
            .embedding
            .dimensions
            .map(|d| d as usize)
            .or_else(|| embedder.established_dimension());
        if let Some(dim) = persisted {
            meta::write_embedder_dimension(&policy_conn, dim)?;
        }
    }
    drop(policy_conn);

    // FR-382 + FR-385: regenerate cached summaries only when at least
    // one skill's content_hash changed (added / modified / removed).
    // Reindex of an unchanged tree is a no-op for summarisation —
    // cached summaries stay valid per FR-423.
    if aggregate.any_changes() {
        crate::summarise::regenerate_for_trigger(ws.scope.name(), &paths)?;
    }

    emit(&scope, &aggregate, mode)
}

/// Resolved scope. `Catalog`s and `Plugin`s carry strings rather than
/// references because the underlying `Config` is consumed during dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Catalog(String),
    Plugin(PluginId),
}

impl Scope {
    fn label(&self) -> String {
        match self {
            Scope::All => "all".to_owned(),
            Scope::Catalog(c) => c.clone(),
            Scope::Plugin(id) => id.to_string(),
        }
    }
}

fn parse_scope(
    raw: Option<&str>,
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
) -> Result<Scope, TomeError> {
    let Some(s) = raw else {
        return Ok(Scope::All);
    };
    // FF2: catalog existence is checked against the `workspace_catalogs` DB
    // enrolment, not `config.toml [catalogs]` (never written in production →
    // every scoped reindex failed with exit 3 on a fresh install). The
    // exit-code contract is unchanged: unknown catalog → CatalogNotFound (3);
    // known catalog + unknown plugin → PluginNotFound (20).
    let conn = open_index_for_read(paths, ws_scope)?;
    let workspace_name = ws_scope.name().as_str();
    if s.contains('/') {
        let id = PluginId::from_str(s)
            .map_err(|e| TomeError::Usage(format!("invalid plugin id `{s}`: {e}")))?;
        if index::workspace_catalogs::find(&conn, workspace_name, &id.catalog)?.is_none() {
            return Err(TomeError::CatalogNotFound(id.catalog));
        }
        // Plugin existence is enforced at the reindex step (PluginNotFound /
        // PluginManifestParseError surface from lifecycle::reindex_plugin). We
        // also cross-check the index here so a typo on a plugin that has
        // never been enabled exits 20 immediately rather than emerging from
        // the lifecycle's resolver.
        let enabled = read_enabled_plugins(paths, ws_scope, &id.catalog)?;
        if !enabled.iter().any(|p| p == &id.plugin) {
            return Err(TomeError::PluginNotFound(id.to_string()));
        }
        Ok(Scope::Plugin(id))
    } else {
        if index::workspace_catalogs::find(&conn, workspace_name, s)?.is_none() {
            return Err(TomeError::CatalogNotFound(s.to_owned()));
        }
        Ok(Scope::Catalog(s.to_owned()))
    }
}

fn resolve_targets(
    scope: &Scope,
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
) -> Result<Vec<PluginId>, TomeError> {
    match scope {
        Scope::Plugin(id) => Ok(vec![id.clone()]),
        Scope::Catalog(c) => {
            let names = read_enabled_plugins(paths, ws_scope, c)?;
            Ok(names
                .into_iter()
                .map(|p| PluginId {
                    catalog: c.clone(),
                    plugin: p,
                })
                .collect())
        }
        Scope::All => {
            // Walk every catalog row in the index, group by catalog. We do
            // this once via a single SQL query rather than iterating
            // catalogs and re-opening the connection. Scope: the resolved
            // workspace.
            let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
            let workspace_name = ws_scope.name().as_str();
            let conn = index::open(
                &paths.index_db,
                &OpenOptions {
                    embedder: embedder_seed,
                    reranker: reranker_seed,
                    summariser: summariser_seed,
                    profile: None,
                },
            )?;
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT s.catalog, s.plugin
                     FROM skills AS s
                     JOIN workspace_skills AS ws ON ws.skill_id = s.id
                     JOIN workspaces       AS w  ON w.id = ws.workspace_id
                     WHERE w.name = ?1
                     ORDER BY s.catalog, s.plugin",
                )
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!("prepare all-scope: {e}"))
                })?;
            let rows = stmt
                .query_map(rusqlite::params![workspace_name], |row| {
                    let c: String = row.get(0)?;
                    let p: String = row.get(1)?;
                    Ok(PluginId {
                        catalog: c,
                        plugin: p,
                    })
                })
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!("query all-scope: {e}"))
                })?;
            rows.collect::<Result<_, _>>().map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!("collect all-scope: {e}"))
            })
        }
    }
}

fn read_enabled_plugins(
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
    catalog: &str,
) -> Result<Vec<String>, TomeError> {
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
            profile: None,
        },
    )?;
    enabled_plugins_for_catalog(&conn, ws_scope.name().as_str(), catalog)
}

fn load_embedder(
    cfg: &crate::config::Config,
    paths: &Paths,
) -> Result<Box<dyn crate::embedding::Embedder>, TomeError> {
    // B4 / Phase 12: build the ACTIVE (remote-or-bundled) embedder. Reindex is
    // the sole drift resolver, so it loads whatever the active config now
    // selects and (for a whole-index run) re-embeds + restamps to match. On the
    // remote path the validator's expected dimension is seeded from
    // `[embedding] dimensions` (authoritative) — when unset, the embedder
    // ESTABLISHES the dimension from its first successful embed of this run, and
    // `run_inner` persists it to `meta.embedder_dimension` afterwards.
    let (e_seed, r_seed, s_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e_seed,
            reranker: r_seed,
            summariser: s_seed,
            profile: None,
        },
    )?;
    let entry = meta::active_embedder(&conn)?;
    // A reindex deliberately does NOT seed from any persisted dimension — it is
    // the path that ESTABLISHES the dimension. Passing `None` lets the
    // `[embedding] dimensions` knob (read inside `build_embedder`) win when set,
    // and otherwise the first embed establishes the run dimension.
    crate::embedding::build_embedder(cfg, paths, entry, None)
}

/// B1 policy gate for `tome reindex`. Decides whether the embedder changed
/// (configured active-profile embedder vs the GLOBAL `meta` stamp) and, if so:
///
/// * a SCOPED run (`whole_index == false`) is REFUSED with
///   [`TomeError::ReindexScopedEmbedderChange`] (exit 47) — re-embedding only
///   some plugins while stamping the global `meta` leaves out-of-scope vectors
///   at the old dimension (the mixed-dimension corruption);
/// * a WHOLE-INDEX run forces a full re-embed so every row is rewritten at the
///   new dimension.
///
/// Returns the EFFECTIVE force flag: `args_force || embedder_changed`. When the
/// embedder did not change the caller's own `--force` is passed through
/// unchanged. Exposed (`pub`) so the model-tiering regression test can drive
/// the exact gate `run_inner` uses without spawning the binary.
pub fn embedder_change_policy(
    conn: &rusqlite::Connection,
    whole_index: bool,
    args_force: bool,
    configured_embedder: &ModelIdent,
) -> Result<bool, TomeError> {
    let stored_name = meta::read(conn, MetaKey::EmbedderName)?.unwrap_or_default();
    let stored_ver = meta::read(conn, MetaKey::EmbedderVersion)?.unwrap_or_default();
    let embedder_changed =
        stored_name != configured_embedder.name || stored_ver != configured_embedder.version;

    if embedder_changed && !whole_index {
        return Err(TomeError::ReindexScopedEmbedderChange {
            stored: stored_name,
            configured: configured_embedder.name.clone(),
        });
    }
    // `skills.rs` SKIPs unchanged-hash skills unless `force`, so an embedder
    // change MUST force or the new-dimension vectors never get written.
    Ok(args_force || embedder_changed)
}

/// B1: stamp the GLOBAL `meta` embedder rows to the configured identity AFTER a
/// whole-index re-embed has committed. Callers MUST NOT invoke this after a
/// partial (scoped) re-embed — see [`embedder_change_policy`]. Exposed for the
/// regression test for the same reason the policy gate is.
pub fn stamp_embedder_after_whole_index(
    conn: &rusqlite::Connection,
    configured_embedder: &ModelIdent,
) -> Result<(), TomeError> {
    meta::write(conn, MetaKey::EmbedderName, &configured_embedder.name)?;
    meta::write(conn, MetaKey::EmbedderVersion, &configured_embedder.version)?;
    Ok(())
}

/// Aggregated outcome of one `tome reindex` invocation.
#[derive(Debug, Clone, Default)]
pub struct ReindexAggregate {
    pub plugins_visited: u32,
    pub skills_checked: u32,
    pub skills_re_embedded: u32,
    pub skills_unchanged: u32,
    /// Number of skills whose row was DELETE'd because the on-disk
    /// SKILL.md is gone. Counted alongside `added` / `modified` when
    /// the summariser-trigger gate (FR-382) decides whether to fire.
    pub skills_removed: u32,
    pub duration_ms: u64,
}

impl ReindexAggregate {
    /// `true` iff any skill changed identity (added / modified /
    /// removed) — the FR-382 gate for triggering summary regeneration
    /// on reindex.
    pub fn any_changes(&self) -> bool {
        self.skills_re_embedded > 0 || self.skills_removed > 0
    }
}

/// Execute a reindex against a pre-built `LifecycleDeps`. Loops over every
/// plugin in `plugins`, calling `lifecycle::reindex_plugin` per plugin.
/// `force` is propagated to each call.
///
/// Exposed for tests that want to drive the library API with `StubEmbedder`
/// rather than spawning the CLI binary (which loads `FastembedEmbedder`).
pub fn execute(
    _scope: &Scope,
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
) -> Result<ReindexAggregate, TomeError> {
    let started = Instant::now();
    let mut aggregate = ReindexAggregate::default();
    for id in plugins {
        let outcome = lifecycle::reindex_plugin(id, deps, force)?;
        aggregate.plugins_visited = aggregate.plugins_visited.saturating_add(1);
        let s: ReindexSummary = outcome.summary;
        let checked = s
            .added
            .saturating_add(s.modified)
            .saturating_add(s.unchanged);
        aggregate.skills_checked = aggregate.skills_checked.saturating_add(checked);
        aggregate.skills_re_embedded = aggregate
            .skills_re_embedded
            .saturating_add(s.added.saturating_add(s.modified));
        aggregate.skills_unchanged = aggregate.skills_unchanged.saturating_add(s.unchanged);
        aggregate.skills_removed = aggregate.skills_removed.saturating_add(s.removed);
    }
    aggregate.duration_ms = duration_ms(started);
    Ok(aggregate)
}

fn duration_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn emit(scope: &Scope, aggregate: &ReindexAggregate, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(scope, aggregate),
        Mode::Json => emit_json(scope, aggregate),
    }
}

fn emit_human(scope: &Scope, agg: &ReindexAggregate) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Reindexed {} ({} plugin{}, {} skill{} checked)",
        scope.label(),
        agg.plugins_visited,
        if agg.plugins_visited == 1 { "" } else { "s" },
        agg.skills_checked,
        if agg.skills_checked == 1 { "" } else { "s" },
    )?;
    writeln!(
        out,
        "  {} Re-embedded: {}",
        colour::success("✓"),
        agg.skills_re_embedded
    )?;
    writeln!(out, "    Unchanged:  {}", agg.skills_unchanged)?;
    Ok(())
}

#[derive(Serialize)]
struct ReindexRecord<'a> {
    scope: String,
    plugins_visited: u32,
    skills_checked: u32,
    skills_re_embedded: u32,
    skills_unchanged: u32,
    duration_ms: u64,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<&'a ()>,
}

fn emit_json(scope: &Scope, agg: &ReindexAggregate) -> Result<(), TomeError> {
    let record = ReindexRecord {
        scope: scope.label(),
        plugins_visited: agg.plugins_visited,
        skills_checked: agg.skills_checked,
        skills_re_embedded: agg.skills_re_embedded,
        skills_unchanged: agg.skills_unchanged,
        duration_ms: agg.duration_ms,
        _phantom: std::marker::PhantomData,
    };
    write_json(&record)
}

/// Helper for tests: take an already-built scope, plugin list, and deps, and
/// drive `execute` directly. Re-exports the same function with no scope
/// validation so tests can scope by plugin without registering a catalog.
pub fn run_with_deps(
    scope: Scope,
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
    mode: Mode,
) -> Result<ReindexAggregate, TomeError> {
    let aggregate = execute(&scope, plugins, deps, force)?;
    emit(&scope, &aggregate, mode)?;
    Ok(aggregate)
}
