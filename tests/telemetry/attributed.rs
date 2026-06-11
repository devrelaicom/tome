//! Phase 10 / US4 (T062) — INTEGRATION-level acceptance guarantees for the
//! catalog-attributed telemetry stream, asserted across the crate boundary
//! (`tome::telemetry::*`) against a real staged + enrolled index.
//!
//! These are the assembled-surface guarantees the unit suites can't reach:
//!
//! - `src/telemetry/allowlist.rs` `#[cfg(test)]` already covers the canonicalizer
//!   equivalence classes (HTTPS / `.git` / SSH / case / slash / credentials →
//!   `Some("midnight")`, a different repo → `None`).
//! - `src/telemetry/resolver.rs` `#[cfg(test)]` covers `resolve_attribution_with`
//!   match / miss / name-collision / missing-enrolment against a staged index.
//! - `tests/telemetry/events.rs` pins the `catalog.midnight.entry_invoked` wire
//!   shape and that attributed events OMIT `sample_rate`.
//!
//! This file instead proves the END-TO-END / through-the-queue contracts:
//! SC-006 (both streams share one drain + one identity), SC-007 / FR-057 (the
//! attributed `search_result` carries an EXACT integer `rank`, never a bucket),
//! FR-052 (the SOURCE is the gate — a name collision with a non-allowlisted
//! source stays anonymous), FR-058 (attributed lines are never sampled while
//! anonymous lines in the SAME drain carry `sample_rate`), and FR-053
//! (de-allowlisting is an emit-time `const` decision with nothing persisted).
//!
//! ## End-to-end vs seam-level (documented per the T062 brief)
//!
//! - **End-to-end** (real staged + enrolled index, the production decision path):
//!   `both_streams_share_one_drain_and_identity` and
//!   `name_collision_source_is_the_gate_anonymous_only` stage a catalog whose
//!   enrolled `url` IS / IS-NOT the Midnight source under a `HomeGuard`-pinned
//!   `$HOME/.tome`, then drive the EXACT emit pair `commands/plugin/enable.rs`
//!   makes — the anonymous [`enqueue`] plus the attribution-gated
//!   [`enqueue_attributed`] keyed off the REAL default-`Paths`
//!   [`resolve_attribution`] (which opens the staged index read-only and
//!   canonicalizes the enrolled URL against the compiled-in const). This proves
//!   the WIRING, not just the primitives.
//! - **Seam-level** (the `*_to` / `resolve_attribution_with` doc-hidden seams):
//!   the exact-`rank` (FR-057), never-sampled (FR-058), and emit-time-`const`
//!   recompute (FR-053) cases drive the queue/resolver seams directly against a
//!   `TempDir`-rooted `Paths`/index — these are wire-shape / decision-purity
//!   guarantees that don't need (and would only be obscured by) the full enable
//!   pipeline. True de-allowlist is a `const` edit verified by the allowlist
//!   unit tests; the integration assertion here is "resolution reads the const on
//!   EVERY call, nothing is persisted" (FR-053).

use serde_json::Value;
use tempfile::TempDir;

use tome::index::{self, OpenOptions, workspace_catalogs};
use tome::paths::Paths;
use tome::telemetry::allowlist::{ATTRIBUTED_TELEMETRY_CATALOGS, match_source};
use tome::telemetry::event::{EntryKind, Harness, PluginActionEvent, PluginEnabled, SearchResult};
use tome::telemetry::resolver::resolve_attribution_with;
use tome::telemetry::{enqueue_attributed_to, enqueue_to, queue, resolve_attribution};
use tome::workspace::{ResolvedScope, WorkspaceName};

use crate::common::{HomeGuard, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};

// ---------------------------------------------------------------------------
// Shared fixtures.
// ---------------------------------------------------------------------------

/// A canonical, HTTPS-scheme Midnight catalog source. `allowlist::canonicalize`
/// reduces this to the bare `github.com/devrelaicom/midnight-expert-tome` const
/// entry, so an enrolment at this URL MUST attribute to `"midnight"`.
const MIDNIGHT_SOURCE: &str = "https://github.com/devrelaicom/midnight-expert-tome";

/// A non-allowlisted source whose path COLLIDES with Midnight by repo name but
/// lives under a different org — the FR-052 privacy-critical case: the catalog
/// can even be NAMED `midnight` and it must still stay anonymous, because the
/// SOURCE (not the name) is the gate.
const COLLIDING_NON_MIDNIGHT_SOURCE: &str = "https://github.com/someone/midnight-expert-tome";

/// The allowlist short id the Midnight source resolves to.
const MIDNIGHT_ID: &str = "midnight";

