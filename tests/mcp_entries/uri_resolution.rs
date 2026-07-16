//! End-to-end `get_skill` URI resolution (Task 7): the `uri` input resolves
//! to a body (unique match), to `matches`/`next_actions` (multi-match), or
//! to an `unknown_skill` `invalid_params` envelope (no match) — driven
//! through the REAL `get_skill::handle` via the in-process `McpHarness`.

use std::path::PathBuf;

use tome::embedding::stub::StubEmbedder;
use tome::mcp::tools::get_skill;
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

use crate::common::mcp_harness::{
    McpHarness, StagedWorkspace, mcp_error_slug, open_index, seed_catalog_enrolment, write_plugin,
};
use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

const SKILL: &str = "---\nname: alpha\ndescription: A skill\n---\nBODY";

#[test]
fn uri_triple_resolves_uniquely_like_the_structured_form() {
    // `stage` seeds one skill; look up its name via search to learn it, then
    // fetch by uri and by triple and compare the resolved identity.
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let h = ws.harness();

    let by_uri = h
        .call_get_skill(get_skill::Input::for_uri(format!(
            "{}:{}:alpha",
            ws.catalog_name, ws.plugin_name
        )))
        .expect("uri triple resolves");
    assert_eq!(by_uri.name.as_deref(), Some("alpha"));
    assert_eq!(by_uri.catalog.as_deref(), Some(ws.catalog_name.as_str()));
    assert!(by_uri.content.is_some(), "unique uri returns a body");

    let by_triple = h
        .call_get_skill(get_skill::Input::triple(
            &ws.catalog_name,
            &ws.plugin_name,
            "alpha",
        ))
        .expect("triple resolves");
    assert_eq!(
        by_uri.content, by_triple.content,
        "uri and triple bodies identical"
    );
    assert_eq!(by_uri.plugin.as_deref(), by_triple.plugin.as_deref());
    assert_eq!(by_uri.kind, by_triple.kind);
}

#[test]
fn uri_bare_name_resolves_when_unique() {
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let h = ws.harness();
    let out = h
        .call_get_skill(get_skill::Input::for_uri("alpha"))
        .expect("bare name");
    assert_eq!(out.name.as_deref(), Some("alpha"));
    assert!(out.content.is_some());
}

#[test]
fn uri_plugin_name_resolves_when_unique() {
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let h = ws.harness();
    let out = h
        .call_get_skill(get_skill::Input::for_uri(format!(
            "{}:alpha",
            ws.plugin_name
        )))
        .expect("plugin:skill resolves");
    assert_eq!(out.name.as_deref(), Some("alpha"));
    assert_eq!(out.catalog.as_deref(), Some(ws.catalog_name.as_str()));
    assert!(out.content.is_some());
}

