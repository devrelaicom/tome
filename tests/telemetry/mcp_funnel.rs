//! Phase 10 / US2 (FR-027/FR-028) — the MCP-surface anonymous funnel emits,
//! driven END-TO-END through the in-process MCP harness against a real staged
//! + indexed workspace (StubEmbedder/StubReranker — no ONNX).
//!
//! The MCP handlers enqueue against the DEFAULT `Paths` (`$HOME/.tome`), not
//! `state.paths` — `telemetry::enqueue` resolves paths internally and self-gates
//! on `is_enabled()`. So the whole staged tree is rooted at `$HOME/.tome` (via a
//! `HomeGuard`-pinned tempdir) and telemetry is force-enabled (`TOME_TELEMETRY=1`,
//! overriding any CI auto-off), so the handler emits land in the SAME isolated
//! home the test reads the queue from.
//!
//! What this asserts:
//! - `search_skills` → `tome.search` (`surface = "mcp"`) lands on the queue.
//! - a following `get_skill` / `get_skill_info` → the funnel event
//!   (`tome.entry_invoked` / `tome.entry_info`) carries a NON-`none`
//!   `rank_bucket` for the just-ranked entry, sharing ONE `session_uuid` with
//!   the search.
//! - `calling_harness` on the MCP events reflects the host harness.
//! - the handler path does NO network: even with a NON-ROUTABLE telemetry
//!   endpoint set, the tool call returns promptly (enqueue only — never flush).
//!
//! The narrow rank-tracking + `calling_harness` mapper unit coverage lives in
//! `src/mcp/` (the lib unit tests); this is the assembled-surface integration.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::{ModelEntry, ModelKind};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::state::McpState;
use tome::mcp::tools::{get_skill, get_skill_info, search_skills};
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, WorkspaceName};

use crate::common::{
    HomeGuard, config_with_catalog, fabricate_models, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Env hygiene: force telemetry ON for the spawned-in-process emit path.
//
// `telemetry::is_enabled()` consults the SAME env precedence the CLI uses, so
// under CI (or with a stray `TOME_TELEMETRY=0`) the silent enqueue would be a
// no-op. We force `TOME_TELEMETRY=1` (overrides CI) for the duration of the
// test and restore on drop, holding `HOME_MUTEX` (via `HomeGuard`) so no
// sibling test races `$HOME`/env. The endpoint var is set to a non-routable
// address to prove the handler never touches the network (it only appends).
// ---------------------------------------------------------------------------

static STUB_EMBEDDER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-embedder",
    version: "0",
    kind: ModelKind::Embedder,
    source_url: "stub://embedder",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    files: &[],
    aux_urls: &[],
};

static STUB_RERANKER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-reranker",
    version: "0",
    kind: ModelKind::Reranker,
    source_url: "stub://reranker",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    files: &[],
    aux_urls: &[],
};

/// Telemetry/CI env vars cleared before forcing the state we want, so the test
/// is deterministic regardless of the host/CI environment. Restored on drop by
/// [`EnvForce`].
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
/// non-routable endpoint, restore everything on drop. Pairs with a `HomeGuard`
/// (held for the whole test) so the env mutation can't race a sibling — both
/// are process-global. Mirrors the `EnvGuard` idiom in `telemetry/config.rs`.
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
            // A guaranteed-unroutable endpoint: TEST-NET-1 (RFC 5737) port 0.
            // If the handler ever tried to flush inline, this would hang/fail;
            // it must NOT, because enqueue only appends.
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

// ---------------------------------------------------------------------------
// Staging — rooted at `$HOME/.tome` so the handlers' default-`Paths` enqueue
// lands where we read it.
// ---------------------------------------------------------------------------

fn open_index(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index db")
}

fn seed_catalog_enrolment(paths: &Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        std::fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
    }
}

/// A skill SKILL.md body with a `name`/`description`, tuned so the StubEmbedder
/// distinguishes the entries enough to give a deterministic ranking.
fn skill_body(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nBody for {name}.\n")
}

