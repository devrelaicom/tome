//! Phase 10 / US2 (T042) — queue mechanics + the headline zero-foreground-network
//! proof, exercised across the crate boundary against `tome::telemetry::*`.
//!
//! Two families live here:
//!
//! 1. **Queue mechanics** (`queue::{append,read_lines,rewrite,classify_lines}`):
//!    multi-writer no-interleave on a local FS, the ≥4096 B per-line drop, the
//!    1 MiB soft-cap drop, FIFO `rewrite`, and the read-only `classify_lines`
//!    corrupt count (the inspect-side behaviour is covered end-to-end in
//!    `inspect.rs`; here we assert the library primitive is read-only).
//!
//! 2. **The HEADLINE no-foreground-network proof** (SC-004 / NFR-001): the
//!    network counter is a per-PROCESS static, so this MUST run in-process — a
//!    spawned binary would have its own counter. With telemetry force-enabled and
//!    an isolated `$HOME`, we drive the CLI-foreground enqueue path AND the
//!    in-process MCP tool path and assert `network_call_count()` does not move.

use tempfile::TempDir;
use tome::paths::Paths;
use tome::telemetry::queue;

use std::sync::Arc;

// The MCP-tool zero-network test stages a catalog via a symlinked cache dir (the
// standard in-process MCP staging shape), so its imports + scaffolding are
// Unix-only like the `mcp_funnel` peer. The CLI-foreground test + queue mechanics
// below are cross-platform.
#[cfg(unix)]
use {
    serde_json::Value,
    std::collections::HashMap,
    std::path::Path,
    tokio::sync::OnceCell,
    tome::embedding::Reranker,
    tome::embedding::registry::{ModelEntry, ModelKind},
    tome::embedding::stub::{StubEmbedder, StubReranker},
    tome::index::{self, OpenOptions},
    tome::mcp::prompts::PromptRegistry,
    tome::mcp::state::McpState,
    tome::mcp::tools::search_skills,
    tome::plugin::PluginId,
    tome::plugin::lifecycle::{self, LifecycleDeps},
    tome::workspace::{ResolvedScope, Scope, WorkspaceName},
};

use tome::telemetry::event::{Install, InstallMethod};
use tome::telemetry::transport::{network_call_count, record_network_call};

use crate::common::HomeGuard;
#[cfg(unix)]
use crate::common::{
    config_with_catalog, fabricate_models, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

/// The hard per-line cap (incl. trailing `\n`), mirrored from `queue.rs` — a
/// line whose `len() + 1` exceeds this is dropped, never split.
const MAX_LINE_BYTES: usize = 4096;
/// The 1 MiB soft queue cap, mirrored from `queue.rs`.
const MAX_QUEUE_BYTES: u64 = 1_048_576;

fn paths_in(dir: &TempDir) -> Paths {
    Paths::from_root(dir.path().to_path_buf())
}

// ===========================================================================
// 1. Queue mechanics
// ===========================================================================

/// Multi-writer append concurrency (local-FS no-interleave): N threads each
/// append M distinct lines into ONE queue; afterwards EVERY line is one of the
/// expected complete lines (no torn/interleaved fragment) and the total count is
/// exactly N×M. `append` re-opens the `O_APPEND` fd per call, so this proves the
/// single-`write` no-interleave guarantee on a local filesystem.
#[test]
fn multi_writer_append_never_interleaves() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 64;

    let dir = TempDir::new().unwrap();
    let paths = Arc::new(paths_in(&dir));

    // Each thread writes lines tagged with its id + sequence so every expected
    // line is distinct and reconstructable. Lines are short (well under 4096 B).
    let expected: std::collections::HashSet<String> = (0..THREADS)
        .flat_map(|t| (0..PER_THREAD).map(move |i| format!("{{\"t\":{t},\"i\":{i}}}")))
        .collect();

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let paths = Arc::clone(&paths);
            std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let line = format!("{{\"t\":{t},\"i\":{i}}}");
                    queue::append(&paths, &line).expect("append ok");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("writer thread joined");
    }

    let lines = queue::read_lines(&paths).expect("read_lines ok");
    assert_eq!(
        lines.len(),
        THREADS * PER_THREAD,
        "FIFO total must be exactly N×M with no lost or duplicated line"
    );

    // No line is torn or interleaved: every observed line is exactly one of the
    // distinct expected lines, and each appears exactly once.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for line in &lines {
        assert!(
            expected.contains(line),
            "observed a torn/interleaved line not in the expected set: {line:?}"
        );
        assert!(seen.insert(line), "a line appeared twice: {line:?}");
    }
    assert_eq!(
        seen.len(),
        expected.len(),
        "every expected line landed once"
    );
}

