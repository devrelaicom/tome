//! INTEGRATION-level acceptance guarantees for the catalog-attributed telemetry
//! stream, asserted across the crate boundary (`tome::telemetry::*`) against a
//! real staged + enrolled index, re-homed onto the `gauge-telemetry` kernel.
//!
//! The kernel owns the queue + envelope, so:
//! - the anonymous + attributed events both land on the ONE kernel queue
//!   (`paths.telemetry_queue()`), read here as the `QueuedEvent` shape
//!   (`event_name` + nested `attributes`);
//! - the attributed event NAMES are `tome.catalog_*` with a `catalog` attribute
//!   (was the old `catalog.<id>.*` envelope name + `catalog_id` field);
//! - there is no per-line install/session uuid (the kernel attaches identity as
//!   OTLP resource attributes at drain time) and no `sample_rate` — the
//!   never-sampled property of attributed events is now structural (the kernel
//!   does not sample), so it is no longer a per-line assertion.
//!
//! What this still proves end-to-end / through the queue:
//! - both streams share one drain (queue): the anonymous + the attributed line;
//! - FR-052 (the SOURCE is the gate — a name collision with a non-allowlisted
//!   source stays anonymous);
//! - FR-057 (the attributed `search_result` carries an EXACT integer `rank`);
//! - FR-053 (de-allowlisting is an emit-time `const` decision; nothing persisted).
//!
//! The resolver/allowlist primitives (`match_source`/`resolve_attribution_with`)
//! are covered by `src/telemetry/{allowlist,resolver}.rs` `#[cfg(test)]`; this
//! file covers the assembled wiring against the kernel queue.

use serde_json::Value;
use tempfile::TempDir;

use tome::index::{self, OpenOptions, workspace_catalogs};
use tome::paths::Paths;
use tome::telemetry::allowlist::{ATTRIBUTED_TELEMETRY_CATALOGS, match_source};
use tome::telemetry::event::{EntryKind, Harness, PluginActionEvent, PluginEnabled, SearchResult};
use tome::telemetry::resolve_attribution;
use tome::telemetry::resolver::resolve_attribution_with;
use tome::workspace::{ResolvedScope, WorkspaceName};

use crate::common::{HomeGuard, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use crate::queue_util::{LOOPBACK_ENDPOINT, TELEMETRY_ENV_VARS, first_named, queue_events};

// ---------------------------------------------------------------------------
// Shared fixtures.
// ---------------------------------------------------------------------------

/// A canonical, HTTPS-scheme Midnight catalog source — reduces to the bare
/// `github.com/devrelaicom/midnight-expert-tome` const entry.
const MIDNIGHT_SOURCE: &str = "https://github.com/devrelaicom/midnight-expert-tome";

/// A non-allowlisted source whose path COLLIDES with Midnight by repo name but
/// lives under a different org — the FR-052 privacy-critical case.
const COLLIDING_NON_MIDNIGHT_SOURCE: &str = "https://github.com/someone/midnight-expert-tome";

/// The allowlist short id the Midnight source resolves to.
const MIDNIGHT_ID: &str = "midnight";

/// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY=1` plus a
/// loopback endpoint, restore on drop. Pairs with a held [`HomeGuard`].
struct EnvForce {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvForce {
    fn install() -> Self {
        let saved = TELEMETRY_ENV_VARS
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        // SAFETY: the caller holds `HOME_MUTEX` via a `HomeGuard`.
        for &k in TELEMETRY_ENV_VARS {
            unsafe { std::env::remove_var(k) };
        }
        unsafe {
            std::env::set_var("TOME_TELEMETRY", "1");
            std::env::set_var("TOME_GAUGE_ENDPOINT", LOOPBACK_ENDPOINT);
        }
        Self { saved }
    }
}

impl Drop for EnvForce {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY: still under the test's `HomeGuard`/`HOME_MUTEX`.
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }
}

/// Open a fresh central index at `paths`, with `global` seeded so enrolments can
/// target it.
fn open_seeded_index(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open seeded index")
}

/// Enrol `(global, catalog_name) -> url` into `workspace_catalogs`.
fn enrol(paths: &Paths, catalog_name: &str, url: &str) {
    let conn = open_seeded_index(paths);
    workspace_catalogs::insert(
        &conn,
        WorkspaceName::global().as_str(),
        catalog_name,
        url,
        "main",
    )
    .expect("enrol catalog");
}

fn global_scope() -> ResolvedScope {
    ResolvedScope::global_fallback()
}

/// Install the process-global emit override pointed at the staged queue.
fn install_handle(paths: &Paths) -> tome::telemetry::TelemetryHandleGuard {
    tome::telemetry::TelemetryHandleGuard::install(tome::telemetry::build_handle_for_test(paths))
}

fn first_of<'a>(events: &'a [Value], event_name: &str) -> Option<&'a Value> {
    first_named(events, event_name)
}