/// Stage `acme/plug` with the supplied skills rooted at `home/.tome`, enabled +
/// indexed against the `global` workspace with the StubEmbedder. Returns the
/// `Paths` (rooted at `$HOME/.tome`) the in-process server will run over.
fn stage_at_home(home: &Path, skills: &[(&str, &str)]) -> Paths {
    let root = home.join(".tome");
    let paths = Paths::from_root(root.clone());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = home.join("catalog");
    std::fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);

    let plugin_dir = catalog_root.join("plug");
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        "name = \"plug\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();
    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    let embedder = StubEmbedder::new();
    let scope = Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plugin");

    paths
}

/// Build the `Arc<McpState>` over the staged `paths`, with `host_harness` set so
/// the `calling_harness` dimension on the MCP events is populated.
fn build_state(paths: &Paths, host_harness: Option<&str>) -> Arc<McpState> {
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry: &STUB_EMBEDDER_ENTRY,
        reranker_entry: &STUB_RERANKER_ENTRY,
        prompt_registry: Arc::new(PromptRegistry::default()),
        host_harness: host_harness.map(str::to_owned),
        last_search_ranks: std::sync::Mutex::new(HashMap::new()),
    })
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// Read every queued telemetry line under the isolated `$HOME/.tome` as parsed
/// JSON objects. Empty when the queue file doesn't exist yet.
fn queue_events(paths: &Paths) -> Vec<Value> {
    tome::telemetry::queue::read_lines(paths)
        .unwrap_or_default()
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}

fn first_of(events: &[Value], event_type: &str) -> Option<Value> {
    events
        .iter()
        .find(|e| e["event_type"] == event_type)
        .cloned()
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn search_then_get_skill_emits_funnel_with_shared_session_and_rank() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    // Two distinct skills so the search returns a ranked list with a clear
    // top entry. The StubEmbedder ranks deterministically by content overlap.
    let paths = stage_at_home(
        home.path(),
        &[
            ("alpha", &skill_body("alpha", "alpha widget configuration")),
            ("beta", &skill_body("beta", "beta gadget tuning")),
        ],
    );
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    // 1. search → `tome.search` (surface=mcp).
    let search_out = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect("search ok");
    assert!(
        !search_out.matches.is_empty(),
        "search must return at least one ranked entry to attribute a rank"
    );
    let top_entry = search_out.matches[0].name.clone();

    // 2. get_skill on the top-ranked entry → `tome.entry_invoked`.
    let _ = rt
        .block_on(get_skill::handle(
            state.clone(),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: top_entry.clone(),
            },
        ))
        .expect("get_skill ok");

    let events = queue_events(&paths);

    // The search event landed with the MCP surface + calling harness.
    let search = first_of(&events, "tome.search").expect("tome.search enqueued");
    assert_eq!(search["surface"], "mcp", "search surface is mcp: {search}");
    assert_eq!(
        search["calling_harness"], "claude-code",
        "search carries the host harness: {search}"
    );
    assert_eq!(
        search["candidates_returned"], "1-4",
        "two results bucket to 1-4: {search}"
    );

    // The funnel event landed with a NON-`none` rank for the selected entry.
    let invoked = first_of(&events, "tome.entry_invoked").expect("tome.entry_invoked enqueued");
    assert_eq!(invoked["entry_kind"], "skill", "get_skill is skill-kind");
    assert_eq!(
        invoked["calling_harness"], "claude-code",
        "entry_invoked carries the host harness: {invoked}"
    );
    assert_ne!(
        invoked["rank_bucket"], "none",
        "the just-searched + selected entry must carry a real rank: {invoked}"
    );
    // The top entry sat at rank 1.
    assert_eq!(
        invoked["rank_bucket"], "1",
        "the rank-1 result selected via get_skill buckets to `1`: {invoked}"
    );

    // Both share ONE session uuid (the funnel join key).
    assert_eq!(
        search["session_uuid"], invoked["session_uuid"],
        "search + funnel events share the per-process session uuid"
    );
    // And one install uuid (lazily minted by the first enqueue, AC#7).
    assert_eq!(search["install_uuid"], invoked["install_uuid"]);
    assert!(
        tome::telemetry::event::Uuid::parse(search["install_uuid"].as_str().unwrap()).is_some(),
        "install uuid is a valid v4"
    );
}