/// T-M2 — multi-writer no-interleave at the BOUNDARY the single-`write`
/// atomicity guarantee is scoped to: each appended line is just UNDER 4096 bytes
/// (incl. its newline) and distinct per (thread, iteration), under contention.
/// Afterwards every read line is one complete expected line (no torn/interleaved
/// fragment) and each appears exactly once. The near-max line size is the point:
/// it exercises the largest single `write` the queue permits, where a torn line
/// would be most likely if the append weren't one atomic syscall.
#[test]
fn multi_writer_near_max_line_never_interleaves() {
    const THREADS: usize = 6;
    const PER_THREAD: usize = 24;
    // Target byte length of each line INCLUDING the trailing newline `append`
    // adds: one under the cap so the line is kept, not dropped.
    const TARGET_WITH_NL: usize = MAX_LINE_BYTES - 1; // 4095
    // The line body length (excluding the newline `append` appends).
    const BODY_LEN: usize = TARGET_WITH_NL - 1; // 4094

    let dir = TempDir::new().unwrap();
    let paths = Arc::new(paths_in(&dir));

    // Build one distinct near-max line per (thread, iteration): a JSON object
    // whose `t`/`i` make it unique, padded with a `p` filler to BODY_LEN so the
    // whole line (plus newline) is exactly one under the cap.
    let line_for = |t: usize, i: usize| -> String {
        let prefix = format!("{{\"t\":{t},\"i\":{i},\"p\":\"");
        let suffix = "\"}";
        // pad so prefix + pad + suffix == BODY_LEN.
        let pad_len = BODY_LEN - prefix.len() - suffix.len();
        let mut s = String::with_capacity(BODY_LEN);
        s.push_str(&prefix);
        s.extend(std::iter::repeat_n('z', pad_len));
        s.push_str(suffix);
        debug_assert_eq!(s.len(), BODY_LEN);
        debug_assert_eq!(s.len() + 1, TARGET_WITH_NL, "line+nl is one under the cap");
        s
    };

    let expected: std::collections::HashSet<String> = (0..THREADS)
        .flat_map(|t| (0..PER_THREAD).map(move |i| line_for(t, i)))
        .collect();

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let paths = Arc::clone(&paths);
            std::thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let line = line_for(t, i);
                    // Each is one under the cap ⇒ kept (not dropped).
                    queue::append(&paths, &line).expect("near-max append ok");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("writer thread joined");
    }

    let lines = queue::read_lines(&paths).expect("read_lines ok");
    assert_eq!(
        lines.len(),
        THREADS * PER_THREAD,
        "every near-max line landed exactly once (no drop, no split)",
    );
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for line in &lines {
        assert!(
            expected.contains(line),
            "observed a torn/interleaved near-max line not in the expected set (len {})",
            line.len(),
        );
        assert!(seen.insert(line), "a near-max line appeared twice");
    }
    assert_eq!(
        seen.len(),
        expected.len(),
        "every expected near-max line once"
    );
}

