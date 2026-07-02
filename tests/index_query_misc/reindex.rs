//! Phase 7 / US5 slice 3 — `tome reindex [<scope>] [--force]`.
//!
//! Library-API tests for the explicit reindex subcommand. The aggregate
//! output / NDJSON record is asserted via the `run_with_deps` entry point
//! (which is a thin wrapper around `execute` + `emit`). The CLI binary
//! path is exercised only for the parse-error / unknown-catalog cases
//! that don't need an embedder — same boundary as plugin enable.

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};
use tempfile::TempDir;
use tome::commands::reindex::{Scope, run_with_deps};
use tome::embedding::stub::StubEmbedder;
use tome::output::Mode;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

fn enable_alpha_and_beta(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) {
    let alpha: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let beta: PluginId = "sample-plugin-catalog/plugin-beta".parse().unwrap();
    let deps = LifecycleDeps {
        paths,
        scope: shared_scope(),
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&alpha, &deps).expect("enable alpha");
    lifecycle::enable(&beta, &deps).expect("enable beta");
}

/// Returns a static reference to the global Scope so it can be embedded
/// in `LifecycleDeps<'a>` without a lifetime issue. The lock is fine for
/// tests — every test in this binary shares the same global-scope value.
fn shared_scope() -> &'static tome::workspace::Scope {
    static SCOPE: std::sync::OnceLock<tome::workspace::Scope> = std::sync::OnceLock::new();
    SCOPE.get_or_init(|| tome::workspace::Scope(tome::workspace::WorkspaceName::global()))
}

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope: shared_scope(),
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    }
}

#[test]
fn reindex_all_visits_every_enabled_plugin_zero_changes() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    let plugins = vec![
        "sample-plugin-catalog/plugin-alpha".parse().unwrap(),
        "sample-plugin-catalog/plugin-beta".parse().unwrap(),
    ];
    let agg = run_with_deps(
        Scope::All,
        &plugins,
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex all");

    assert_eq!(agg.plugins_visited, 2);
    // plugin-alpha has 4 valid skills, plugin-beta has 3 valid skills.
    assert_eq!(agg.skills_checked, 5);
    assert_eq!(agg.skills_re_embedded, 0);
    assert_eq!(agg.skills_unchanged, 5);
    assert_eq!(
        embedder.call_count(),
        baseline,
        "no skill changed — no embed call should fire",
    );
}

#[test]
fn reindex_one_plugin_re_embeds_only_modified_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    // Mutate skill-a in plugin-alpha.
    let skill_a = catalog_root
        .join("plugin-alpha")
        .join("skills")
        .join("skill-a")
        .join("SKILL.md");
    std::fs::write(
        &skill_a,
        "---\nname: skill-a\ndescription: rewritten by slice 3\n---\nbody\n",
    )
    .unwrap();

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let agg = run_with_deps(
        Scope::Plugin(id.clone()),
        &[id],
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex one plugin");

    assert_eq!(agg.plugins_visited, 1);
    assert_eq!(agg.skills_checked, 4);
    assert_eq!(agg.skills_re_embedded, 1);
    assert_eq!(agg.skills_unchanged, 3);
    assert_eq!(
        embedder.call_count() - baseline,
        1,
        "exactly one embed call for the modified skill",
    );
}

#[test]
fn reindex_re_embeds_when_only_when_to_use_changes() {
    // US4.b (T317): pins the wiring that US1.a established —
    // `when_to_use` participates in `content_hash`, so a frontmatter
    // change to that field alone (no description or body change)
    // triggers reindex to re-embed. Proves the path:
    //   SKILL.md frontmatter when_to_use change
    //   → parse_skill_frontmatter picks it up
    //   → upsert_skill computes content_hash via embedding_text
    //   → reindex compares stored vs new hash, re-embeds on diff.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    // Add when_to_use to skill-a's frontmatter without changing
    // description or body. Pre-US4.b the content_hash was over only
    // (name, description, body); now it includes when_to_use too, so
    // the row's stored hash must mismatch and reindex must re-embed.
    let skill_a = catalog_root
        .join("plugin-alpha")
        .join("skills")
        .join("skill-a")
        .join("SKILL.md");
    std::fs::write(
        &skill_a,
        "---\n\
         name: skill-a\n\
         description: Well-formed skill that documents how to make alpha widgets shine.\n\
         when_to_use: When the user asks about alpha widget polish.\n\
         ---\n\
         \n\
         # skill-a\n\
         \n\
         Detailed body describing the alpha widget skill.\n",
    )
    .unwrap();

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let agg = run_with_deps(
        Scope::Plugin(id.clone()),
        &[id],
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex one plugin");

    assert_eq!(
        agg.skills_re_embedded, 1,
        "when_to_use change must trigger re-embed"
    );
    assert_eq!(
        embedder.call_count() - baseline,
        1,
        "exactly one embed call for the when_to_use-modified skill",
    );
}

#[test]
fn reindex_force_re_embeds_every_skill_in_scope() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let agg = run_with_deps(
        Scope::Plugin(id.clone()),
        &[id],
        &build_deps(&paths, &config, &embedder),
        true,
        Mode::Json,
    )
    .expect("force reindex");

    assert_eq!(agg.plugins_visited, 1);
    assert_eq!(agg.skills_re_embedded, 4);
    assert_eq!(agg.skills_unchanged, 0);
    assert_eq!(embedder.call_count() - baseline, 4);
}

