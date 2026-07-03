//! The MCP-surface anonymous + attributed funnel emits, driven END-TO-END
//! through the in-process MCP harness against a real staged + indexed workspace
//! (StubEmbedder/StubReranker — no ONNX).
//!
//! Re-homed onto the `gauge-telemetry` kernel: the MCP handlers route through the
//! process-global `telemetry::emit`, which is set-once. A `TelemetryHandleGuard`
//! installs an ENABLED handle pointed at the staged `$HOME/.tome` queue for the
//! test, so the handler emits land where the test reads them. The produced line is
//! the kernel `QueuedEvent` (`event_name` + nested `attributes`); quantities are
//! RAW integers (no bucket tokens), the attributed catalog event names are
//! `tome.catalog_*` with a `catalog` attribute (was the old `catalog.<id>.*`
//! envelope name + `catalog_id` field), and there is NO per-line install/session
//! uuid (the kernel attaches identity as OTLP resource attributes at drain only).
//!
//! What this asserts:
//! - `search_skills` → `tome.search` (`surface = "mcp"`) lands on the queue.
//! - a following `get_skill` / `get_skill_info` → the funnel event
//!   (`tome.entry_invoked` / `tome.entry_info`) carries the EXACT integer `rank`
//!   for the just-ranked entry.
//! - `calling_harness` on the MCP events reflects the host harness.
//! - an ALLOWLISTED catalog also emits the attributed `tome.catalog_search_result`
//!   / `tome.catalog_error` stream ALONGSIDE the anonymous one.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use crate::queue_util::{LOOPBACK_ENDPOINT, TELEMETRY_ENV_VARS, first_named, queue_events};

static STUB_EMBEDDER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-embedder",
    version: "0",
    kind: ModelKind::Embedder,
    source_url: "stub://embedder",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    embedding_dim: Some(384),
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
    embedding_dim: None,
    files: &[],
    aux_urls: &[],
};

/// Snapshot + clear the telemetry/CI env vars, force `TOME_TELEMETRY=1` plus a
/// loopback endpoint (so `build_handle_for_test` builds an ENABLED handle),
/// restore on drop. Pairs with a `HomeGuard` held for the whole test.
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

fn open_index(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index db")
}

/// The canonical Midnight catalog source — exactly what the compiled-in allowlist
/// canonicalizes to. An enrolment recorded at this URL attributes to `"midnight"`.
const MIDNIGHT_SOURCE: &str = "https://github.com/devrelaicom/midnight-expert-tome";

fn seed_catalog_enrolment(paths: &Paths, catalog_root: &Path, catalog_name: &str, enrol_url: &str) {
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, enrol_url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(enrol_url);
    if let Some(parent) = cache_dir.parent() {
        std::fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
    }
}

fn skill_body(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nBody for {name}.\n")
}

/// Stage `acme/plug` rooted at `home/.tome`, enabled + indexed against the
/// `global` workspace. The catalog is enrolled at a plain `file://` URL (NOT
/// allowlisted ⇒ anonymous only).
fn stage_at_home(home: &Path, skills: &[(&str, &str)]) -> Paths {
    let catalog_root = home.join("catalog");
    let file_url = format!("file://{}", catalog_root.display());
    stage_at_home_with_url(home, skills, &file_url)
}

/// Path-injectable [`stage_at_home`] recording the enrolment at `enrol_url` — pass
/// [`MIDNIGHT_SOURCE`] to make the catalog allowlisted (emits the attributed
/// `tome.catalog_*` stream).
fn stage_at_home_with_url(home: &Path, skills: &[(&str, &str)], enrol_url: &str) -> Paths {
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
    seed_catalog_enrolment(&paths, &catalog_root, "acme", enrol_url);
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
        embedder_seed: tome::index::MetaSeed {
            name: STUB_EMBEDDER_ENTRY.name.into(),
            version: STUB_EMBEDDER_ENTRY.version.into(),
        },
        reranker_entry: &STUB_RERANKER_ENTRY,
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(PromptRegistry::default()))),
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