/// A line at/over the 4096-byte cap (including the appended newline) is dropped,
/// never split: the queue is left byte-for-byte unchanged.
#[test]
fn oversize_line_is_dropped_not_split() {
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);

    // Seed one good line so we can prove byte-identity across the dropped append.
    queue::append(&paths, "{\"ok\":true}").expect("seed append");
    let before = std::fs::read(paths.telemetry_queue()).expect("read queue");

    // `len() + 1` (newline) == 4097 > 4096 ⇒ dropped.
    let huge = "x".repeat(MAX_LINE_BYTES);
    assert_eq!(
        huge.len() + 1,
        MAX_LINE_BYTES + 1,
        "fixture is over the cap"
    );
    queue::append(&paths, &huge).expect("oversize append returns Ok (dropped)");

    let after = std::fs::read(paths.telemetry_queue()).expect("read queue");
    assert_eq!(
        before, after,
        "an oversize line must not touch the queue file"
    );
    // And it was not split into a partial line.
    assert_eq!(
        queue::read_lines(&paths).unwrap(),
        vec!["{\"ok\":true}".to_string()],
        "only the original good line remains"
    );
}

/// Filling the queue near 1 MiB then appending one more line that would cross the
/// cap drops the new line silently: the file size stays ≤ cap and the pre-cap
/// lines are untouched.
#[test]
fn over_one_mib_append_is_dropped_silently() {
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);

    // ~2 KiB filler lines; 600 of them ≈ 1.2 MiB, putting the queue over the cap.
    let filler_line = format!("{{\"x\":\"{}\"}}", "z".repeat(2000));
    let filler: Vec<String> = std::iter::repeat_n(filler_line, 600).collect();
    queue::rewrite(&paths, &filler).expect("seed via rewrite");

    let size_before = std::fs::metadata(paths.telemetry_queue()).unwrap().len();
    assert!(
        size_before > MAX_QUEUE_BYTES,
        "fixture must exceed the 1 MiB cap (was {size_before})"
    );
    let count_before = queue::count_pending(&paths);

    // The queue is already over the cap ⇒ this append is silently dropped.
    queue::append(&paths, "{\"overflow\":true}").expect("overflow append returns Ok");

    let size_after = std::fs::metadata(paths.telemetry_queue()).unwrap().len();
    assert_eq!(size_after, size_before, "over-cap append must add no bytes");
    assert_eq!(
        queue::count_pending(&paths),
        count_before,
        "over-cap append adds no line"
    );
    // The pre-cap lines all remain.
    assert_eq!(queue::read_lines(&paths).unwrap().len(), 600);
}

/// FIFO `rewrite`: seeding lines then rewriting with a kept subset leaves the
/// file as exactly that subset, in order, 0600.
#[test]
fn rewrite_keeps_exactly_the_subset_in_order() {
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);

    for n in 1..=5 {
        queue::append(&paths, &format!("{{\"n\":{n}}}")).expect("append");
    }

    // Keep lines 2, 3, 5 in their original order (a FIFO post-drain survivor set).
    let kept = vec![
        "{\"n\":2}".to_string(),
        "{\"n\":3}".to_string(),
        "{\"n\":5}".to_string(),
    ];
    queue::rewrite(&paths, &kept).expect("rewrite");

    assert_eq!(
        queue::read_lines(&paths).unwrap(),
        kept,
        "rewrite leaves exactly the kept subset in order"
    );
    // Exact bytes: the three lines, each newline-terminated, nothing else.
    let body = std::fs::read_to_string(paths.telemetry_queue()).unwrap();
    assert_eq!(body, "{\"n\":2}\n{\"n\":3}\n{\"n\":5}\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(paths.telemetry_queue())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "rewritten queue must be 0600");
    }
}

