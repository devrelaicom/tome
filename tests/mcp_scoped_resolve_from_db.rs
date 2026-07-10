//! FF3 regression: the MCP tools (`get_skill`, `search_skills`) must resolve
//! catalog existence from the `workspace_catalogs` DB, NOT `config.toml
//! [catalogs]`.
//!
//! Companion to `plugin_list_resolve_from_db.rs` (FF2, the CLI discovery
//! surfaces) and `plugin_resolve_from_db.rs` (FF1, the shared resolver). Both
//! MCP tools previously gated on `config.catalogs.contains_key(...)` before
//! their DB lookup; on a fresh install that map is empty (`tome catalog add`
//! enrols only into the DB), so every `get_skill` / catalog-filtered
//! `search_skills` against an enrolled catalog returned `unknown_catalog`.
//!
//! [`StagedWorkspace`] enrols the catalog ONLY in the DB (no `config.toml`
//! is written — see its header), so these assertions are honest about the
//! no-config state a real user lands in. The `unknown_catalog` envelope is
//! preserved for genuinely-absent catalogs.

mod common;

use common::mcp_harness::{StagedWorkspace, mcp_error_slug};
use tome::mcp::tools::{get_skill, search_skills};

const SKILL: &str = "---\nname: searchable\ndescription: a findable skill.\n---\nthe body\n";

/// `get_skill` must resolve a DB-enrolled catalog with no `config.toml`
/// present and return the skill body. Before FF3 the handler `store::load`ed
/// the (absent) config and rejected with `unknown_catalog`.
#[test]
fn get_skill_resolves_db_enrolled_catalog_without_config() {
    let staged = StagedWorkspace::stage(&[("searchable", SKILL)], &[]);

    // Guard the production invariant: enrolment is DB-only.
    assert!(
        !staged.paths.global_config_file.exists(),
        "this test must run with NO config.toml",
    );

    let out = staged
        .harness()
        .call_get_skill(get_skill::Input {
            catalog: staged.catalog_name.clone(),
            plugin: staged.plugin_name.clone(),
            name: "searchable".into(),
            raw: false,
            include_resource_bodies: false,
        })
        .expect("get_skill must resolve the DB-enrolled catalog (no config.toml)");

    assert!(
        out.content.contains("the body"),
        "get_skill must return the skill body; got {:?}",
        out.content,
    );
}

/// A catalog absent from the DB must still surface `unknown_catalog` — the
/// migration narrows the lookup to the DB; it does not weaken the contract
/// for genuinely-unknown catalogs.
#[test]
fn get_skill_unknown_catalog_still_errors_without_config() {
    let staged = StagedWorkspace::stage(&[("searchable", SKILL)], &[]);

    let err = staged
        .harness()
        .call_get_skill(get_skill::Input {
            catalog: "ghost-catalog".into(),
            plugin: staged.plugin_name.clone(),
            name: "searchable".into(),
            raw: false,
            include_resource_bodies: false,
        })
        .expect_err("an absent catalog must still reject");

    assert_eq!(
        mcp_error_slug(&err),
        "unknown_catalog",
        "absent catalog must remain unknown_catalog; got {err:?}",
    );
}

/// `search_skills --catalog <name>` must resolve the named catalog from the
/// DB enrolment (no `config.toml`) and return matches.
#[test]
fn search_skills_catalog_filter_resolves_from_db_without_config() {
    let staged = StagedWorkspace::stage(&[("searchable", SKILL)], &[]);

    assert!(
        !staged.paths.global_config_file.exists(),
        "this test must run with NO config.toml",
    );

    let out = staged
        .harness()
        .call_search_skills(search_skills::Input {
            query: "findable".into(),
            top_k: Some(10),
            catalog: Some(staged.catalog_name.clone()),
            plugin: None,
            kind: None,
            rerank: None,
            min_score: None,
            description_max_chars: Some(150),
        })
        .expect("search_skills --catalog must resolve from the DB (no config.toml)");

    assert!(
        out.matches.iter().any(|m| m.name == "searchable"),
        "the catalog-filtered search must surface the indexed skill; got {:?}",
        out.matches.iter().map(|m| &m.name).collect::<Vec<_>>(),
    );
}

/// `search_skills --catalog <name>` for a catalog absent from the DB must
/// still reject with `unknown_catalog`.
#[test]
fn search_skills_unknown_catalog_filter_still_errors_without_config() {
    let staged = StagedWorkspace::stage(&[("searchable", SKILL)], &[]);

    let err = staged
        .harness()
        .call_search_skills(search_skills::Input {
            query: "findable".into(),
            top_k: Some(10),
            catalog: Some("ghost-catalog".into()),
            plugin: None,
            kind: None,
            rerank: None,
            min_score: None,
            description_max_chars: Some(150),
        })
        .expect_err("an absent catalog filter must still reject");

    assert_eq!(
        mcp_error_slug(&err),
        "unknown_catalog",
        "absent catalog filter must remain unknown_catalog; got {err:?}",
    );
}