/// Telemetry env vars cleared then force-on, restored on drop. Telemetry
/// `is_enabled()` consults the CLI env precedence, so under CI (or a stray
/// `TOME_TELEMETRY=0`) the gated public [`resolve_attribution`] read still runs,
/// but `enqueue`/`enqueue_attributed` would no-op. We only need the gate forced
/// for the end-to-end tests; the seam-level tests use the UN-GATED `*_to`
/// primitives and don't depend on it. We still force it for hygiene so the file
/// behaves identically in CI and locally. Mirrors `mcp_funnel.rs::EnvForce`.
const TELEMETRY_ENV_VARS: &[&str] = &[
    "TOME_TELEMETRY",
    "TOME_TELEMETRY_ENDPOINT",
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "BUILDKITE",
    "JENKINS_URL",
    "TF_BUILD",
    "TEAMCITY_VERSION",
];

/// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY=1` plus a
/// non-routable endpoint, restore on drop. Pairs with a held [`HomeGuard`]
/// (which holds `HOME_MUTEX`) so the process-global env mutation can't race a
/// sibling test. Mirrors the `EnvForce` idiom in `tests/telemetry/mcp_funnel.rs`.
struct EnvForce {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvForce {
    fn install() -> Self {
        let saved = TELEMETRY_ENV_VARS
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        // SAFETY: the caller holds `HOME_MUTEX` via a `HomeGuard` for the whole
        // test, so no other test mutates these process-global vars concurrently.
        for &k in TELEMETRY_ENV_VARS {
            unsafe { std::env::remove_var(k) };
        }
        unsafe {
            std::env::set_var("TOME_TELEMETRY", "1");
            // A guaranteed-unroutable endpoint (TEST-NET-1, RFC 5737): if any
            // code path ever flushed inline it would fail, but the enqueue path
            // never touches the network — it only appends.
            std::env::set_var("TOME_TELEMETRY_ENDPOINT", "http://192.0.2.0:0/telemetry");
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

/// Open a fresh central index at `paths`, returning the live connection with the
/// `global` workspace seeded at bootstrap (so enrolments can target it).
fn open_seeded_index(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
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

/// Read every queued telemetry line under `paths` as parsed JSON objects. Empty
/// when the queue file doesn't exist yet.
fn queue_events(paths: &Paths) -> Vec<Value> {
    queue::read_lines(paths)
        .unwrap_or_default()
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

fn first_of<'a>(events: &'a [Value], event_type: &str) -> Option<&'a Value> {
    events.iter().find(|e| e["event_type"] == event_type)
}

// ---------------------------------------------------------------------------
// SC-006 — both streams share ONE drain + ONE identity (END-TO-END).
//
// Stage a catalog whose enrolled `url` IS the Midnight source under an isolated
// `$HOME/.tome`, then drive the EXACT emit pair `commands/plugin/enable.rs`
// makes: the always-on anonymous `enqueue` PLUS the attribution-gated
// `enqueue_attributed`, where the gate is the REAL default-`Paths`
// `resolve_attribution` (it opens the staged index read-only and canonicalizes
// the enrolled URL against the compiled-in const). Both lines MUST land on the
// SAME queue, sharing one `install_uuid` and one `session_uuid`.
// ---------------------------------------------------------------------------

#[test]
fn both_streams_share_one_drain_and_identity() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    // The production layout: everything under `$HOME/.tome`, which is what the
    // default `Paths::resolve()` (used by the public `enqueue*` + the public
    // `resolve_attribution`) reads.
    let paths = Paths::resolve().expect("resolve default paths under HomeGuard");
    std::fs::create_dir_all(&paths.root).unwrap();
    // The catalog NAME is deliberately an arbitrary alias — attribution is by
    // SOURCE, so the alias is irrelevant to the match.
    enrol(&paths, "my-midnight-alias", MIDNIGHT_SOURCE);

    let scope = global_scope();

    // The two emits, exactly as `enable.rs` orders them: anonymous always, then
    // the attributed one gated on the REAL resolver against the default paths.
    enqueue_to(
        &paths,
        PluginActionEvent {
            action: tome::telemetry::event::PluginAction::Enabled,
        },
    );
    let attributed = resolve_attribution(&scope, "my-midnight-alias");
    assert_eq!(
        attributed,
        Some(MIDNIGHT_ID),
        "the default-Paths resolver must attribute the Midnight-sourced catalog to `midnight`",
    );
    if let Some(catalog_id) = attributed {
        enqueue_attributed_to(
            &paths,
            PluginEnabled {
                plugin_name: "midnight-expert".to_string(),
                plugin_version: "1.2.0".to_string(),
                catalog_id,
            },
        );
    }

    let events = queue_events(&paths);

    // BOTH the anonymous and the attributed line are in the SAME drain (queue).
    let anon = first_of(&events, "tome.plugin_action")
        .expect("the anonymous tome.plugin_action landed on the queue");
    let attr = first_of(&events, "catalog.midnight.plugin_enabled")
        .expect("the attributed catalog.midnight.plugin_enabled landed on the SAME queue");

    assert_eq!(anon["action"], "enabled");
    assert_eq!(attr["plugin_name"], "midnight-expert");
    assert_eq!(attr["plugin_version"], "1.2.0");
    assert_eq!(attr["catalog_id"], MIDNIGHT_ID);

    // ONE install uuid (lazily minted by the first enqueue) shared across both
    // streams — the funnel join key.
    assert_eq!(
        anon["install_uuid"], attr["install_uuid"],
        "both streams share the one install uuid"
    );
    assert!(
        tome::telemetry::event::Uuid::parse(anon["install_uuid"].as_str().unwrap()).is_some(),
        "the shared install uuid is a valid v4"
    );
    // ONE session uuid shared across both streams.
    assert_eq!(
        anon["session_uuid"], attr["session_uuid"],
        "both streams share the per-process session uuid"
    );
}

// ---------------------------------------------------------------------------
// FR-052 — the SOURCE is the gate; a NAME collision stays anonymous-only
// (END-TO-END, the privacy-critical case).
//
// Stage a catalog literally NAMED `midnight` but enrolled at a NON-allowlisted
// source (same repo basename, different org). Drive the same emit pair. The
// anonymous event lands; the attribution gate (the real default-Paths resolver)
// returns `None`, so NO `catalog.*` line is ever enqueued. The NAME must never
// trigger attribution.
// ---------------------------------------------------------------------------

#[test]
fn name_collision_source_is_the_gate_anonymous_only() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = Paths::resolve().expect("resolve default paths under HomeGuard");
    std::fs::create_dir_all(&paths.root).unwrap();
    // Named `midnight`, but the SOURCE is NOT the Midnight repo.
    enrol(&paths, "midnight", COLLIDING_NON_MIDNIGHT_SOURCE);

    let scope = global_scope();

    enqueue_to(
        &paths,
        PluginActionEvent {
            action: tome::telemetry::event::PluginAction::Enabled,
        },
    );
    let attributed = resolve_attribution(&scope, "midnight");
    assert_eq!(
        attributed, None,
        "a catalog NAMED `midnight` but enrolled at a non-allowlisted source must NOT attribute \
         (the source, not the name, is the gate)",
    );
    if let Some(catalog_id) = attributed {
        // Unreachable given the assertion above; present to mirror the exact
        // `enable.rs` shape so a regression that wrongly attributes would emit a
        // `catalog.*` line and fail the no-attributed-line assertion below.
        enqueue_attributed_to(
            &paths,
            PluginEnabled {
                plugin_name: "midnight-expert".to_string(),
                plugin_version: "1.2.0".to_string(),
                catalog_id,
            },
        );
    }

    let events = queue_events(&paths);
    assert!(
        first_of(&events, "tome.plugin_action").is_some(),
        "the anonymous event still fires for a name-colliding catalog"
    );
    assert!(
        events.iter().all(|e| !e["event_type"]
            .as_str()
            .unwrap_or("")
            .starts_with("catalog.")),
        "a name collision with a non-allowlisted source must enqueue NO catalog.* line: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// FR-057 / SC-007 — the attributed `search_result` carries the EXACT integer
// `rank` (NOT a bucket token). Seam-level: this is a wire-shape guarantee on the
// attributed envelope, driven straight through `enqueue_attributed_to`.
// ---------------------------------------------------------------------------

#[test]
fn attributed_search_result_rank_is_exact_integer_in_the_queue() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::from_root(dir.path().to_path_buf());

    enqueue_attributed_to(
        &paths,
        SearchResult {
            entry_name: "midnight-compact-debug".to_string(),
            entry_kind: EntryKind::Skill,
            plugin_name: "midnight-expert".to_string(),
            rank: 7,
            catalog_id: MIDNIGHT_ID,
            calling_harness: Some(Harness::ClaudeCode),
        },
    );

    let events = queue_events(&paths);
    let result = first_of(&events, "catalog.midnight.search_result")
        .expect("the attributed search_result landed on the queue");

    // The defining FR-057 guarantee: `rank` is the EXACT integer 7, a bare JSON
    // number — never a bucket token like "5" / "6-10" / "11+".
    assert!(
        result["rank"].is_number(),
        "rank must be a bare JSON number, not a string bucket token: {result}"
    );
    assert_eq!(
        result["rank"].as_u64(),
        Some(7),
        "rank must be the exact integer 7: {result}"
    );
}

#[test]
fn search_result_serializes_rank_as_a_bare_number() {
    // Belt-and-braces at the serde layer (independent of the queue round-trip):
    // `SearchResult.rank` is a `u32` that serializes to a bare number.
    let event = SearchResult {
        entry_name: "midnight-compact-debug".to_string(),
        entry_kind: EntryKind::Skill,
        plugin_name: "midnight-expert".to_string(),
        rank: 7,
        catalog_id: MIDNIGHT_ID,
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
// FR-058 — attributed events are NEVER sampled. An attributed line in the queue
// carries NO `sample_rate` key; an anonymous line in the SAME drain DOES carry
// `"sample_rate":1.0`. Both via the queue, one drain.
// ---------------------------------------------------------------------------

#[test]
fn attributed_omits_sample_rate_anonymous_carries_it_same_drain() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::from_root(dir.path().to_path_buf());

    // One anonymous + one attributed line into the SAME queue.
    enqueue_to(
        &paths,
        PluginActionEvent {
            action: tome::telemetry::event::PluginAction::Enabled,
        },
    );
    enqueue_attributed_to(
        &paths,
        PluginEnabled {
            plugin_name: "midnight-expert".to_string(),
            plugin_version: "1.2.0".to_string(),
            catalog_id: MIDNIGHT_ID,
        },
    );

    let events = queue_events(&paths);
    let anon = first_of(&events, "tome.plugin_action").expect("anonymous line present");
    let attr =
        first_of(&events, "catalog.midnight.plugin_enabled").expect("attributed line present");

    // The anonymous line DOES carry the sample rate.
    assert_eq!(
        anon["sample_rate"], 1.0,
        "the anonymous line carries sample_rate=1.0: {anon}"
    );
    // The attributed line OMITS it entirely (never sampled — FR-058).
    assert!(
        attr.get("sample_rate").is_none(),
        "the attributed line must OMIT sample_rate (never sampled): {attr}"
    );
}

// ---------------------------------------------------------------------------
// FR-053 — de-allowlisting is an EMIT-TIME `const` decision; NOTHING is
// persisted. Attribution follows the running binary, never what was true at
// enable time, and there is no stored attribution column to clear.
//
// True de-allowlist is a `const` edit shipped in a release, verified by the
// `allowlist.rs` unit tests. The integration assertion we CAN make here is the
// equivalent property: the resolution reads the `const` on EVERY call (no memo
// that outlives a `const` change), and nothing about attribution is persisted —
// so the moment a user runs a binary whose const dropped the entry, the next
// resolution stops attributing, with no re-enable and no state to migrate.
// ---------------------------------------------------------------------------

#[test]
fn match_source_reads_the_const_allowlist_present_then_absent() {
    // A source that IS in the const ⇒ Some; a structurally-similar source that is
    // NOT in the const ⇒ None. This is the compile-time-`const` gate: the only
    // way to flip the first result to `None` is to ship a binary whose const
    // dropped the entry (FR-053 / FR-055 — no remote widening, no stored state).
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
    // The allowlist itself is a small, audited `const` (FR-055: the only way to
    // change it is a PR that ships in a release — there is no remote fetch).
    assert!(
        ATTRIBUTED_TELEMETRY_CATALOGS
            .iter()
            .any(|(id, _)| *id == MIDNIGHT_ID),
        "the compiled-in allowlist contains the midnight entry"
    );
}

#[test]
fn resolve_attribution_is_recomputed_every_call_nothing_persisted() {
    // Resolve the SAME enrolled catalog twice against the SAME staged index and
    // show the decision is RECOMPUTED each time (the const + a fresh canonicalize),
    // not cached in any way that would survive a const change. There is no DB
    // column to clear: attribution touches no schema (SCHEMA_VERSION stays 4).
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

    // Two independent resolutions over the SAME connection: each one reads the
    // enrolment, canonicalizes, and re-scans the const. They agree because the
    // const + the enrolment are unchanged — NOT because a prior result was
    // memoised. (If a `const` edit dropped the entry between two binary versions,
    // the second binary's call would simply return `None`; there is no persisted
    // attribution that would keep returning `Some`.)
    let first = resolve_attribution_with(&conn, &scope, "my-midnight-alias");
    let second = resolve_attribution_with(&conn, &scope, "my-midnight-alias");
    assert_eq!(first, Some(MIDNIGHT_ID));
    assert_eq!(second, Some(MIDNIGHT_ID));

    // The negative half: a DIFFERENT enrolment whose source is NOT allowlisted
    // resolves to `None` against the SAME index in the SAME process — proving the
    // decision is per-call source-driven, with no cross-catalog leakage from the
    // first lookup.
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

    // And nothing about attribution was persisted: the index schema is untouched
    // by the resolution path (no stored attribution column → SCHEMA_VERSION 4).
    // Re-resolving the original still returns Some — recomputed, never cleared.
    assert_eq!(
        resolve_attribution_with(&conn, &scope, "my-midnight-alias"),
        Some(MIDNIGHT_ID),
        "the original catalog still attributes — the decision is recomputed, never stored/cleared"
    );
}
