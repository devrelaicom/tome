//! Phase 7 / SC-008 — README getting-started smoke test.
//!
//! The README's getting-started section is the project's front door (FR-021).
//! This suite REBUILDS the `tome` binary (Cargo does that for us via
//! `CARGO_BIN_EXE_tome`) and runs *every* getting-started command against a
//! `file://` local-catalog fixture, asserting each resolves to its documented
//! exit code / outcome. Using a local fixture decouples the check from the
//! availability of the public catalog the README's worked example names
//! (`devrelaicom/midnight-expert-tome`), per the spec assumption: the README
//! launch is gated on that catalog being public; this automated check is not.
//!
//! ## Why the model-dependent commands are split out
//!
//! `tome plugin enable` and `tome query` load the real `FastembedEmbedder`,
//! and on a fresh machine `enable`'s registry-wide model-presence gate offers
//! to download every pinned model — embedder + reranker + summariser, ~804 MB
//! total. Downloading those in the fast path would make the suite slow,
//! non-deterministic, and network-dependent — exactly what a CI smoke test
//! must not be. So the fast path covers their CLI wiring (`--help` parses +
//! exits 0); the real end-to-end run lives in a single `#[ignore]`d test that a
//! human can opt into with `cargo test --test readme_smoke -- --ignored`.
//!
//! Every other getting-started command is exercised end-to-end against the
//! fixture with fabricated (sparse, free) model manifests so the
//! model-presence gates pass without a download. The index DB is seeded with
//! the production `MODEL_REGISTRY` identities so `tome status` sees no embedder
//! drift and reports a clean `Ok`.

use std::fs;
use std::path::Path;

use crate::common::{
    Fixture, ToolEnv, fabricate_all_registry_models, has_global_enrolment, paths_for,
};

/// The fixture catalog's manifest `name` (see
/// `tests/fixtures/sample-plugin-catalog/tome-catalog.toml`). Stands in for the
/// README's `midnight-expert-tome`.
const CATALOG: &str = "sample-plugin-catalog";
/// A plugin the fixture catalog offers (see its `tome-catalog.toml`). Stands in
/// for the README's `compact-expert`.
const PLUGIN: &str = "plugin-alpha";

/// Seed the central DB with the production `MODEL_REGISTRY` identities so a
/// later `tome status` open agrees on embedder/reranker/summariser identity and
/// reports no drift. Mirrors `exit_codes_e2e.rs::seed_workspace_with_registry_seeds`
/// but without inserting a named workspace — the privileged `global` workspace
/// (seeded by `schema::bootstrap`) is all the getting-started flow needs.
fn open_with_registry_seeds(paths: &tome::paths::Paths) {
    let (embedder, reranker, summariser) = tome::commands::plugin::registry_seeds();
    let _ = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder,
            reranker,
            summariser,
            profile: None,
        },
    )
    .expect("open index db with registry seeds");
}

