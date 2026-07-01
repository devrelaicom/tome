//! Integration tests for enhancement #309: `tome plugin list` / `tome plugin
//! show` surface a plugin's *last-indexed* and *last-upstream-change*
//! timestamps as human-relative durations instead of the old unconditional
//! `—` placeholder.
//!
//! Two axes:
//!   * When the catalog clone is a real git repo, `last_upstream_change` is
//!     populated from `git log -1 --format=%cI -- <plugin subtree>` and both
//!     the human view ("Last upstream change") and `--json`
//!     (`last_upstream_change`) carry a real timestamp.
//!   * When the catalog clone is NOT a git repo (the common test-fixture
//!     shape), the command still SUCCEEDS, `last_upstream_change` degrades to
//!     null / `—`, and the honest "Last indexed" value is still shown (never
//!     the bare placeholder for an enabled, indexed plugin).
//!
//! Read-only, no advisory lock — mirrors the existing list/show contract.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Run a `git` subcommand in `cwd`, panicking with captured stderr on failure.
/// Configures a deterministic identity so `git commit` never depends on the
/// host's global git config (CI runners frequently lack `user.email`).
fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Tome Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Tome Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Enable `plugin-alpha` against the (already-staged) catalog clone at
/// `catalog_root`, using the stub embedder so no model artefacts are needed.
fn enable_alpha(paths: &Paths, catalog_root: &Path, catalog_name: &str) {
    let cli_config = config_with_catalog(catalog_name, catalog_root);
    write_config_for_cli(paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = format!("{catalog_name}/plugin-alpha").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");
}

/// Stage the catalog fixture at the deterministic content-addressed clone dir
/// so `write_config_for_cli`'s symlink step is a no-op and OUR tree is used by
/// `resolve_plugin_dir`. When `as_git_repo` is true the staged tree is a real
/// git repo with one commit touching the whole catalog.
///
/// Returns `(paths, cache_root)`; the fixture temp dir is kept alive by the
/// caller.
fn stage(env: &ToolEnv, fixture_tmp: &TempDir, catalog_name: &str, as_git_repo: bool) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // The fixture lives in the TempDir; the enrolment URL is `file://<root>`.
    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");
    if as_git_repo {
        git(&catalog_root, &["init", "-q"]);
        git(&catalog_root, &["add", "-A"]);
        git(&catalog_root, &["commit", "-q", "-m", "seed catalog"]);
    }

    // Pre-stage `cache_dir_for(url)` as a symlink onto the fixture so the
    // helper below skips its own staging and `git log` runs against a tree
    // that has (or hasn't) a `.git`.
    let url = format!("file://{}", catalog_root.display());
    let cache_root: PathBuf = paths.cache_dir_for(&url);
    if let Some(parent) = cache_root.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&catalog_root, &cache_root).expect("symlink clone");

    enable_alpha(&paths, &catalog_root, catalog_name);
    paths
}

/// `— ` is the placeholder; anything else (e.g. "just now", "3d ago",
/// RFC-3339) counts as "populated".
fn is_placeholder(s: &str) -> bool {
    s.trim() == "—"
}