/// Install the process-global emit override pointed at the staged queue. The
/// returned guard must outlive the tool calls so their emits land here.
fn install_handle(paths: &Paths) -> tome::telemetry::TelemetryHandleGuard {
    tome::telemetry::TelemetryHandleGuard::install(tome::telemetry::build_handle_for_test(paths))
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn search_then_get_skill_emits_funnel_with_rank() {
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
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    // 1. search → `tome.search` (surface=mcp).
    let search_out = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
                raw: false,
            },
        ))
        .expect("get_skill ok");

    let events = queue_events(&paths);

    // The search event landed with the MCP surface + calling harness.
    let search = first_named(&events, "tome.search").expect("tome.search enqueued");
    let sa = &search["attributes"];
    assert_eq!(sa["surface"], "mcp", "search surface is mcp: {search}");
    assert_eq!(
        sa["calling_harness"], "claude-code",
        "search carries the host harness: {search}"
    );
    assert_eq!(
        sa["candidates_returned"], 2,
        "two results report a raw count of 2: {search}"
    );

    // The funnel event landed with the EXACT rank for the selected entry.
    let invoked = first_named(&events, "tome.entry_invoked").expect("tome.entry_invoked enqueued");
    let ia = &invoked["attributes"];
    assert_eq!(ia["entry_kind"], "skill", "get_skill is skill-kind");
    assert_eq!(
        ia["calling_harness"], "claude-code",
        "entry_invoked carries the host harness: {invoked}"
    );
    // The top entry sat at rank 1 (exact integer, never bucketed).
    assert_eq!(
        ia["rank"], 1,
        "the rank-1 result selected via get_skill: {invoked}"
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
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("cursor"));
    let rt = rt();

    let search_out = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
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
    let info = first_named(&events, "tome.entry_info").expect("tome.entry_info enqueued");
    let a = &info["attributes"];
    assert_eq!(
        a["calling_harness"], "cursor",
        "entry_info carries the host harness: {info}"
    );
    assert!(
        a["rank"].as_u64().unwrap_or(0) >= 1,
        "the just-searched entry must carry a real rank on entry_info: {info}"
    );
}

#[test]
fn get_skill_without_preceding_search_has_zero_rank() {
    // A bare `get_skill` (no search this session) → `rank = 0`.
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("codex"));
    let rt = rt();

    let _ = rt
        .block_on(get_skill::handle(
            state.clone(),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "alpha".into(),
                raw: false,
            },
        ))
        .expect("get_skill ok");

    let events = queue_events(&paths);
    let invoked = first_named(&events, "tome.entry_invoked").expect("entry_invoked enqueued");
    let a = &invoked["attributes"];
    assert_eq!(a["rank"], 0, "no preceding search ⇒ rank is 0: {invoked}");
    assert_eq!(a["calling_harness"], "codex");
    assert!(
        first_named(&events, "tome.search").is_none(),
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
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("not-a-real-harness"));
    let rt = rt();

    let _ = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search ok");

    let events = queue_events(&paths);
    let search = first_named(&events, "tome.search").expect("tome.search enqueued");
    assert!(
        search["attributes"].get("calling_harness").is_none(),
        "an unmapped host harness must OMIT calling_harness, not guess: {search}"
    );
}

#[test]
fn tool_call_returns_promptly_with_loopback_endpoint() {
    // The handler path does NO network — even with the kernel endpoint configured
    // (loopback, in `EnvForce`), the tool call returns promptly because emit only
    // appends a local line (delivery is off the handler path).
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    let started = Instant::now();
    let _ = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search ok");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "handler must not touch the network on the enqueue path; took {elapsed:?}"
    );
    assert!(
        first_named(&queue_events(&paths), "tome.search").is_some(),
        "the search event must still be enqueued"
    );
}