/// `classify_lines` is read-only + counts corrupt lines: a corrupt entry yields
/// `corrupt > 0`, and the queue file is left byte-identical (self-heal is the
/// flusher's job, not the classifier's). This is the LIBRARY-primitive analogue
/// of the end-to-end read-only assertions in `inspect.rs` — kept here so the
/// queue module's own contract is pinned without re-driving the CLI surface.
#[test]
fn classify_lines_counts_corrupt_and_is_read_only() {
    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    std::fs::create_dir_all(paths.telemetry_dir()).unwrap();

    let seeded = "{\"a\":1}\nthis is not json\n{\"b\":2}\n";
    std::fs::write(paths.telemetry_queue(), seeded).unwrap();
    let before = std::fs::read(paths.telemetry_queue()).unwrap();

    let (values, corrupt) = queue::classify_lines(&paths).expect("classify ok");
    assert_eq!(values.len(), 2, "two parsable lines");
    assert_eq!(corrupt, 1, "one corrupt (unparsable) line");

    let after = std::fs::read(paths.telemetry_queue()).unwrap();
    assert_eq!(
        before, after,
        "classify_lines must not mutate the queue (read-only; no self-heal here)"
    );
}

// ===========================================================================
// 2. HEADLINE — zero foreground network (SC-004, NFR-001)
//
// The network counter (`transport::NETWORK_CALLS`) is a per-PROCESS static; the
// only site that increments it is the US3 POST. Both tests below drive a
// FOREGROUND emit path and assert the counter is UNCHANGED — the structural
// proof that no foreground path performs network I/O.
// ===========================================================================

/// T-C1 [negative control] — the seam the `== before` foreground proofs depend
/// on is LOAD-BEARING: snapshotting the counter, calling `record_network_call()`
/// exactly once, and asserting the LOCAL delta is exactly 1.
///
/// Without this, the `does_no_network` tests above could pass vacuously against a
/// counter that never moves (e.g. if the seam were accidentally a no-op). This
/// proves the counter increments, so those `after == before` assertions are
/// FALSIFIABLE by contrast — a foreground path that networked WOULD move the
/// counter and be caught.
///
/// Race-safety: `NETWORK_CALLS` is a process-global `AtomicU64` shared with the
/// foreground `== before` tests, which run in parallel threads of this same
/// binary. This test's `+1` increment would corrupt a foreground test's
/// `before→after` window if the two overlapped (the foreground delta would read
/// `+1` and fail). To make the shared counter safe WITHOUT relying on ordering,
/// this test acquires the SAME `HOME_MUTEX` the foreground tests hold (via a
/// `HomeGuard`), so it can never run concurrently with any foreground emit path.
/// We assert only the LOCAL delta (`after - before == 1`), never an absolute
/// value, so the snapshot stays correct regardless of the counter's prior value.
#[test]
fn network_counter_seam_is_load_bearing() {
    // Hold HOME_MUTEX for the snapshot window so no foreground network test
    // (all of which hold it via their own `HomeGuard`) can interleave our `+1`.
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());

    let before = network_call_count();
    record_network_call();
    let after = network_call_count();
    assert_eq!(
        after - before,
        1,
        "record_network_call must move the counter by exactly 1 — \
         the seam the foreground no-network proofs rely on",
    );
}

/// CLI-foreground path: the `enqueue` library entry (the foreground emit path)
/// appends a local line and performs NO network I/O — the counter is unchanged.
///
/// Driven through the DEFAULT-`Paths` `enqueue` (resolving `$HOME/.tome` via a
/// `HomeGuard`-pinned tempdir, telemetry force-enabled), exactly the call site
/// `main.rs` uses, so this is the real foreground path — not just the
/// path-injected `enqueue_to`. `enqueue_to` (also a foreground path) is exercised
/// against an explicit `Paths` for good measure.
///
/// FALSIFIABILITY: this asserts a `before`/`after` DELTA of zero (never an
/// absolute `0`), so the process-global counter is never assumed pristine — a
/// sibling test (`network_counter_seam_is_load_bearing`) deliberately moves it.
/// That negative control proves the seam this `== before` relies on actually
/// increments, so a foreground path that DID network would be caught here by
/// contrast. The end-to-end proof completes in US3, when the `reqwest::blocking`
/// POST becomes the one `record_network_call` increment site; until then this
/// negative control guards the seam.
#[test]
fn cli_foreground_enqueue_does_no_network() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let before = network_call_count();

    // The real default-Paths foreground entry (gated on is_enabled → forced on).
    tome::telemetry::enqueue(Install {
        install_method: InstallMethod::Cargo,
    });
    tome::telemetry::enqueue(Install {
        install_method: InstallMethod::Brew,
    });

    // And the path-injected sibling foreground path against an explicit Paths.
    let root = home.path().join(".tome");
    let paths = Paths::from_root(root);
    tome::telemetry::enqueue_to(
        &paths,
        Install {
            install_method: InstallMethod::Curl,
        },
    );

    assert_eq!(
        network_call_count(),
        before,
        "the foreground enqueue path must perform ZERO network calls"
    );

    // Sanity: the foreground path DID append (so we know the no-network claim is
    // about a path that actually ran, not a silently-skipped no-op).
    let lines = queue::read_lines(&paths).expect("read default-home queue");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"event_type\":\"tome.install\"")),
        "at least one tome.install line landed on the queue: {lines:?}"
    );
}

