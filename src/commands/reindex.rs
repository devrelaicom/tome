//! `tome reindex [<scope>...] [--catalog <name>...] [--plugin <id>...] [--force]`.
//!
//! Explicit re-embedding outside the `tome catalog update` schedule.
//! Used for embedder upgrades (FR-016 recovery path) and integrity recovery.
//! See `contracts/reindex.md`.
//!
//! Selection grammar (issue #316 — widened from a single optional slash-string):
//!
//! * NOTHING (no positional scopes AND no `--catalog` AND no `--plugin`) — the
//!   WHOLE-INDEX form: every enabled plugin across every enrolled catalog. This
//!   is the ONLY form that restamps the global embedder identity + dimension and
//!   is permitted under embedder drift.
//! * a positional token WITHOUT `/` — a whole `<catalog>` (every enabled plugin
//!   in it); a `*` glob matches enrolled catalog NAMES.
//! * a positional token WITH `/` — a `<catalog>/<plugin>`; a `*` glob in either
//!   half matches against the enabled-plugin set.
//! * `--catalog <name>` (repeatable) — same as a bare positional token; a `*`
//!   glob matches enrolled catalog names.
//! * `--plugin <catalog>/<plugin>` (repeatable) — same as a slash positional; a
//!   `*` glob is allowed in the plugin segment.
//!
//! ANY explicit selection (even one that happens to cover every plugin) is a
//! NON-whole-index run: it never restamps `meta`, and it is REFUSED under
//! embedder drift with [`TomeError::ReindexScopedEmbedderChange`] (exit 47).
//! Reindexing only some plugins while advertising a global dimension is the
//! mixed-dimension corruption the policy prevents.

use std::io::Write;
use std::str::FromStr;
use std::time::Instant;

use serde::Serialize;

use crate::cli::ReindexArgs;
use crate::error::TomeError;
use crate::index::skills::ReindexSummary;
use crate::index::{self, OpenOptions};
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::plugin::selector::{glob_match, is_glob};
use crate::presentation::colour;
use crate::workspace::ResolvedScope;

use crate::commands::plugin::{open_index_for_read, registry_seeds};
use crate::index::meta::{self, MetaKey, ModelIdent};

// NOTE: this module's local `Scope` enum is the reindex *target* (all /
// catalog / plugin). To avoid a name collision with the Phase 3
// `workspace::Scope`, the workspace scope is always referenced as
// `&ResolvedScope` (or `&crate::workspace::Scope`) at function boundaries.
// Since issue #316 `Scope` is retained ONLY as the library-test entry-point
// shape (`run_with_deps`); the production path resolves a `Selection` instead.