#[test]
fn search_on_allowlisted_catalog_emits_attributed_search_result() {
    // A search over an ALLOWLISTED catalog emits, on the SAME queue as the
    // anonymous `tome.search`, one attributed `tome.catalog_search_result` per
    // ranked entry with an EXACT 1-indexed `rank` and the `catalog` attribute.
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home_with_url(
        home.path(),
        &[
            ("alpha", &skill_body("alpha", "alpha widget configuration")),
            ("beta", &skill_body("beta", "beta gadget tuning")),
        ],
        MIDNIGHT_SOURCE,
    );
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    let out = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                min_score: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search ok");
    assert!(!out.matches.is_empty(), "search returns ranked entries");
    let top_name = out.matches[0].name.clone();

    let events = queue_events(&paths);
    assert!(
        first_named(&events, "tome.search").is_some(),
        "anonymous tome.search still enqueued alongside the attributed stream"
    );
    let result = events
        .iter()
        .find(|e| {
            e["event_name"] == "tome.catalog_search_result"
                && e["attributes"]["entry_name"] == top_name
        })
        .cloned()
        .expect("an attributed tome.catalog_search_result for the top entry");
    let a = &result["attributes"];
    // EXACT 1-indexed integer rank (never bucketed).
    assert_eq!(
        a["rank"], 1,
        "the rank-1 entry's attributed search_result carries the exact integer rank 1: {result}"
    );
    assert_eq!(a["catalog"], "midnight");
    assert_eq!(
        a["calling_harness"], "claude-code",
        "the MCP search_result carries the host harness"
    );
}

#[test]
fn post_resolution_get_skill_failure_on_allowlisted_entry_emits_attributed_error() {
    // A get_skill failure that occurs AFTER the entry row resolved (the SKILL.md
    // is deleted post-index, so the read fails with `skill_file_missing`) emits,
    // on an ALLOWLISTED catalog, the attributed `tome.catalog_error` — carrying
    // the resolved plugin/entry names + the captured `plugin_version` — ALONGSIDE
    // the anonymous `tome.error`.
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home_with_url(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
        MIDNIGHT_SOURCE,
    );
    let _handle = install_handle(&paths);
    let state = build_state(&paths, Some("claude-code"));

    // Delete the on-disk SKILL.md AFTER indexing so the row still resolves but the
    // post-resolution read fails → `skill_file_missing`.
    let skill_md = home
        .path()
        .join("catalog")
        .join("plug")
        .join("skills")
        .join("alpha")
        .join("SKILL.md");
    std::fs::remove_file(&skill_md).expect("delete the staged SKILL.md");

    let rt = rt();
    let res = rt.block_on(get_skill::handle(
        state.clone(),
        get_skill::Input {
            catalog: "acme".into(),
            plugin: "plug".into(),
            name: "alpha".into(),
            raw: false,
        },
    ));
    assert!(
        res.is_err(),
        "get_skill must fail once the resolved entry's SKILL.md is gone"
    );

    let events = queue_events(&paths);
    // The anonymous MCP-surface error landed.
    let anon = first_named(&events, "tome.error").expect("anonymous tome.error enqueued");
    assert_eq!(
        anon["attributes"]["surface"], "mcp",
        "the anonymous error is MCP-surface"
    );

    // The attributed error landed on the SAME queue with the resolved names +
    // version (the entry resolved before the read failed).
    let attr = first_named(&events, "tome.catalog_error").expect("attributed tome.catalog_error");
    let a = &attr["attributes"];
    assert_eq!(a["plugin_name"], "plug");
    assert_eq!(a["entry_name"], "alpha");
    assert_eq!(a["plugin_version"], "1.0.0");
    assert_eq!(a["catalog"], "midnight");
    // A skill-file-missing read failure maps to the `io` error class.
    assert_eq!(a["error_class"], "io");
}