/// MCP-tool foreground path: an in-process `search_skills` handler call emits
/// `tome.search` via enqueue-only and performs NO network I/O — even with a
/// non-routable endpoint configured. Mirrors the `mcp_funnel.rs` staging.
///
/// FALSIFIABILITY: same `before`/`after` DELTA discipline as the CLI test above —
/// `network_counter_seam_is_load_bearing` proves the counter moves, so this
/// `== before` is a real (falsifiable) no-network claim, not a vacuous one. The
/// US3 POST is the increment site that closes the end-to-end proof.
#[cfg(unix)]
#[test]
fn mcp_tool_foreground_call_does_no_network() {
    let home = TempDir::new().unwrap();
    let _home_guard = HomeGuard::install(home.path());
    let _env = EnvForce::install();

    let paths = stage_at_home(
        home.path(),
        &[("alpha", &skill_body("alpha", "alpha widget configuration"))],
    );
    let state = build_state(&paths, Some("claude-code"));
    let rt = rt();

    let before = network_call_count();

    let _ = rt
        .block_on(search_skills::handle(
            state.clone(),
            search_skills::Input {
                query: "alpha widget configuration".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search ok");

    assert_eq!(
        network_call_count(),
        before,
        "the MCP tool handler must perform ZERO network calls on the enqueue path"
    );
    // Sanity: the search event was enqueued (the foreground path actually ran).
    assert!(
        queue_events(&paths)
            .iter()
            .any(|e| e["event_type"] == "tome.search"),
        "the search event must still be enqueued"
    );
}

// ---------------------------------------------------------------------------
// Shared MCP-staging scaffolding for the zero-network MCP test. Lifted from the
// `mcp_funnel.rs` shape (same StubEmbedder-rooted-at-`$HOME/.tome` setup) so the
// handler's default-`Paths` enqueue lands where we read it.
// ---------------------------------------------------------------------------

#[cfg(unix)]
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

#[cfg(unix)]
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

/// Telemetry/CI env vars cleared before forcing the state we want.
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
/// (held for the whole test) so the env mutation can't race a sibling.
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
            // A guaranteed-unroutable endpoint (TEST-NET-1, RFC 5737): if the
            // foreground path ever tried to flush inline this would hang/fail.
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
fn skill_body(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nBody for {name}.\n")
}

#[cfg(unix)]
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

#[cfg(unix)]
fn build_state(paths: &Paths, host_harness: Option<&str>) -> Arc<McpState> {
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry: &STUB_EMBEDDER_ENTRY,
        reranker_entry: &STUB_RERANKER_ENTRY,
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(PromptRegistry::default()))),
        host_harness: host_harness.map(str::to_owned),
        last_search_ranks: std::sync::Mutex::new(HashMap::new()),
        flush_signal: std::sync::Arc::new(tokio::sync::Notify::new()),
        enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
    })
}

#[cfg(unix)]
fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

#[cfg(unix)]
fn queue_events(paths: &Paths) -> Vec<Value> {
    queue::read_lines(paths)
        .unwrap_or_default()
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).expect("queued line is JSON"))
        .collect()
}