pub fn run(args: ReindexArgs, ws: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let forced = args.force;
    // Derive the telemetry scope structurally from the raw args (before
    // validation) so a failure during resolution still carries the right
    // dimension. No selection→All; any `--plugin`/slash-positional→Plugin; any
    // other explicit selection (bare positional / `--catalog`)→Catalog.
    let tele_scope = reindex_scope_of(&args);

    let result = run_inner(args, ws, mode);

    // OUTCOME-bearing: emit on BOTH success and failure. A failed reindex emits
    // `Reindex{outcome:Failed}` here AND the boundary emits `tome.error` — two
    // distinct signals (intentional). One infallible `enqueue`.
    crate::telemetry::emit(crate::telemetry::event::Reindex {
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

/// Structurally map the raw args to the telemetry
/// [`ReindexScope`](crate::telemetry::event::ReindexScope) — no validation, so
/// it is meaningful even when resolution later rejects a token. Whole-index (no
/// selection) → `All`; any `--plugin` or slash-bearing positional token →
/// `Plugin`; any other explicit selection (`--catalog` or a bare positional) →
/// `Catalog`.
fn reindex_scope_of(args: &ReindexArgs) -> crate::telemetry::event::ReindexScope {
    use crate::telemetry::event::ReindexScope;
    let no_selection = args.scopes.is_empty() && args.catalog.is_empty() && args.plugin.is_empty();
    if no_selection {
        ReindexScope::All
    } else if !args.plugin.is_empty() || args.scopes.iter().any(|s| s.contains('/')) {
        ReindexScope::Plugin
    } else {
        ReindexScope::Catalog
    }
}

fn run_inner(args: ReindexArgs, ws: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    // Phase 12 / US2: load the global config strictly so the embedder resolves
    // remote-vs-bundled, the policy gate compares the right identity, and (on a
    // remote whole-index run) the established dimension can be persisted.
    let cfg = crate::config::load(&paths)?;

    // Issue #316: resolve the variadic scopes / `*` globs / `--catalog` /
    // `--plugin` into a deduped target set plus the load-bearing `whole_index`
    // bool and a human label. Resolution is FAIL-FAST (the first invalid token
    // errors) — the meta-stamp policy below depends on an all-or-nothing target
    // set, so this deliberately does NOT use the selector's forward-progress.
    let Selection {
        targets: plugins,
        whole_index,
        label,
    } = resolve_selection(&args, &paths, &ws.scope)?;

    if plugins.is_empty() {
        // A VALID exact selection with no enabled plugins in scope (e.g. an
        // enrolled-but-empty catalog, or the whole-index form on a fresh
        // install). A glob that matched zero already errored in
        // `resolve_selection` (Usage/2), so this branch is only the benign case.
        //
        // Issue #498: on a WHOLE-INDEX empty reindex we must still adopt the
        // active embedder as the baseline before returning — otherwise a model
        // change on an empty index (no plugins enabled yet) is a dead end: the
        // drift guard (`guard_embedder_drift`, 41/42) refuses `plugin enable`
        // and directs the user to reindex, but a whole-index reindex on an empty
        // corpus hits THIS branch and returns before the meta stamp below (the
        // sole drift resolver), so the drift never clears. There are zero
        // vectors to regenerate, but the global `meta` embedder identity +
        // `embedder_dimension` are reconciled to the active embedder so the
        // stored baseline matches the configured one and the drift clears. An
        // empty index has no stored vectors that could mismatch the new
        // embedder, so adopting it as the baseline is always safe.
        //
        // This is GATED on `whole_index`: a SCOPED empty selection (e.g. an
        // enrolled-but-empty catalog) keeps the current no-stamp behaviour, per
        // the "never stamp after a partial re-embed" invariant — stamping a
        // global dimension after a partial run advertises a dimension the
        // out-of-scope rows may not carry.
        if whole_index {
            stamp_active_embedder_on_empty_index(&paths, &cfg)?;
        }
        // Exit 0 with a small notice so the user knows this wasn't a silent
        // failure.
        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(out, "Nothing to reindex (no enabled plugins in scope).")?;
        }
        return Ok(());
    }

    // B1: a profile-driven embedder change requires a WHOLE-INDEX re-embed; the
    // GLOBAL `meta` embedder stamp is gated on it. Open one writable handle for
    // the active-embedder read + the (post-commit) stamp. `whole_index` is the
    // "no explicit selection at all" discriminant from `resolve_selection`
    // above — ANY `--catalog`/`--plugin`/positional token makes it `false`.
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

    // Each plugin re-embeds in its OWN transaction + lock (lifecycle.rs), and
    // `execute` stops on the first error. So a mid-run failure (e.g. a remote
    // `RemoteEmbeddingInvalid`/95 on plugin N during a model switch) is NOT
    // silent corruption: plugins 1..N-1 committed at the new dimension, N+1..
    // remain at the old, and the GLOBAL `meta` embedder + `embedder_dimension`
    // stamps below are SKIPPED (they only run after `execute` returns Ok). Every
    // subsequent read/write is then fail-closed — `query`/MCP hit embedder drift
    // (41), `plugin enable`/`catalog update` hit `guard_embedder_drift` (41/42),
    // and `vec_distance_cosine` hard-errors on any mixed-dimension row. A re-run
    // of `tome reindex --force` re-embeds everything and is fully self-healing.
    let aggregate = execute_targets(&plugins, &deps, force)?;

    // B1 + issue #516: stamp the GLOBAL `meta` embedder rows ONLY after a
    // WHOLE-INDEX re-embed commits, inside a single SQLite transaction so no
    // concurrent reader can observe a partial write from THIS writer's commit
    // (e.g. EmbedderName updated but EmbedderVersion or EmbedderDimension not
    // yet). Readers that issue multiple separate SELECT statements without their
    // own read transaction can still see inconsistency across those queries —
    // the WAL transaction only makes our write atomic, not their read snapshot.
    // Never stamp after a partial (scoped) re-embed — the `meta` table is a
    // single global key/value store describing the entire index, and a partial
    // stamp would advertise a dimension the out-of-scope rows do not carry.
    // `force` is true here whenever the embedder changed.
    //
    // `stamp_embedder_meta_atomically` handles the `whole_index && force` guard
    // for the identity rows internally via `stamp_embedder_after_whole_index`
    // (which is always correct to call here — see comment in the called fn).
    // `reconcile_embedder_dimension` on the whole-index path is always correct
    // regardless of `force` (it clears a stale remote dimension on the bundled
    // path even when the embedder didn't change).
    if whole_index {
        let established = embedder.established_dimension();
        let persisted_dim = cfg.embedding.dimensions.map(|d| d as usize).or(established);
        if force {
            // Embedder changed (or --force): stamp identity + reconcile dimension
            // atomically so readers never see a half-updated meta.
            stamp_embedder_meta_atomically(
                &policy_conn,
                &configured_ident,
                remote_embedding,
                persisted_dim,
            )?;
        } else {
            // Embedder unchanged: only reconcile the dimension key (no identity
            // rows to update). A single write/delete is already atomic.
            reconcile_embedder_dimension(&policy_conn, remote_embedding, persisted_dim)?;
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

    emit_label(&label, &aggregate, mode)
}

/// Issue #498: adopt the active embedder as the global `meta` baseline on a
/// WHOLE-INDEX reindex whose corpus is empty (no enabled plugins). This is the
/// empty-corpus analogue of the stamp + dimension reconcile the non-empty
/// whole-index path performs after its embed loop (see `run_inner`) — the sole
/// resolver for embedder drift when the index has no vectors yet.
///
/// There is nothing to re-embed, so no [`Embedder`](crate::embedding::Embedder)
/// is constructed (avoiding a model download/load just to stamp identity). Only
/// three inputs are needed: the configured active-embedder identity, whether the
/// active embedder is remote, and the pinned `[embedding] dimensions` (the
/// established-from-first-embed dimension is necessarily unknown here — nothing
/// was embedded — so it contributes `None`).
///
/// The caller GATES this on `whole_index`; it must never run after a scoped
/// selection. On an empty corpus stamping is unconditionally safe (there are no
/// stored vectors that could mismatch the adopted identity or dimension), so —
/// unlike the non-empty path's `whole_index && force` stamp gate — the stamp is
/// applied whenever this is reached, ensuring the drift clears even without an
/// explicit `--force`.
fn stamp_active_embedder_on_empty_index(
    paths: &Paths,
    cfg: &crate::config::Config,
) -> Result<(), TomeError> {
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
    let configured = meta::active_embedder(&policy_conn)?;
    let active_embedder_seed = crate::embedding::embedder_seed(cfg, configured)?;
    let configured_ident = ModelIdent {
        name: active_embedder_seed.name.clone(),
        version: active_embedder_seed.version.clone(),
    };
    let remote_embedding =
        crate::provider::resolve(cfg, crate::provider::Capability::Embedding)?.is_some();

    // Issue #516: write identity + dimension atomically. No embed happened so
    // there is no established dimension; only a pinned `[embedding] dimensions`
    // contributes. `stamp_embedder_meta_atomically` wraps both the identity
    // rows (`stamp_embedder_after_whole_index`) and the dimension key
    // (`reconcile_embedder_dimension`) in one SQLite transaction.
    let persisted_dim = cfg.embedding.dimensions.map(|d| d as usize);
    stamp_embedder_meta_atomically(
        &policy_conn,
        &configured_ident,
        remote_embedding,
        persisted_dim,
    )?;
    Ok(())
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

/// The resolved outcome of the #316 selection: the deduped, order-preserving
/// target set, the load-bearing `whole_index` flag (true ONLY for the no-selection
/// form), and a human label for emit.
#[derive(Debug)]
struct Selection {
    targets: Vec<PluginId>,
    whole_index: bool,
    label: String,
}

/// Resolve the variadic scopes / `*` globs / `--catalog` / `--plugin` flags into
/// a [`Selection`]. FAIL-FAST: validate every token and error on the first
/// invalid one (the meta-stamp policy depends on an all-or-nothing target set —
/// see the module header). This deliberately does NOT reuse
/// `selector::resolve`, whose bare-token = plugin-name semantics are wrong here:
/// a bare token to `reindex` is a whole CATALOG, not a plugin name.
///
/// Exit-code contract (preserved from the pre-#316 `parse_scope`, pinned by
/// `tests/index_query_misc/reindex.rs`):
/// * a malformed slash literal (`bad/id/extra`) → [`TomeError::Usage`] (2);
/// * an unknown catalog → [`TomeError::CatalogNotFound`] (3);
/// * a known catalog + unknown/not-enabled plugin → [`TomeError::PluginNotFound`]
///   (20);
/// * a glob matching zero → [`TomeError::Usage`] (2), never a silent no-op.
fn resolve_selection(
    args: &ReindexArgs,
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
) -> Result<Selection, TomeError> {
    let no_selection = args.scopes.is_empty() && args.catalog.is_empty() && args.plugin.is_empty();

    // ---- WHOLE-INDEX: no selection at all ---------------------------------
    if no_selection {
        let targets = enabled_plugin_ids_for_workspace(paths, ws_scope)?;
        return Ok(Selection {
            targets,
            whole_index: true,
            label: "all".to_owned(),
        });
    }

    // Any explicit selection: read the candidate universe ONCE, then resolve
    // purely (no I/O) so the classification + glob expansion is unit-testable.
    let candidates = enabled_plugin_ids_for_workspace(paths, ws_scope)?;
    let enrolled_catalogs = enrolled_catalog_names(paths, ws_scope)?;
    resolve_explicit(args, &candidates, &enrolled_catalogs)
}

/// Pure resolution of an EXPLICIT selection (at least one of scopes / catalog /
/// plugin is non-empty) against the pre-read candidate universe. No I/O — the
/// enabled-plugin candidate set and enrolled catalog names are passed in — so
/// the token classification + `*` expansion is unit-testable without a DB.
///
/// FAIL-FAST: validates every token in order and returns the FIRST error. The
/// meta-stamp policy depends on an all-or-nothing target set, so this does NOT
/// use forward-progress (unlike `selector::resolve`). It also does NOT reuse
/// `selector::resolve`: a bare token to reindex is a whole CATALOG, not a plugin
/// name.
fn resolve_explicit(
    args: &ReindexArgs,
    candidates: &[PluginId],
    enrolled_catalogs: &[String],
) -> Result<Selection, TomeError> {
    // Classify each source into catalog-level vs plugin-level tokens. A
    // `--catalog` value and a slash-free positional are catalog-level; a
    // `--plugin` value and a slash-bearing positional are plugin-level.
    let mut catalog_tokens: Vec<&str> = Vec::new();
    let mut plugin_tokens: Vec<&str> = Vec::new();
    for tok in &args.catalog {
        catalog_tokens.push(tok.as_str());
    }
    for tok in &args.plugin {
        plugin_tokens.push(tok.as_str());
    }
    for tok in &args.scopes {
        if tok.contains('/') {
            plugin_tokens.push(tok.as_str());
        } else {
            catalog_tokens.push(tok.as_str());
        }
    }

    let mut targets: Vec<PluginId> = Vec::new();
    let push_unique = |targets: &mut Vec<PluginId>, id: PluginId| {
        if !targets.contains(&id) {
            targets.push(id);
        }
    };

    // ---- catalog-level tokens ---------------------------------------------
    for tok in &catalog_tokens {
        if is_glob(tok) {
            // Match the pattern against enrolled catalog NAMES; each match
            // expands to that catalog's enabled plugins. Zero catalog matches
            // is a usage error, echoing the pattern (never a silent no-op).
            let mut any = false;
            for cat in enrolled_catalogs {
                if glob_match(tok, cat) {
                    any = true;
                    for cand in candidates.iter().filter(|c| &c.catalog == cat) {
                        push_unique(&mut targets, cand.clone());
                    }
                }
            }
            if !any {
                return Err(TomeError::Usage(format!(
                    "pattern `{tok}` matched no enrolled catalogs\n\
                     hint: run `tome catalog list` to see enrolled catalogs"
                )));
            }
        } else {
            // Exact catalog: must be enrolled else CatalogNotFound (3). Expand
            // to its enabled plugins (may be empty — a valid empty selection).
            if !enrolled_catalogs.iter().any(|c| c == tok) {
                return Err(TomeError::CatalogNotFound((*tok).to_owned()));
            }
            for cand in candidates.iter().filter(|c| c.catalog == **tok) {
                push_unique(&mut targets, cand.clone());
            }
        }
    }

    // ---- plugin-level tokens ----------------------------------------------
    for tok in &plugin_tokens {
        if is_glob(tok) {
            // Split on the FIRST `/` into (cat_pat, plug_pat); a slash-free
            // `--plugin` glob has no catalog half, so match its whole value
            // against the plugin segment across every enrolled catalog.
            let (cat_pat, plug_pat) = match tok.split_once('/') {
                Some((c, p)) => (c, p),
                None => ("*", *tok),
            };
            let mut any = false;
            for cand in candidates {
                if glob_match(cat_pat, &cand.catalog) && glob_match(plug_pat, &cand.plugin) {
                    any = true;
                    push_unique(&mut targets, cand.clone());
                }
            }
            if !any {
                return Err(TomeError::Usage(format!(
                    "pattern `{tok}` matched no enabled plugins\n\
                     hint: run `tome plugin list` to see enabled `<catalog>/<plugin>` ids"
                )));
            }
        } else {
            // Literal `<catalog>/<plugin>`: parse via the PluginId SSOT so a
            // malformed id (`bad/id/extra`) is Usage/2 (preserving the pin);
            // then enforce existence exactly as the pre-#316 `parse_scope` did —
            // catalog enrolled? else CatalogNotFound/3; plugin in the enabled
            // set? else PluginNotFound/20.
            let id = PluginId::from_str(tok)
                .map_err(|e| TomeError::Usage(format!("invalid plugin id `{tok}`: {e}")))?;
            if !enrolled_catalogs.iter().any(|c| c == &id.catalog) {
                return Err(TomeError::CatalogNotFound(id.catalog));
            }
            if !candidates
                .iter()
                .any(|c| c.catalog == id.catalog && c.plugin == id.plugin)
            {
                return Err(TomeError::PluginNotFound(id.to_string()));
            }
            push_unique(&mut targets, id);
        }
    }

    let label = selection_label(&catalog_tokens, &plugin_tokens, targets.len());
    Ok(Selection {
        targets,
        whole_index: false,
        label,
    })
}

/// A concise human label describing an explicit selection, e.g.
/// `midnight, other/* (3 plugins)`. Used only for the human/JSON emit summary;
/// the actual re-embed is driven by the resolved target set.
fn selection_label(catalog_tokens: &[&str], plugin_tokens: &[&str], count: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(catalog_tokens.iter().map(|t| (*t).to_owned()));
    parts.extend(plugin_tokens.iter().map(|t| (*t).to_owned()));
    let joined = parts.join(", ");
    format!(
        "{joined} ({count} plugin{})",
        if count == 1 { "" } else { "s" }
    )
}

/// Every enrolled catalog NAME for the resolved workspace — the match universe
/// for catalog-level globs and the exact-catalog existence check.
fn enrolled_catalog_names(
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
) -> Result<Vec<String>, TomeError> {
    let conn = open_index_for_read(paths, ws_scope)?;
    let enrolments =
        index::workspace_catalogs::list_for_workspace(&conn, ws_scope.name().as_str())?;
    Ok(enrolments.into_iter().map(|e| e.catalog_name).collect())
}

/// Every enabled plugin (as a [`PluginId`]) across every catalog for the
/// resolved workspace, ordered by `(catalog, plugin)`. The whole-index target
/// set AND the candidate universe for #316 glob matching.
fn enabled_plugin_ids_for_workspace(
    paths: &Paths,
    ws_scope: &crate::workspace::Scope,
) -> Result<Vec<PluginId>, TomeError> {
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
    let pairs = index::skills::enabled_plugins_for_workspace(&conn, ws_scope.name().as_str())?;
    Ok(pairs
        .into_iter()
        .map(|(catalog, plugin)| PluginId { catalog, plugin })
        .collect())
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
///
/// NOTE: this function writes two separate autocommit statements. Production
/// callers should use [`stamp_embedder_meta_atomically`] which wraps both this
/// call and [`reconcile_embedder_dimension`] in a single SQLite transaction so
/// the identity stamp + dimension key are never half-visible to concurrent
/// readers (issue #516).
pub fn stamp_embedder_after_whole_index(
    conn: &rusqlite::Connection,
    configured_embedder: &ModelIdent,
) -> Result<(), TomeError> {
    meta::write(conn, MetaKey::EmbedderName, &configured_embedder.name)?;
    meta::write(conn, MetaKey::EmbedderVersion, &configured_embedder.version)?;
    Ok(())
}

/// Phase 12 / US2 (FR-015a) + US4: reconcile `meta.embedder_dimension` after a
/// WHOLE-INDEX re-embed has committed. `embedder_dimension` is a REMOTE-only
/// concept — the bundled storage is dimension-free (there is no registry
/// dimension to record). The caller gates this on `whole_index` so a partial
/// (scoped) reindex can never stamp/clear a dimension the out-of-scope rows may
/// not share.
///
/// * `remote == true` — PERSIST `persisted_dim` (the `[embedding] dimensions`
///   knob if the user pinned one, else the dimension established from the first
///   successful embed of the run). `persisted_dim == None` (a run that
///   re-embedded nothing AND established no dimension) leaves any prior value
///   untouched.
/// * `remote == false` (BUNDLED) — DELETE any stale remote value. After a
///   remote→bundled switch the stored vectors are bundled-dimension, but
///   `meta.embedder_dimension` would still carry the old remote value — so the
///   doctor corrupt-index check (FR-017) would compare bundled-dim vectors
///   against the stale remote dim FOREVER, a finding that could never
///   self-heal. Clearing the key here makes a bundled `doctor --fix`
///   (→ reindex → delete) self-healing AND extends the same heal to a manual
///   `tome reindex`. A no-op when the row is already absent (the common
///   bundled case).
///
/// Exposed (`pub`) so the corrupt-index-to-extinction regression test can drive
/// the exact reconcile `run_inner` uses with a `StubEmbedder`, without loading a
/// real on-disk model.
///
/// NOTE: this function writes at most one autocommit statement. Production
/// callers should use [`stamp_embedder_meta_atomically`] which wraps both this
/// call and [`stamp_embedder_after_whole_index`] in a single SQLite transaction
/// (issue #516).
pub fn reconcile_embedder_dimension(
    conn: &rusqlite::Connection,
    remote: bool,
    persisted_dim: Option<usize>,
) -> Result<(), TomeError> {
    if remote {
        if let Some(dim) = persisted_dim {
            meta::write_embedder_dimension(conn, dim)?;
        }
    } else {
        meta::delete_embedder_dimension(conn)?;
    }
    Ok(())
}

/// Atomically stamp the GLOBAL `meta` embedder identity + dimension after a
/// WHOLE-INDEX re-embed has committed (issue #516).
///
/// Wraps [`stamp_embedder_after_whole_index`] and [`reconcile_embedder_dimension`]
/// inside a single SQLite WAL transaction so the three potential writes
/// (`EmbedderName`, `EmbedderVersion`, and `EmbedderDimension`) land in one
/// commit — no concurrent reader can observe a partial write from THIS writer
/// (e.g. name updated but version or dimension not yet). This is writer-side
/// atomicity: readers such as `detect_drift` that issue multiple separate
/// autocommit queries without an enclosing read transaction may still see
/// inconsistency across their own reads, because WAL isolation does not force
/// a snapshot across the reader's independent statements.
///
/// The `configured_embedder`, `remote`, and `persisted_dim` parameters have the
/// same semantics as their counterparts in the two inner functions.
///
/// Callers MUST NOT invoke this after a partial (scoped) re-embed — the
/// meta table is a single global key/value store and a partial stamp would
/// advertise an identity the out-of-scope rows do not share. See
/// [`embedder_change_policy`].
pub fn stamp_embedder_meta_atomically(
    conn: &rusqlite::Connection,
    configured_embedder: &ModelIdent,
    remote: bool,
    persisted_dim: Option<usize>,
) -> Result<(), TomeError> {
    // A DEFERRED transaction: we hold only a SHARED lock until we first write,
    // at which point SQLite promotes to RESERVED/EXCLUSIVE — exactly the
    // minimal-contention pattern used by `reindex_plugin_atomic` (skills.rs).
    // The advisory lockfile (`index.lock`) is an application-level guard for
    // mutating writers; this SQLite transaction makes the meta writes commit as
    // one atomic unit — no concurrent reader can observe a partial write from
    // this commit. Readers that issue multiple separate autocommit queries
    // (e.g. detect_drift) may still see inconsistency across their own reads.
    let tx = conn.unchecked_transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("begin embedder-stamp tx: {e}"))
    })?;
    stamp_embedder_after_whole_index(&tx, configured_embedder)?;
    reconcile_embedder_dimension(&tx, remote, persisted_dim)?;
    tx.commit().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("commit embedder-stamp tx: {e}"))
    })?;
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
/// `force` is propagated to each call. FAIL-FAST: stops on the first per-plugin
/// error (the meta-stamp policy in `run_inner` depends on an all-or-nothing
/// target set).
fn execute_targets(
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
) -> Result<ReindexAggregate, TomeError> {
    let started = Instant::now();
    let mut aggregate = ReindexAggregate::default();
    // #480: a multi-plugin run owns ONE aggregate plugin-level bar here
    // instead of the per-entry bar `reindex_plugin` would construct and
    // clear inside every iteration — many small plugins otherwise read as
    // rapid bar churn. A single-plugin run keeps the finer per-entry bar.
    // The closure keeps ONE clear point for the bar on both the success and
    // the fail-fast error path.
    let multi = plugins.len() > 1;
    let agg_bar = if multi {
        crate::presentation::progress::bar(plugins.len() as u64, "reindexing")
    } else {
        indicatif::ProgressBar::hidden()
    };
    let result = (|| -> Result<(), TomeError> {
        for id in plugins {
            agg_bar.set_prefix(format!("reindexing {}/{}", id.catalog, id.plugin));
            let outcome = lifecycle::reindex_plugin_with_entry_bar(id, deps, force, !multi)?;
            agg_bar.inc(1);
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
        Ok(())
    })();
    agg_bar.finish_and_clear();
    result?;
    aggregate.duration_ms = duration_ms(started);
    Ok(aggregate)
}

/// Test/library entry-point shim: drive [`execute_targets`] from a legacy
/// [`Scope`] value. The `Scope` no longer drives production selection (that is
/// `resolve_selection` → `execute_targets`); it survives only so the pre-#316
/// `run_with_deps` tests keep compiling and describing their intent. Existence
/// is NOT validated here — tests pass an already-resolved plugin list.
pub fn execute(
    _scope: &Scope,
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
) -> Result<ReindexAggregate, TomeError> {
    execute_targets(plugins, deps, force)
}

fn duration_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn emit_label(label: &str, aggregate: &ReindexAggregate, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(label, aggregate),
        Mode::Json => emit_json(label, aggregate),
    }
}

fn emit_human(label: &str, agg: &ReindexAggregate) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Reindexed {} ({} plugin{}, {} skill{} checked)",
        label,
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

fn emit_json(label: &str, agg: &ReindexAggregate) -> Result<(), TomeError> {
    let record = ReindexRecord {
        scope: label.to_owned(),
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
/// validation so tests can scope by plugin without registering a catalog. The
/// emitted `scope` label uses the legacy [`Scope::label`] (test-only path).
pub fn run_with_deps(
    scope: Scope,
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
    mode: Mode,
) -> Result<ReindexAggregate, TomeError> {
    let aggregate = execute(&scope, plugins, deps, force)?;
    emit_label(&scope.label(), &aggregate, mode)?;
    Ok(aggregate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(catalog: &str, plugin: &str) -> PluginId {
        PluginId {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
        }
    }

    /// The enabled-plugin candidate universe shared across the #316 resolution
    /// tests: two catalogs, one of them holding a `compact-*` family.
    fn candidates() -> Vec<PluginId> {
        vec![
            id("midnight", "compact-lint"),
            id("midnight", "compact-fmt"),
            id("midnight", "audit"),
            id("other", "helper"),
        ]
    }

    fn catalogs() -> Vec<String> {
        vec!["midnight".to_owned(), "other".to_owned()]
    }

    /// Build a `ReindexArgs` directly (bypassing clap) for the pure resolver.
    fn args(scopes: &[&str], catalog: &[&str], plugin: &[&str]) -> ReindexArgs {
        ReindexArgs {
            scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
            catalog: catalog.iter().map(|s| (*s).to_owned()).collect(),
            plugin: plugin.iter().map(|s| (*s).to_owned()).collect(),
            force: false,
        }
    }

    fn resolve(scopes: &[&str], catalog: &[&str], plugin: &[&str]) -> Result<Selection, TomeError> {
        resolve_explicit(&args(scopes, catalog, plugin), &candidates(), &catalogs())
    }

    // ---- whole-catalog (bare positional) ----------------------------------

    #[test]
    fn bare_positional_expands_a_whole_catalog() {
        let sel = resolve(&["midnight"], &[], &[]).expect("resolve");
        assert!(
            !sel.whole_index,
            "an explicit selection is NEVER whole-index"
        );
        assert_eq!(
            sel.targets,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt"),
                id("midnight", "audit"),
            ],
        );
    }

    #[test]
    fn unknown_catalog_bare_positional_is_catalog_not_found() {
        let err = resolve(&["ghost"], &[], &[]).expect_err("unknown catalog");
        assert_eq!(err.exit_code(), 3);
        assert!(matches!(err, TomeError::CatalogNotFound(c) if c == "ghost"));
    }

    #[test]
    fn enrolled_catalog_with_no_enabled_plugins_is_a_valid_empty_selection() {
        // `midnight`/`other` are enrolled; a THIRD enrolled catalog with zero
        // enabled plugins resolves to an empty target set (the benign
        // "Nothing to reindex" path), NOT an error.
        let cats = vec!["midnight".to_owned(), "empty-cat".to_owned()];
        let sel = resolve_explicit(&args(&["empty-cat"], &[], &[]), &candidates(), &cats)
            .expect("empty-but-enrolled catalog resolves");
        assert!(sel.targets.is_empty());
        assert!(!sel.whole_index);
    }

    // ---- --catalog flag ----------------------------------------------------

    #[test]
    fn catalog_flag_matches_bare_positional_semantics() {
        let sel = resolve(&[], &["midnight"], &[]).expect("resolve");
        assert_eq!(
            sel.targets,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt"),
                id("midnight", "audit"),
            ],
        );
    }

    #[test]
    fn catalog_glob_matches_enrolled_names() {
        // `*` matches BOTH enrolled catalogs → every enabled plugin, deduped.
        let sel = resolve(&[], &["*"], &[]).expect("resolve");
        assert_eq!(
            sel.targets,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt"),
                id("midnight", "audit"),
                id("other", "helper"),
            ],
        );
    }

    #[test]
    fn catalog_glob_zero_match_is_usage_error() {
        let err = resolve(&[], &["ghost-*"], &[]).expect_err("zero catalog match");
        assert_eq!(err.exit_code(), 2, "a glob matching nothing is Usage/2");
        assert!(matches!(&err, TomeError::Usage(m) if m.contains("ghost-*")));
    }

    // ---- plugin-level (slash positional / --plugin) -----------------------

    #[test]
    fn slash_positional_single_plugin() {
        let sel = resolve(&["midnight/audit"], &[], &[]).expect("resolve");
        assert_eq!(sel.targets, vec![id("midnight", "audit")]);
    }

    #[test]
    fn plugin_flag_single_plugin() {
        let sel = resolve(&[], &[], &["other/helper"]).expect("resolve");
        assert_eq!(sel.targets, vec![id("other", "helper")]);
    }

    #[test]
    fn plugin_glob_expands_within_catalog() {
        let sel = resolve(&["midnight/compact-*"], &[], &[]).expect("resolve");
        assert_eq!(
            sel.targets,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt")
            ],
        );
    }

    #[test]
    fn plugin_glob_zero_match_is_usage_error() {
        let err = resolve(&["midnight/xyz-*"], &[], &[]).expect_err("zero plugin match");
        assert_eq!(err.exit_code(), 2);
        assert!(matches!(&err, TomeError::Usage(m) if m.contains("midnight/xyz-*")));
    }

    #[test]
    fn malformed_slash_literal_is_usage() {
        // The `bad/id/extra` pin: two slashes → invalid id → Usage/2, NOT a
        // hand-split catalog. Preserves `reindex_invalid_scope_format_exits_2`.
        let err = resolve(&["bad/id/extra"], &[], &[]).expect_err("malformed id");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn slash_literal_unknown_catalog_is_catalog_not_found() {
        let err = resolve(&["ghost/audit"], &[], &[]).expect_err("unknown catalog");
        assert_eq!(err.exit_code(), 3);
    }

    #[test]
    fn slash_literal_known_catalog_unknown_plugin_is_plugin_not_found() {
        let err = resolve(&["midnight/ghost"], &[], &[]).expect_err("unknown plugin");
        assert_eq!(err.exit_code(), 20);
        assert!(matches!(&err, TomeError::PluginNotFound(p) if p == "midnight/ghost"));
    }

    // ---- union + dedupe ----------------------------------------------------

    #[test]
    fn multiple_positional_scopes_union_and_dedupe() {
        // A whole catalog + one of its plugins (already covered) + a plugin in
        // the other catalog: `midnight/audit` must appear ONCE.
        let sel =
            resolve(&["midnight", "midnight/audit", "other/helper"], &[], &[]).expect("resolve");
        assert_eq!(
            sel.targets,
            vec![
                id("midnight", "compact-lint"),
                id("midnight", "compact-fmt"),
                id("midnight", "audit"),
                id("other", "helper"),
            ],
            "midnight/audit is not duplicated by the whole-catalog token",
        );
    }

    #[test]
    fn catalog_and_plugin_flags_combine_as_a_union() {
        // `--catalog other` (→ other/helper) unioned with `--plugin
        // midnight/audit`.
        let sel = resolve(&[], &["other"], &["midnight/audit"]).expect("resolve");
        assert_eq!(
            sel.targets,
            vec![id("other", "helper"), id("midnight", "audit")],
        );
    }

    // ---- label -------------------------------------------------------------

    #[test]
    fn label_reflects_tokens_and_count() {
        let sel = resolve(&["midnight/compact-*"], &[], &[]).expect("resolve");
        assert_eq!(sel.label, "midnight/compact-* (2 plugins)");
        let one = resolve(&["midnight/audit"], &[], &[]).expect("resolve");
        assert_eq!(one.label, "midnight/audit (1 plugin)");
    }

    // ---- telemetry scope classification -----------------------------------

    #[test]
    fn tele_scope_all_when_no_selection() {
        use crate::telemetry::event::ReindexScope;
        assert_eq!(reindex_scope_of(&args(&[], &[], &[])), ReindexScope::All);
    }

    #[test]
    fn tele_scope_plugin_when_slash_or_plugin_flag() {
        use crate::telemetry::event::ReindexScope;
        assert_eq!(
            reindex_scope_of(&args(&["midnight/audit"], &[], &[])),
            ReindexScope::Plugin,
        );
        assert_eq!(
            reindex_scope_of(&args(&[], &[], &["midnight/audit"])),
            ReindexScope::Plugin,
        );
    }

    #[test]
    fn tele_scope_catalog_for_bare_or_catalog_flag() {
        use crate::telemetry::event::ReindexScope;
        assert_eq!(
            reindex_scope_of(&args(&["midnight"], &[], &[])),
            ReindexScope::Catalog,
        );
        assert_eq!(
            reindex_scope_of(&args(&[], &["midnight"], &[])),
            ReindexScope::Catalog,
        );
    }
}