// ---------------------------------------------------------------------------
// Both streams share ONE drain (END-TO-END).
//
// Stage a catalog whose enrolled `url` IS the Midnight source under an isolated
// `$HOME/.tome`, then drive the EXACT emit pair `commands/plugin/enable.rs`
// makes: the always-on anonymous `emit` PLUS the attribution-gated emit, where
// the gate is the REAL default-`Paths` `resolve_attribution` (it opens the staged
// index read-only and canonicalizes the enrolled URL against the compiled-in
// const). Both lines MUST land on the SAME kernel queue.
// ---------------------------------------------------------------------------

#[test]
fn both_streams_share_one_drain() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    // The production layout: everything under `$HOME/.tome`, which is what the
    // default `Paths::resolve()` (used by `resolve_attribution`) reads.
    let paths = Paths::resolve().expect("resolve default paths under HomeGuard");
    std::fs::create_dir_all(&paths.root).unwrap();
    let _handle = install_handle(&paths);
    // The catalog NAME is deliberately an arbitrary alias — attribution is by
    // SOURCE, so the alias is irrelevant to the match.
    enrol(&paths, "my-midnight-alias", MIDNIGHT_SOURCE);

    let scope = global_scope();

    // The two emits, exactly as `enable.rs` orders them: anonymous always, then
    // the attributed one gated on the REAL resolver against the default paths.
    tome::telemetry::emit(PluginActionEvent {
        action: tome::telemetry::event::PluginAction::Enabled,
    });
    let attributed = resolve_attribution(&scope, "my-midnight-alias");
    assert_eq!(
        attributed,
        Some(MIDNIGHT_ID),
        "the default-Paths resolver must attribute the Midnight-sourced catalog to `midnight`",
    );
    if let Some(catalog_id) = attributed {
        tome::telemetry::emit(PluginEnabled {
            catalog: catalog_id,
            plugin_name: "midnight-expert".to_string(),
            plugin_version: "1.2.0".to_string(),
        });
    }

    let events = queue_events(&paths);

    // BOTH the anonymous and the attributed line are in the SAME drain (queue).
    let anon = first_of(&events, "tome.plugin_action")
        .expect("the anonymous tome.plugin_action landed on the queue");
    let attr = first_of(&events, "tome.catalog_plugin_enabled")
        .expect("the attributed tome.catalog_plugin_enabled landed on the SAME queue");

    assert_eq!(anon["attributes"]["action"], "enabled");
    assert_eq!(attr["attributes"]["plugin_name"], "midnight-expert");
    assert_eq!(attr["attributes"]["plugin_version"], "1.2.0");
    assert_eq!(attr["attributes"]["catalog"], MIDNIGHT_ID);
}

// ---------------------------------------------------------------------------
// FR-052 — the SOURCE is the gate; a NAME collision stays anonymous-only.
// ---------------------------------------------------------------------------