#[test]
fn get_skill_info_emits_entry_info_with_rank() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[
            ("alpha", &skill_body("alpha", "alpha widget configuration")),
            ("beta", &skill_body("beta", "beta gadget tuning")),
        ],
    );
    let state = build_state(&paths, Some("cursor"));
    let rt = rt();

    let search_out = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect("search ok");
    let top_entry = search_out.matches[0].name.clone();

    let _ = rt
        .block_on(get_skill_info::handle(
            state.clone(),
            get_skill_info::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: top_entry,
                kind: EntryKind::Skill,
            },
        ))
        .expect("get_skill_info ok");

    let events = queue_events(&paths);
    let info = first_of(&events, "tome.entry_info").expect("tome.entry_info enqueued");
    assert_eq!(
        info["calling_harness"], "cursor",
        "entry_info carries the host harness: {info}"
    );
    assert_ne!(
        info["rank_bucket"], "none",
        "the just-searched entry must carry a real rank on entry_info: {info}"
    );
    // Sanity: the search event uses the same session uuid.
    let search = first_of(&events, "tome.search").unwrap();
    assert_eq!(search["session_uuid"], info["session_uuid"]);
}

#[test]
fn get_skill_without_preceding_search_has_none_rank() {
    // A bare `get_skill` (no search this session) → `rank_bucket = none`.
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let state = build_state(&paths, Some("codex"));
    let rt = rt();

    let _ = rt
        .block_on(get_skill::handle(
            state.clone(),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "alpha".into(),
            },
        ))
        .expect("get_skill ok");

    let events = queue_events(&paths);
    let invoked = first_of(&events, "tome.entry_invoked").expect("entry_invoked enqueued");
    assert_eq!(
        invoked["rank_bucket"], "none",
        "no preceding search ⇒ rank_bucket is none: {invoked}"
    );
    assert_eq!(invoked["calling_harness"], "codex");
    // No `tome.search` event was produced.
    assert!(
        first_of(&events, "tome.search").is_none(),
        "no search ran ⇒ no tome.search event"
    );
}

#[test]
fn unknown_host_harness_omits_calling_harness() {
    // An unmapped host id ⇒ `calling_harness` resolves to None ⇒ the optional
    // field is OMITTED from the wire shape (never a guessed enum value).
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let state = build_state(&paths, Some("not-a-real-harness"));
    let rt = rt();

    let _ = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect("search ok");

    let events = queue_events(&paths);
    let search = first_of(&events, "tome.search").expect("tome.search enqueued");
    assert!(
        search.get("calling_harness").is_none(),
        "an unmapped host harness must OMIT calling_harness, not guess: {search}"
    );
}

#[test]
fn tool_call_returns_promptly_with_nonroutable_endpoint() {
    // SC-009: the handler path does NO network — even with a non-routable
    // telemetry endpoint configured (set in `EnvForce`), the tool call returns
    // promptly because enqueue only appends a local line (the flush is US3 and
    // lives off the handler path entirely).
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    let started = Instant::now();
    let _ = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: 10,
                catalog: None,
                plugin: None,
                description_max_chars: 150,
            },
        ))
        .expect("search ok");
    let elapsed = started.elapsed();

    // A network round-trip to a non-routable host would take seconds (connect
    // timeout) at minimum. The whole stub-embedder search + enqueue is well
    // under a second; give generous headroom for slow CI without masking a
    // real inline-network regression.
    assert!(
        elapsed < Duration::from_secs(2),
        "handler must not touch the network on the enqueue path; took {elapsed:?}"
    );
    // The event still landed (the append happened).
    assert!(
        first_of(&queue_events(&paths), "tome.search").is_some(),
        "the search event must still be enqueued"
    );
}