/// Assert a `tome` invocation exited 0, surfacing stdout+stderr on failure.
fn assert_ok(label: &str, out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "README command `{label}` must resolve (exit 0), got {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Build a fixture catalog + isolated env with fabricated models and a
/// registry-seeded DB, then register the catalog via the real `catalog add`.
/// Returns the env + fixture (both must outlive the test) and the project dir
/// used for workspace/harness commands.
fn setup() -> (ToolEnv, Fixture, std::path::PathBuf) {
    let fix = Fixture::build_from(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-plugin-catalog"),
    );
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("tome root");

    // Fabricate sparse model artefacts + manifests so model-presence gates pass
    // without a download, and stamp `meta` with the registry identities.
    fabricate_all_registry_models(&paths);
    open_with_registry_seeds(&paths);

    // A project directory under HOME (avoids the cwd-is-home refusal) for the
    // workspace-bind + harness commands later in the walkthrough.
    let project = env.home_path().join("demo-project");
    fs::create_dir_all(&project).expect("create project dir");

    (env, fix, project)
}

#[test]
fn getting_started_catalog_flow_resolves() {
    let (env, fix, _project) = setup();
    let paths = paths_for(&env);

    // 1. `tome catalog add <url>` — the real command against the fixture.
    let add = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn catalog add");
    assert_ok("catalog add", &add);
    assert!(
        has_global_enrolment(&paths, CATALOG),
        "catalog add must enrol `{CATALOG}` in the central DB",
    );

    // `tome catalog list` — confirm it registered.
    let list = env
        .cmd()
        .args(["catalog", "list"])
        .output()
        .expect("spawn catalog list");
    assert_ok("catalog list", &list);
    assert!(
        String::from_utf8_lossy(&list.stdout).contains(CATALOG),
        "catalog list must show `{CATALOG}`",
    );

    // 2. `tome catalog show <name>` — lists the catalog's plugins (resolves via
    //    the central DB, the discovery path the README points users at).
    let show = env
        .cmd()
        .args(["catalog", "show", CATALOG])
        .output()
        .expect("spawn catalog show");
    assert_ok("catalog show", &show);
    let show_out = String::from_utf8_lossy(&show.stdout);
    assert!(
        show_out.contains(PLUGIN),
        "catalog show must list plugin `{PLUGIN}`; got:\n{show_out}",
    );
}

#[test]
fn getting_started_plugin_list_resolves() {
    // `tome plugin list` (bare) — with a catalog enrolled, lists its plugins
    // (all disabled) before any enable, which is the documented exit-0 outcome.
    let (env, _fix, _project) = setup();
    let out = env
        .cmd()
        .args(["plugin", "list"])
        .output()
        .expect("spawn plugin list");
    assert_ok("plugin list", &out);
}

#[test]
fn getting_started_status_models_reindex_resolve() {
    let (env, fix, _project) = setup();
    // Register the catalog so the maintenance commands have something to see.
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn catalog add");

    // `tome status` — a health verdict. With fabricated models + registry
    // seeds the verdict is `Ok` (exit 0).
    let status = env.cmd().args(["status"]).output().expect("spawn status");
    assert_ok("status", &status);

    // `tome models list` — read-only listing.
    let models = env
        .cmd()
        .args(["models", "list"])
        .output()
        .expect("spawn models list");
    assert_ok("models list", &models);

    // `tome reindex` — no enabled plugins → "Nothing to reindex" (exit 0).
    let reindex = env.cmd().args(["reindex"]).output().expect("spawn reindex");
    assert_ok("reindex", &reindex);
}

#[test]
fn getting_started_workspace_and_harness_flow_resolves() {
    let (env, _fix, project) = setup();

    // `tome workspace init my-project`.
    let init = env
        .cmd()
        .args(["workspace", "init", "my-project"])
        .output()
        .expect("spawn workspace init");
    assert_ok("workspace init", &init);

    // `tome workspace list`.
    let list = env
        .cmd()
        .args(["workspace", "list"])
        .output()
        .expect("spawn workspace list");
    assert_ok("workspace list", &list);

    // `tome workspace use my-project` — from inside the project dir. Writes the
    // `.tome/config.toml` marker and runs harness sync (no harnesses declared
    // yet → a clean no-op sync).
    let use_ws = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "use", "my-project"])
        .output()
        .expect("spawn workspace use");
    assert_ok("workspace use", &use_ws);
    assert!(
        project.join(".tome/config.toml").is_file(),
        "workspace use must write the project marker",
    );

    // `tome workspace current` — from inside the now-bound project dir it
    // prints JUST the bound name on one line and exits 0 (the prompt/script
    // contract documented in the README).
    let current = env
        .cmd()
        .current_dir(&project)
        .args(["workspace", "current"])
        .output()
        .expect("spawn workspace current");
    assert_ok("workspace current", &current);
    assert_eq!(
        String::from_utf8_lossy(&current.stdout),
        "my-project\n",
        "workspace current must print the bare bound name",
    );

    // `tome harness` (bare) — list the supported harnesses.
    let harness_bare = env
        .cmd()
        .current_dir(&project)
        .args(["harness"])
        .output()
        .expect("spawn harness");
    assert_ok("harness", &harness_bare);

    // `tome harness use claude-code` — defaults to project scope; the marker
    // written by `workspace use` makes the project scope resolvable.
    let harness_use = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "use", "claude-code"])
        .output()
        .expect("spawn harness use");
    assert_ok("harness use", &harness_use);

    // `tome harness list` — effective list with the source chain.
    let harness_list = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "list"])
        .output()
        .expect("spawn harness list");
    assert_ok("harness list", &harness_list);

    // `tome sync` — reconcile rules + harness files; byte-for-byte idempotent.
    let sync = env
        .cmd()
        .current_dir(&project)
        .args(["sync"])
        .output()
        .expect("spawn sync");
    assert_ok("sync", &sync);
}

