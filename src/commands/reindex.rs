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

use crate::catalog::store;
use crate::cli::ReindexArgs;
use crate::config::Config;
use crate::embedding::fastembed::FastembedEmbedder;
use crate::error::TomeError;
use crate::index::skills::ReindexSummary;
use crate::index::{self, OpenOptions, enabled_plugins_for_catalog};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::colour;

use crate::commands::plugin::{embedder_entry, registry_seeds};

pub fn run(args: ReindexArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let config = store::load(&paths.config_file)?;

    let scope = parse_scope(args.scope.as_deref(), &config, &paths)?;
    let plugins = resolve_targets(&scope, &paths)?;

    if plugins.is_empty() {
        // No enabled plugins anywhere in scope: nothing to reindex. Exit 0
        // with a small notice so the user knows this wasn't a silent failure.
        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(out, "Nothing to reindex (no enabled plugins in scope).")?;
        }
        return Ok(());
    }

    let embedder = load_embedder(&paths)?;
    let (embedder_seed, reranker_seed) = registry_seeds();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed,
        reranker_seed,
        allow_model_download: false,
    };

    let aggregate = execute(&scope, &plugins, &deps, args.force)?;
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

fn parse_scope(raw: Option<&str>, config: &Config, paths: &Paths) -> Result<Scope, TomeError> {
    let Some(s) = raw else {
        return Ok(Scope::All);
    };
    if s.contains('/') {
        let id = PluginId::from_str(s)
            .map_err(|e| TomeError::Usage(format!("invalid plugin id `{s}`: {e}")))?;
        if !config.catalogs.contains_key(&id.catalog) {
            return Err(TomeError::CatalogNotFound(id.catalog));
        }
        // Plugin existence is enforced at the reindex step (PluginNotFound /
        // PluginManifestParseError surface from lifecycle::reindex_plugin). We
        // also cross-check the index here so a typo on a plugin that has
        // never been enabled exits 20 immediately rather than emerging from
        // the lifecycle's resolver.
        let enabled = read_enabled_plugins(paths, &id.catalog)?;
        if !enabled.iter().any(|p| p == &id.plugin) {
            return Err(TomeError::PluginNotFound(id.to_string()));
        }
        Ok(Scope::Plugin(id))
    } else {
        if !config.catalogs.contains_key(s) {
            return Err(TomeError::CatalogNotFound(s.to_owned()));
        }
        Ok(Scope::Catalog(s.to_owned()))
    }
}

fn resolve_targets(scope: &Scope, paths: &Paths) -> Result<Vec<PluginId>, TomeError> {
    match scope {
        Scope::Plugin(id) => Ok(vec![id.clone()]),
        Scope::Catalog(c) => {
            let names = read_enabled_plugins(paths, c)?;
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
            // catalogs and re-opening the connection.
            let (embedder_seed, reranker_seed) = registry_seeds();
            let conn = index::open(
                &paths.index_db,
                &OpenOptions {
                    embedder: embedder_seed,
                    reranker: reranker_seed,
                },
            )?;
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT catalog, plugin FROM skills
                     WHERE enabled = 1
                     ORDER BY catalog, plugin",
                )
                .map_err(|e| {
                    TomeError::IndexIntegrityCheckFailure(format!("prepare all-scope: {e}"))
                })?;
            let rows = stmt
                .query_map([], |row| {
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

fn read_enabled_plugins(paths: &Paths, catalog: &str) -> Result<Vec<String>, TomeError> {
    let (embedder_seed, reranker_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )?;
    enabled_plugins_for_catalog(&conn, catalog)
}

fn load_embedder(paths: &Paths) -> Result<FastembedEmbedder, TomeError> {
    let entry = embedder_entry();
    let dir = paths.model_path(entry.name)?;
    FastembedEmbedder::load(entry, &dir)
}

/// Aggregated outcome of one `tome reindex` invocation.
#[derive(Debug, Clone, Default)]
pub struct ReindexAggregate {
    pub plugins_visited: u32,
    pub skills_checked: u32,
    pub skills_re_embedded: u32,
    pub skills_unchanged: u32,
    pub duration_ms: u64,
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