#[test]
fn show_json_populates_last_upstream_change_for_git_catalog() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(&env, &fixture_tmp, "git-catalog", true);

    let out = env
        .cmd()
        .args(["plugin", "show", "git-catalog/plugin-alpha", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");

    // `last_upstream_change` is a real RFC-3339 string, not null.
    let ts = v["last_upstream_change"].as_str();
    assert!(
        ts.is_some_and(|s| s.contains('T')),
        "last_upstream_change must be a populated RFC-3339 timestamp for a git \
         catalog, got {}",
        v["last_upstream_change"],
    );
    // `last_indexed_at` remains a real timestamp (raw ISO in JSON, unchanged).
    assert!(
        v["last_indexed_at"]
            .as_str()
            .is_some_and(|s| s.contains('T')),
        "last_indexed_at must stay a raw ISO timestamp in JSON, got {}",
        v["last_indexed_at"],
    );
}

#[test]
fn show_human_shows_relative_times_not_placeholder_for_git_catalog() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(&env, &fixture_tmp, "git-catalog", true);

    let out = env
        .cmd()
        .args(["plugin", "show", "git-catalog/plugin-alpha"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let text = String::from_utf8_lossy(&out.stdout);

    // Both honest lines are present and neither is the bare placeholder.
    let indexed = text
        .lines()
        .find_map(|l| l.strip_prefix("Last indexed:"))
        .expect("`Last indexed:` line present")
        .trim();
    let upstream = text
        .lines()
        .find_map(|l| l.strip_prefix("Last upstream change:"))
        .expect("`Last upstream change:` line present")
        .trim();

    // "Last indexed" carries the relative time + author (` — <author>`); take
    // the relative-time head before the em-dash separator.
    let indexed_rel = indexed.split(" — ").next().unwrap_or(indexed).trim();
    assert!(
        !is_placeholder(indexed_rel),
        "Last indexed must be a relative time, not `—`; line was {indexed:?}",
    );
    assert!(
        !is_placeholder(upstream),
        "Last upstream change must be a relative time for a git catalog, not \
         `—`; line was {upstream:?}",
    );
    // Sanity: the old unconditional `—` placeholder must not survive on either.
    assert!(
        !text.contains("Last updated: —"),
        "the old `Last updated: —` placeholder must be gone; output:\n{text}",
    );
}

#[test]
fn list_human_shows_last_upstream_change_column_for_git_catalog() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(&env, &fixture_tmp, "git-catalog", true);

    let out = env
        .cmd()
        .args(["plugin", "list"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Last upstream change"),
        "the `Last upstream change` column header must be present; output:\n{text}",
    );
    // The enabled plugin-alpha row shows a relative "ago" / "just now" value in
    // both time columns (a fresh commit + fresh index → "just now").
    assert!(
        text.contains("just now") || text.contains("ago"),
        "an enabled+indexed git plugin must render a relative time, not only \
         `—`; output:\n{text}",
    );
}

#[test]
fn non_git_catalog_degrades_gracefully_and_still_shows_last_indexed() {
    // The degrade path: a non-git catalog clone. `last_upstream_change` is
    // null / `—`, but the command SUCCEEDS and the honest `last_indexed_at`
    // value is still populated (never the bare placeholder for an enabled,
    // indexed plugin).
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(&env, &fixture_tmp, "plain-catalog", false);

    // JSON: last_upstream_change degrades to null, last_indexed_at stays real.
    let out = env
        .cmd()
        .args(["plugin", "show", "plain-catalog/plugin-alpha", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "show must succeed on a non-git catalog: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(
        v["last_upstream_change"].is_null(),
        "last_upstream_change degrades to null for a non-git catalog, got {}",
        v["last_upstream_change"],
    );
    assert!(
        v["last_indexed_at"]
            .as_str()
            .is_some_and(|s| s.contains('T')),
        "last_indexed_at must still be populated, got {}",
        v["last_indexed_at"],
    );

    // Human: "Last indexed" is a relative time; "Last upstream change" is `—`.
    let human = env
        .cmd()
        .args(["plugin", "show", "plain-catalog/plugin-alpha"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    let indexed = text
        .lines()
        .find_map(|l| l.strip_prefix("Last indexed:"))
        .expect("`Last indexed:` line present")
        .trim();
    let indexed_rel = indexed.split(" — ").next().unwrap_or(indexed).trim();
    assert!(
        !is_placeholder(indexed_rel),
        "Last indexed must be a relative time even for a non-git catalog; line \
         was {indexed:?}",
    );
    let upstream = text
        .lines()
        .find_map(|l| l.strip_prefix("Last upstream change:"))
        .expect("`Last upstream change:` line present")
        .trim();
    assert!(
        is_placeholder(upstream),
        "Last upstream change is `—` when the catalog clone has no git history; \
         line was {upstream:?}",
    );
}