#[test]
fn reindex_catalog_scope_visits_every_plugin_in_that_catalog() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);

    let plugins = vec![
        "sample-plugin-catalog/plugin-alpha".parse().unwrap(),
        "sample-plugin-catalog/plugin-beta".parse().unwrap(),
    ];
    let agg = run_with_deps(
        Scope::Catalog("sample-plugin-catalog".to_owned()),
        &plugins,
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex catalog");
    assert_eq!(agg.plugins_visited, 2);
    assert_eq!(agg.skills_checked, 5);
    assert_eq!(agg.skills_unchanged, 5);
}

// ---- CLI binary tests for the easy error paths --------------------------
//
// These don't need an embedder — they only hit the scope-parse path.

#[test]
fn reindex_unknown_catalog_exits_3() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["reindex", "ghost"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
}

// ---- FF2: scoped reindex resolves the catalog from the DB enrolment ------
//
// `parse_scope` previously checked `config.catalogs` for catalog existence;
// on a fresh install that map is empty, so `reindex <catalog>` and
// `reindex <catalog>/<plugin>` failed with exit 3 even for a DB-enrolled
// catalog. These drive the CLI binary against a catalog enrolled ONLY in
// the DB (no config.toml) and assert the exit-code contract is preserved:
// unknown catalog → 3, known catalog + unknown plugin → 20.

#[test]
fn reindex_catalog_scope_resolves_from_db_without_config() {
    // A DB-enrolled catalog with no enabled plugins reindexes to a clean
    // zero (exit 0, "Nothing to reindex") — the point is that the catalog
    // RESOLVES (no exit 3), not that there is work to do.
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    assert!(
        !paths.global_config_file.exists(),
        "this test must run with NO config.toml",
    );

    let out = env
        .cmd()
        .args(["reindex", "sample-plugin-catalog"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "reindex <db-enrolled-catalog> must resolve (exit 0), not exit 3; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Nothing to reindex"),
        "no enabled plugins → clean zero",
    );
}

#[test]
fn reindex_known_catalog_unknown_plugin_exits_20_without_config() {
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "sample-plugin-catalog/ghost-plugin"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(20),
        "known catalog + unknown plugin must be PluginNotFound (exit 20); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn reindex_unknown_catalog_scoped_plugin_exits_3_without_config() {
    // `<catalog>/<plugin>` where the catalog itself is unknown → exit 3,
    // checked before plugin existence.
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "ghost-catalog/plugin-alpha"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "unknown catalog in a scoped id must remain CatalogNotFound (exit 3); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn reindex_invalid_scope_format_exits_2() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["reindex", "bad/id/extra"])
        .output()
        .unwrap();
    // Two slashes — invalid plugin id format → Usage (exit 2).
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn reindex_empty_scope_with_no_enabled_plugins_is_a_clean_zero() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["reindex"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Nothing to reindex"),
        "expected 'Nothing to reindex' message, got: {stdout}",
    );
}

// ---- issue #316: --catalog / --plugin flags + globs / multi-scope --------
//
// The parse/resolution error paths don't need an embedder — the same boundary
// as the FF2 exit-code tests above. Cases that actually re-embed enabled
// plugins (glob expansion, multi-scope union) are covered at the library level
// in the `resolve_explicit` unit tests (no `FastembedEmbedder` on-disk model)
// and the whole-index invariant is proven in `tests/model_tiering.rs`.

#[test]
fn reindex_positional_conflicts_with_catalog_flag_exits_2() {
    // clap `conflicts_with = "scopes"` rejects mixing a positional token and
    // `--catalog` → usage error (exit 2) before any resolution runs.
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["reindex", "some-catalog", "--catalog", "other"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn reindex_positional_conflicts_with_plugin_flag_exits_2() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["reindex", "some-catalog", "--plugin", "cat/plug"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn reindex_catalog_glob_zero_match_exits_2() {
    // A `*` glob that matches no enrolled catalog is a usage error (exit 2),
    // NOT a silent no-op.
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "--catalog", "ghost-*"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "a catalog glob matching nothing must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ghost-*"),
        "the error must echo the pattern",
    );
}

#[test]
fn reindex_plugin_glob_zero_match_exits_2() {
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    // No plugins are ENABLED in the staged catalog, so ANY plugin glob matches
    // zero → exit 2 (echoing the pattern).
    let out = env
        .cmd()
        .args(["reindex", "--plugin", "sample-plugin-catalog/*"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "a plugin glob matching nothing must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn reindex_catalog_flag_empty_catalog_is_a_clean_zero() {
    // `--catalog <enrolled-but-empty>`: the catalog RESOLVES (no exit 3) and has
    // no enabled plugins → the benign "Nothing to reindex" (exit 0). This is an
    // exact selection, so the empty target set is NOT collapsed to a glob error.
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "--catalog", "sample-plugin-catalog"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "an enrolled-but-empty catalog reindexes to a clean zero; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Nothing to reindex"),
        "no enabled plugins → clean zero",
    );
}

#[test]
fn reindex_catalog_flag_unknown_catalog_exits_3() {
    // `--catalog <unknown>` preserves the CatalogNotFound (exit 3) contract.
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "--catalog", "ghost"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn reindex_plugin_flag_known_catalog_unknown_plugin_exits_20() {
    // `--plugin <known>/<unknown>` preserves PluginNotFound (exit 20).
    let env = ToolEnv::new();
    let paths = crate::common::paths_for(&env);
    crate::common::stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["reindex", "--plugin", "sample-plugin-catalog/ghost-plugin"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(20),
        "known catalog + unknown plugin via --plugin must be PluginNotFound (20); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}
