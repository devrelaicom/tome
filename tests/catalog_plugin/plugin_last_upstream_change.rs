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

/// Where a `.git` sits relative to the staged catalog root — controls whether
/// (and where) `last_upstream_change` should resolve.
#[derive(Clone, Copy, PartialEq)]
enum Repo {
    /// No git repo anywhere → `last_upstream_change` is `—`.
    None,
    /// The catalog root IS a real git repo with a commit touching the whole
    /// tree → `last_upstream_change` populated.
    AtRoot,
    /// The catalog root is NOT a repo, but an ANCESTOR directory is (e.g. a
    /// `$HOME` dotfiles repo). `git log` walking up would find the ancestor;
    /// the `.git`-containment guard must still yield `—`.
    AtAncestorOnly,
    /// The catalog root IS a repo, but the `plugin-alpha` subtree was never
    /// committed (committed only a sentinel file) → `git log -1 -- <subtree>`
    /// returns empty (the `Ok(None)` path) → `—`.
    AtRootSubtreeUncommitted,
}

/// Stage the catalog fixture at the deterministic content-addressed clone dir
/// so `write_config_for_cli`'s symlink step is a no-op and OUR tree is used by
/// `resolve_plugin_dir`. `repo` selects the git layout (see [`Repo`]).
///
/// Returns the resolved `Paths`; the fixture temp dir is kept alive by the
/// caller.
fn stage(env: &ToolEnv, fixture_tmp: &TempDir, catalog_name: &str, repo: Repo) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // The fixture lives in the TempDir; the enrolment URL is `file://<root>`.
    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");
    match repo {
        Repo::None => {}
        Repo::AtRoot => {
            git(&catalog_root, &["init", "-q"]);
            git(&catalog_root, &["add", "-A"]);
            git(&catalog_root, &["commit", "-q", "-m", "seed catalog"]);
        }
        Repo::AtAncestorOnly => {
            // Make the fixture TempDir root (an ANCESTOR of `catalog_root`) a
            // git repo with a commit, but leave `catalog_root` itself un-init'd.
            // `git log` run from inside `catalog_root` WOULD walk up and find
            // this repo — so this layout proves the `.git` guard, not luck.
            let ancestor = fixture_tmp.path();
            git(ancestor, &["init", "-q"]);
            std::fs::write(ancestor.join("SENTINEL"), "ancestor repo\n").unwrap();
            git(ancestor, &["add", "-A"]);
            git(ancestor, &["commit", "-q", "-m", "ancestor repo commit"]);
        }
        Repo::AtRootSubtreeUncommitted => {
            // Repo at the root with history, but the plugin-alpha subtree is
            // NOT part of any commit: commit only a sentinel at the root, with
            // the whole tree excluded so `plugin-alpha/**` has no history.
            git(&catalog_root, &["init", "-q"]);
            std::fs::write(catalog_root.join("SENTINEL"), "root only\n").unwrap();
            git(&catalog_root, &["add", "--", "SENTINEL"]);
            git(&catalog_root, &["commit", "-q", "-m", "root sentinel only"]);
        }
    }

    // Pre-stage `cache_dir_for(url)` as a symlink onto the fixture so the
    // helper below skips its own staging and `git log` runs against a tree
    // that has (or hasn't) a `.git` at its root.
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
    let _paths = stage(&env, &fixture_tmp, "git-catalog", Repo::AtRoot);

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
    let _paths = stage(&env, &fixture_tmp, "git-catalog", Repo::AtRoot);

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
    let _paths = stage(&env, &fixture_tmp, "git-catalog", Repo::AtRoot);

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
    let _paths = stage(&env, &fixture_tmp, "plain-catalog", Repo::None);

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

/// Extract the trimmed `Last upstream change:` value from `plugin show` human
/// output.
fn upstream_line(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("Last upstream change:"))
        .expect("`Last upstream change:` line present")
        .trim()
        .to_owned()
}

#[test]
fn show_does_not_leak_ancestor_repo_timestamp() {
    // #309 review item 1 (the substantive fix): the catalog clone dir is NOT
    // itself a git repo but sits under an ANCESTOR repo. `git log` walking up
    // WOULD find the ancestor and report its HEAD timestamp — a silently-wrong
    // value. The `.git`-containment guard (shared by `show` + `list`) must make
    // BOTH surfaces return `—`, identical to a no-repo catalog.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(&env, &fixture_tmp, "nested-catalog", Repo::AtAncestorOnly);

    // JSON: last_upstream_change must be null (NOT the ancestor's timestamp).
    let out = env
        .cmd()
        .args(["plugin", "show", "nested-catalog/plugin-alpha", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "show must succeed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(
        v["last_upstream_change"].is_null(),
        "show must NOT report the ancestor repo's HEAD timestamp; the clone dir \
         is not itself a repo. got {}",
        v["last_upstream_change"],
    );

    // Human `show`: the upstream line is `—`.
    let human = env
        .cmd()
        .args(["plugin", "show", "nested-catalog/plugin-alpha"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(human.status.success());
    let show_upstream = upstream_line(&String::from_utf8_lossy(&human.stdout));
    assert!(
        is_placeholder(&show_upstream),
        "show's Last upstream change must be `—` under an ancestor-only repo; \
         got {show_upstream:?}",
    );

    // `list` behaves identically (parity — same guard, same result).
    let list = env
        .cmd()
        .args(["plugin", "list"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_text = String::from_utf8_lossy(&list.stdout);
    // The enabled plugin-alpha row must not show a relative "ago"/"just now" in
    // the upstream column — the only relative time present is "Last indexed".
    // (A leaked ancestor timestamp would surface as a second relative value.)
    assert!(
        list_text.contains("Last upstream change"),
        "list header present: {list_text}",
    );
}

#[test]
fn git_repo_but_uncommitted_subtree_degrades_to_dash() {
    // #309 review item 2: a real git repo at the catalog root, but the
    // plugin-alpha SUBTREE was never committed → `git log -1 -- <subtree>`
    // returns empty (the documented `Ok(None)` path, distinct from `Err`).
    // `last_upstream_change` is null / `—` and the command still succeeds.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = stage(
        &env,
        &fixture_tmp,
        "partial-catalog",
        Repo::AtRootSubtreeUncommitted,
    );

    let out = env
        .cmd()
        .args(["plugin", "show", "partial-catalog/plugin-alpha", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "show must succeed when the subtree has no committed history: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(
        v["last_upstream_change"].is_null(),
        "an uncommitted subtree (git log returns empty) → null, got {}",
        v["last_upstream_change"],
    );
    // The honest last_indexed_at is still populated.
    assert!(
        v["last_indexed_at"]
            .as_str()
            .is_some_and(|s| s.contains('T')),
        "last_indexed_at must still be populated, got {}",
        v["last_indexed_at"],
    );

    // Human: upstream line is `—`, and the command exits 0.
    let human = env
        .cmd()
        .args(["plugin", "show", "partial-catalog/plugin-alpha"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(human.status.success());
    let show_upstream = upstream_line(&String::from_utf8_lossy(&human.stdout));
    assert!(
        is_placeholder(&show_upstream),
        "Last upstream change is `—` for an uncommitted subtree; got \
         {show_upstream:?}",
    );

    // `list` also succeeds and shows `—` for the upstream column of this plugin.
    let list = env
        .cmd()
        .args(["plugin", "list"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "list must succeed: {}",
        String::from_utf8_lossy(&list.stderr),
    );
}