#[test]
fn name_collision_source_is_the_gate_anonymous_only() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = Paths::resolve().expect("resolve default paths under HomeGuard");
    std::fs::create_dir_all(&paths.root).unwrap();
    let _handle = install_handle(&paths);
    // Named `midnight`, but the SOURCE is NOT the Midnight repo.
    enrol(&paths, "midnight", COLLIDING_NON_MIDNIGHT_SOURCE);

    let scope = global_scope();

    tome::telemetry::emit(PluginActionEvent {
        action: tome::telemetry::event::PluginAction::Enabled,
    });
    let attributed = resolve_attribution(&scope, "midnight");
    assert_eq!(
        attributed, None,
        "a catalog NAMED `midnight` but enrolled at a non-allowlisted source must NOT attribute \
         (the source, not the name, is the gate)",
    );
    if let Some(catalog_id) = attributed {
        // Unreachable given the assertion; mirrors the exact `enable.rs` shape so a
        // regression that wrongly attributes would emit a `tome.catalog_*` line and
        // fail the no-attributed-line assertion below.
        tome::telemetry::emit(PluginEnabled {
            catalog: catalog_id,
            plugin_name: "midnight-expert".to_string(),
            plugin_version: "1.2.0".to_string(),
        });
    }

    let events = queue_events(&paths);
    assert!(
        first_of(&events, "tome.plugin_action").is_some(),
        "the anonymous event still fires for a name-colliding catalog"
    );
    assert!(
        events.iter().all(|e| !e["event_name"]
            .as_str()
            .unwrap_or("")
            .starts_with("tome.catalog_")),
        "a name collision with a non-allowlisted source must enqueue NO tome.catalog_* line: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// FR-057 — the attributed `search_result` carries the EXACT integer `rank`.
// ---------------------------------------------------------------------------

#[test]
fn attributed_search_result_rank_is_exact_integer_in_the_queue() {
    let dir = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(dir.path());
    let _env = EnvForce::install();
    let paths = Paths::from_root(dir.path().join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    let _handle = install_handle(&paths);

    tome::telemetry::emit(SearchResult {
        entry_name: "midnight-compact-debug".to_string(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".to_string(),
        rank: 7,
        catalog: MIDNIGHT_ID,
        calling_harness: Some(Harness::ClaudeCode),
    });

    let events = queue_events(&paths);
    let result = first_of(&events, "tome.catalog_search_result")
        .expect("the attributed search_result landed on the queue");

    // The defining FR-057 guarantee: `rank` is the EXACT integer 7, a bare JSON
    // number — never a bucket token.
    let rank = &result["attributes"]["rank"];
    assert!(
        rank.is_number(),
        "rank must be a bare JSON number: {result}"
    );
    assert_eq!(
        rank.as_u64(),
        Some(7),
        "rank must be the exact integer 7: {result}"
    );
}

#[test]
fn search_result_serializes_rank_as_a_bare_number() {
    // Belt-and-braces at the serde layer: `SearchResult.rank` is a `u32` that
    // serializes to a bare number.
    let event = SearchResult {
        entry_name: "midnight-compact-debug".to_string(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".to_string(),
        rank: 7,
        catalog: MIDNIGHT_ID,
        calling_harness: None,
    };
    let json = serde_json::to_string(&event).expect("serialize SearchResult");
    assert!(
        json.contains("\"rank\":7"),
        "rank serializes as a bare number `\"rank\":7`, not a quoted bucket: {json}"
    );
    assert!(
        !json.contains("\"rank\":\"7\""),
        "rank must NOT be a quoted string: {json}"
    );
}

// ---------------------------------------------------------------------------
// FR-053 — de-allowlisting is an EMIT-TIME `const` decision; nothing persisted.
// ---------------------------------------------------------------------------

#[test]
fn match_source_reads_the_const_allowlist_present_then_absent() {
    assert_eq!(
        match_source(MIDNIGHT_SOURCE),
        Some(MIDNIGHT_ID),
        "the Midnight source matches the compiled-in const"
    );
    assert_eq!(
        match_source(COLLIDING_NON_MIDNIGHT_SOURCE),
        None,
        "a source absent from the const is not attributed — de-allowlisting is a const edit"
    );
    assert!(
        ATTRIBUTED_TELEMETRY_CATALOGS
            .iter()
            .any(|(id, _)| *id == MIDNIGHT_ID),
        "the compiled-in allowlist contains the midnight entry"
    );
}

#[test]
fn resolve_attribution_is_recomputed_every_call_nothing_persisted() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::from_root(dir.path().to_path_buf());
    let conn = open_seeded_index(&paths);
    workspace_catalogs::insert(
        &conn,
        WorkspaceName::global().as_str(),
        "my-midnight-alias",
        MIDNIGHT_SOURCE,
        "main",
    )
    .expect("enrol Midnight-sourced catalog");

    let scope = global_scope();

    let first = resolve_attribution_with(&conn, &scope, "my-midnight-alias");
    let second = resolve_attribution_with(&conn, &scope, "my-midnight-alias");
    assert_eq!(first, Some(MIDNIGHT_ID));
    assert_eq!(second, Some(MIDNIGHT_ID));

    workspace_catalogs::insert(
        &conn,
        WorkspaceName::global().as_str(),
        "not-midnight",
        COLLIDING_NON_MIDNIGHT_SOURCE,
        "main",
    )
    .expect("enrol non-allowlisted catalog");
    assert_eq!(
        resolve_attribution_with(&conn, &scope, "not-midnight"),
        None,
        "a non-allowlisted source resolves to None on its own emit-time lookup"
    );

    assert_eq!(
        resolve_attribution_with(&conn, &scope, "my-midnight-alias"),
        Some(MIDNIGHT_ID),
        "the original catalog still attributes — the decision is recomputed, never stored/cleared"
    );
}