#[test]
fn bare_tome_help_shows_getting_started_quickstart() {
    // #293: bare `tome --help` must surface a 3-step getting-started block so a
    // first-time user has an order to follow, not just a flat command list.
    let env = ToolEnv::new();
    let out = env.cmd().args(["--help"]).output().expect("spawn --help");
    assert_ok("--help", &out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Getting started:"),
        "expected a getting-started block in `tome --help`, got:\n{stdout}",
    );
    for needle in ["tome catalog add", "tome plugin enable", "tome query"] {
        assert!(
            stdout.contains(needle),
            "quickstart must mention `{needle}`, got:\n{stdout}",
        );
    }
}

#[test]
fn getting_started_model_dependent_commands_parse() {
    // `tome plugin enable`, `tome query`, and `tome mcp` all load the real
    // embedder, prompt to download every pinned model (~804 MB on a fresh
    // machine), or run a long-lived server, so the fast path covers their CLI
    // wiring: `--help` parses and exits 0. The real end-to-end run is the
    // `#[ignore]`d test below.
    let env = ToolEnv::new();
    for (label, args) in [
        ("plugin enable --help", &["plugin", "enable", "--help"][..]),
        ("query --help", &["query", "--help"][..]),
        ("mcp --help", &["mcp", "--help"][..]),
    ] {
        let out = env.cmd().args(args).output().expect("spawn --help");
        assert_ok(label, &out);
    }
}

/// Full end-to-end of the model-dependent getting-started commands against the
/// fixture. This test does NOT fabricate model manifests, so the registry-wide
/// presence gate in `plugin enable --yes` downloads every pinned model — the
/// embedder + reranker + summariser, ~804 MB total. Excluded from the default
/// run for CI speed + determinism; opt in with:
///
/// ```sh
/// cargo test --test readme_smoke -- --ignored
/// ```
///
/// This exercises `tome plugin enable <catalog>/<plugin>` then `tome query`,
/// asserting each resolves against the published model artefacts (SC-008's
/// strongest form).
#[test]
#[ignore = "downloads ~804 MB of pinned models (embedder + reranker + summariser); run explicitly with --ignored"]
fn getting_started_plugin_enable_and_query_with_real_models() {
    let fix = Fixture::build_from(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-plugin-catalog"),
    );
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).expect("tome root");

    // Register the catalog AND seed config.toml so the discovery path (which
    // `plugin enable` consults via `resolve_plugin_dir`) finds the plugin dir.
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("catalog add");
    let cache = cache_dir_for(env.home_path(), &fix.url);
    let cfg = format!(
        "[catalogs.{CATALOG}]\nname = \"{CATALOG}\"\nurl = \"{url}\"\nref = \"main\"\n\
         path = \"{path}\"\nlast_synced = \"2026-06-02T00:00:00Z\"\n",
        url = fix.url,
        path = cache.display(),
    );
    fs::write(&paths.global_config_file, cfg).expect("seed config.toml");

    // `tome plugin enable <catalog>/<plugin> --yes` — the registry-wide
    // presence gate downloads all three pinned models on first run, then
    // indexes. `--yes` allows the download from this non-TTY context.
    let enable = env
        .cmd()
        .args(["plugin", "enable", &format!("{CATALOG}/{PLUGIN}"), "--yes"])
        .output()
        .expect("spawn plugin enable");
    assert_ok("plugin enable", &enable);

    // `tome query "..."` — semantic search across the now-indexed plugin.
    let query = env
        .cmd()
        .args(["query", "alpha widget"])
        .output()
        .expect("spawn query");
    assert_ok("query", &query);
}

/// Content-addressed cache dir for a catalog URL (sha256 hex), matching the
/// binary's layout. Duplicated locally because `crate::common::cache_dir_for` takes a
/// `ToolEnv` and we want the raw form here.
fn cache_dir_for(home: &Path, url: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    home.join(".tome/catalogs").join(hex::encode(h.finalize()))
}