/// Stage a workspace whose catalog clone lives DIRECTLY at
/// `paths.cache_dir_for(url)` — no `#[cfg(unix)]` symlink indirection. This
/// deliberately does NOT reuse `StagedWorkspace::stage` /
/// `seed_catalog_enrolment`: those symlink the content-addressed cache dir
/// onto a separate on-disk `catalog_root`, and `uri_resolver::resolve_path`'s
/// `is_symlinked` guard (FR-S-02, "reject a hostile catalog's symlinked
/// SKILL.md") walks every ANCESTOR of a candidate body path — so it also
/// trips on that symlinked cache dir itself, filtering every record out of
/// path-candidate resolution before a match is even attempted. Building the
/// plugin tree straight into the (non-symlinked) cache dir sidesteps that,
/// so this test exercises the intended path-equivalence logic in
/// `body_matches_target` rather than the symlink guard.
fn stage_workspace_with_real_cache_dir(
    skill_name: &str,
    skill_body: &str,
) -> (tempfile::TempDir, tome::paths::Paths, PathBuf, &'static str) {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    const CATALOG_NAME: &str = "direct";
    const PLUGIN_NAME: &str = "plug";
    let url = format!("file://{}/direct-catalog-marker", tmp.path().display());
    let cache_dir = paths.cache_dir_for(&url);
    std::fs::create_dir_all(&cache_dir).unwrap();
    write_plugin(&cache_dir, PLUGIN_NAME, &[(skill_name, skill_body)], &[]);

    let conn = open_index(&paths);
    tome::index::workspace_catalogs::insert(&conn, "global", CATALOG_NAME, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let config = config_with_catalog(CATALOG_NAME, &cache_dir);
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
    let id: PluginId = format!("{CATALOG_NAME}/{PLUGIN_NAME}").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin");

    (tmp, paths, cache_dir, PLUGIN_NAME)
}

#[test]
fn uri_absolute_path_resolves() {
    let (_tmp, paths, cache_dir, plugin_name) = stage_workspace_with_real_cache_dir("alpha", SKILL);
    let h = McpHarness::new(&paths);
    let body = cache_dir
        .join(plugin_name)
        .join("skills")
        .join("alpha")
        .join("SKILL.md");
    let out = h
        .call_get_skill(get_skill::Input::for_uri(body.display().to_string()))
        .expect("path resolves");
    assert_eq!(out.name.as_deref(), Some("alpha"));
}

#[test]
fn uri_unknown_is_invalid_params() {
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let h = ws.harness();
    let err = h
        .call_get_skill(get_skill::Input::for_uri("no:such:thing"))
        .unwrap_err();
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(mcp_error_slug(&err), "unknown_skill");
}

#[test]
fn both_uri_and_triple_is_invalid_params() {
    let ws = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let h = ws.harness();
    let mut input = get_skill::Input::triple(&ws.catalog_name, &ws.plugin_name, "alpha");
    input.uri = Some("alpha".into());
    let err = h.call_get_skill(input).unwrap_err();
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

/// Headline multi-match coverage: two catalogs (`acme` from `StagedWorkspace`
/// + a second `beta` catalog staged manually here, mirroring
/// `StagedWorkspace::stage`'s own setup) each ship a plugin with an entry
/// named `collide`. A bare-name `uri` then collides across catalogs, and
/// `get_skill` must return `matches` + index-aligned `next_actions` instead
/// of a body.
#[test]
fn uri_bare_name_multi_match_across_catalogs_returns_matches_and_next_actions() {
    const COLLIDE: &str = "---\nname: collide\ndescription: The collide skill.\n---\nCollide body.";

    let ws = StagedWorkspace::stage(&[("collide", COLLIDE)], &[]);

    // Second catalog `beta` / plugin `plug2`, staged the same way
    // `StagedWorkspace::stage` builds `acme`/`plug`, sharing the SAME
    // central index (`ws.paths`) so both catalogs land in one workspace.
    let catalog_root2 = ws.tmp.path().join("catalog2");
    std::fs::create_dir_all(&catalog_root2).unwrap();
    let config2 = config_with_catalog("beta", &catalog_root2);
    write_plugin(&catalog_root2, "plug2", &[("collide", COLLIDE)], &[]);

    let embedder = tome::embedding::stub::StubEmbedder::new();
    let scope = Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths: &ws.paths,
        scope: &scope,
        config: &config2,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "beta/plug2".parse().unwrap();
    seed_catalog_enrolment(&ws.paths, &catalog_root2, "beta");
    lifecycle::enable(&id, &deps).expect("enable second plugin");

    let h = ws.harness();
    let out = h
        .call_get_skill(get_skill::Input::for_uri("collide"))
        .expect("multi-match returns Ok, not Err");

    assert!(out.content.is_none(), "multi-match must not return a body");
    assert!(
        out.catalog.is_none(),
        "multi-match must not return identity"
    );

    let matches = out.matches.expect("matches present");
    let next_actions = out.next_actions.expect("next_actions present");
    assert_eq!(matches.len(), 2, "collides across both catalogs");
    assert_eq!(next_actions.len(), 2);

    // Deterministic `(catalog, plugin, kind, name)` order: acme before beta.
    assert_eq!(matches[0].catalog, "acme");
    assert_eq!(matches[0].plugin, "plug");
    assert_eq!(matches[0].name, "collide");
    assert_eq!(matches[0].kind, EntryKind::Skill);
    assert_eq!(matches[1].catalog, "beta");
    assert_eq!(matches[1].plugin, "plug2");
    assert_eq!(matches[1].name, "collide");
    assert_eq!(matches[1].kind, EntryKind::Skill);

    // `next_actions` is index-aligned with `matches`.
    for (m, a) in matches.iter().zip(next_actions.iter()) {
        assert_eq!(a.tool, "get_skill");
        assert_eq!(a.arguments.catalog, m.catalog);
        assert_eq!(a.arguments.plugin, m.plugin);
        assert_eq!(a.arguments.name, m.name);
        assert_eq!(a.arguments.kind, m.kind);
    }

    // Each disambiguating next_action resolves uniquely on its own.
    let disambiguated = h
        .call_get_skill(get_skill::Input::triple(
            &next_actions[0].arguments.catalog,
            &next_actions[0].arguments.plugin,
            &next_actions[0].arguments.name,
        ))
        .expect("next_action triple resolves");
    assert_eq!(disambiguated.catalog.as_deref(), Some("acme"));
}
