//! Phase 7 / FR-012 — end-to-end exit-code coverage driven through a REAL
//! in-process MCP server (closes CONCERNS GAP-1; SC-010).
//!
//! `tests/exit_codes.rs` is unit-level (constructs each `TomeError`,
//! asserts `exit_code()`); `tests/exit_codes_e2e.rs` drives the CLI
//! binary. Neither reaches the MCP-internal exit codes the `tome mcp`
//! server surfaces but the CLI binary cannot — those previously sat
//! either library-stub-covered (the F3 substitution stub never failed,
//! so codes 9/28 were only hand-pinned in
//! `tests/mcp_prompts_get_error_json_shape.rs`) or untested end-to-end.
//!
//! This file drives the in-process [`common::mcp_harness::McpHarness`]
//! (a real `mcp::server::Server` over a StubEmbedder-backed workspace) to
//! genuinely reach each code through the live `prompts/get` / `get_skill`
//! handlers, then asserts the surfaced `McpError`'s slug maps —
//! through the production `TomeError::exit_code()` — to the documented
//! number:
//!
//! | Code | Variant                   | Reached via                                            |
//! |------|---------------------------|--------------------------------------------------------|
//! | 9    | PluginDataDirWriteFailed  | `get_skill` body `${TOME_PLUGIN_DATA}` + unwritable root |
//! | 26   | PromptArgumentMismatch    | `prompts/get` with an unknown named-arg key            |
//! | 27   | EntryNotFound             | `get_skill` on an enabled row whose plugin dir is gone |
//! | 28   | SubstitutionFailed        | `prompts/get` with the context-builder seam tripped    |
//! | 29   | InvalidArgumentFrontmatter| `tome plugin enable` rejecting illegal `arguments` names |
//!
//! Plus the FR-004 (F-MCP-PROMPT-COLLISION / K4) end-to-end gate: a
//! `Command foo` + user-invocable `Skill foo` + `Command foo2` fixture
//! driven through `prompts/list` + `prompts/get`, proving all three are
//! advertised under distinct names AND each resolves (the single-global-
//! taken-set fix, verified through the real server rather than the
//! resolver unit in `tests/prompt_collision_global.rs`).
//!
//! Library-API tests using `StubEmbedder` + `StubReranker`; no ONNX model
//! load; the CLI binary is not invoked. The harness drives the async MCP
//! handlers on a tokio runtime owned per `McpHarness` — this does NOT
//! violate `tests/sync_boundary.rs` (which scans `src/`, never `tests/`).

mod common;

use serde_json::{Map, json};
use tempfile::TempDir;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

use common::mcp_harness::{
    McpHarness, StagedWorkspace, mcp_error_exit_code, mcp_error_slug, seed_catalog_enrolment,
    write_plugin,
};
use common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};
use tome::embedding::stub::StubEmbedder;
use tome::mcp::tools::{get_skill, search_skills};

// ===========================================================================
// Code 9 — PluginDataDirWriteFailed.
//
// A skill body referencing `${TOME_PLUGIN_DATA}` drives the Stage-1
// built-in `ensure_plugin_data`, which `create_dir_all`s
// `<root>/plugin-data/<catalog>/<plugin>/`. We pre-create `<root>/
// plugin-data` as a REGULAR FILE so the `create_dir_all` fails with
// `NotADirectory` → `SubstitutionError::PluginDataDirCreationFailed` →
// `TomeError::PluginDataDirWriteFailed` (exit 9). Real fixture, no seam.
// ===========================================================================

#[test]
fn get_skill_plugin_data_dir_unwritable_exits_9() {
    // Skill (default user_invocable=false, fine — get_skill reads skills
    // directly) whose body forces the plugin-data dir creation.
    let skill_body =
        "---\nname: needs-data\ndescription: writes scratch.\n---\ndata at ${TOME_PLUGIN_DATA}\n";
    let staged = StagedWorkspace::stage(&[("needs-data", skill_body)], &[]);

    // Wedge the plugin-data root: place a regular file where the tree
    // root must be a directory. `create_dir_all` of any child then fails.
    let plugin_data_root = staged.paths.plugin_data_root();
    assert!(
        !plugin_data_root.exists(),
        "fixture precondition: plugin-data root must not yet exist",
    );
    std::fs::write(&plugin_data_root, b"not a directory").expect("write blocking file");

    let harness = staged.harness();
    let err = harness
        .call_get_skill(get_skill::Input {
            catalog: staged.catalog_name.clone(),
            plugin: staged.plugin_name.clone(),
            name: "needs-data".into(),
        })
        .expect_err("plugin-data dir creation must fail when the root is a file");

    assert_eq!(
        mcp_error_slug(&err),
        "plugin_data_dir_write_failed",
        "slug surfaced by the live get_skill render path; got {err:?}",
    );
    assert_eq!(
        mcp_error_exit_code(&err),
        9,
        "PluginDataDirWriteFailed must map to exit 9",
    );
}

// ===========================================================================
// Code 26 — PromptArgumentMismatch.
//
// `prompts/get` for a command declaring `[who]` but called with an
// unknown key `bogus` → `map_caller_arguments` rejects → exit 26.
// ===========================================================================

#[test]
fn prompts_get_unknown_named_arg_exits_26() {
    let cmd_body =
        "---\nname: greet\ndescription: greet someone.\narguments: [who]\n---\nHello $who!\n";
    let staged = StagedWorkspace::stage(&[], &[("greet", cmd_body)]);
    let harness = staged.harness();

    // Sanity: the prompt IS advertised (so the mismatch is the failure
    // under test, not a missing prompt).
    assert!(
        harness.prompt_names().contains(&"plug__greet".to_owned()),
        "fixture precondition: the command must be advertised; got {:?}",
        harness.prompt_names(),
    );

    let mut args = Map::new();
    args.insert("bogus".into(), json!("value"));

    let err = harness
        .prompts_get("plug__greet", Some(args))
        .expect_err("unknown named arg key must reject");

    assert_eq!(
        mcp_error_slug(&err),
        "prompt_argument_mismatch",
        "slug surfaced by the live prompts/get path; got {err:?}",
    );
    assert_eq!(
        mcp_error_exit_code(&err),
        26,
        "PromptArgumentMismatch must map to exit 26",
    );
}

// ===========================================================================
// Code 27 — EntryNotFound.
//
// `get_skill` on an enabled skill row whose on-disk plugin directory was
// removed AFTER enable (catalog cache evicted / manifest drift). The
// index row survives; `resolve_entry_body_path` finds the plugin dir
// absent → `TomeError::EntryNotFound` (slug `entry_not_found`, exit 27).
// ===========================================================================

#[test]
fn get_skill_with_missing_plugin_dir_exits_27() {
    let skill_body = "---\nname: ghost\ndescription: about to vanish.\n---\nbody\n";
    let staged = StagedWorkspace::stage(&[("ghost", skill_body)], &[]);

    // Remove the on-disk plugin dir the catalog manifest resolves to. The
    // DB row (enabled) remains, so lookup succeeds but body-path
    // resolution discovers the plugin dir is gone.
    let plugin_dir = staged.plugin_dir();
    assert!(plugin_dir.is_dir(), "fixture wrote the plugin dir");
    std::fs::remove_dir_all(&plugin_dir).expect("remove plugin dir");

    let harness = staged.harness();
    let err = harness
        .call_get_skill(get_skill::Input {
            catalog: staged.catalog_name.clone(),
            plugin: staged.plugin_name.clone(),
            name: "ghost".into(),
        })
        .expect_err("missing plugin dir must surface EntryNotFound");

    assert_eq!(
        mcp_error_slug(&err),
        "entry_not_found",
        "slug surfaced by the live get_skill lookup path; got {err:?}",
    );
    assert_eq!(
        mcp_error_exit_code(&err),
        27,
        "EntryNotFound must map to exit 27",
    );
}

// ===========================================================================
// Code 28 — SubstitutionFailed.
//
// `prompts/get`'s `build_get_context` wraps ANY `SubstitutionContext`
// builder failure as `TomeError::SubstitutionFailed` (exit 28). The
// builder only fails on a missing required field, which the production
// `build_context_for_entry` never triggers — so the wrap is unreachable
// through fixtures alone. The documented `#[doc(hidden)]` seam
// (`FORCE_CONTEXT_BUILD_FAILURE`, no-op in prod) trips a missing field so
// the GENUINE production wrap path executes end-to-end through the live
// server. (cf. the hand-pinned `substitution_failed` envelope in
// `tests/mcp_prompts_get_error_json_shape.rs`, which FR-012 supersedes
// with a real end-to-end trigger.)
// ===========================================================================

#[test]
fn prompts_get_context_build_failure_exits_28() {
    // A no-args command renders cleanly without the seam — proving the
    // seam (not the fixture) is what forces the failure.
    let cmd_body = "---\nname: plain\ndescription: a plain command.\n---\nDo a thing.\n";
    let staged = StagedWorkspace::stage(&[], &[("plain", cmd_body)]);
    let harness = staged.harness();

    // Baseline: resolves fine with the seam OFF.
    let ok = harness
        .prompts_get_text("plug__plain", None)
        .expect("baseline prompts/get must succeed without the seam");
    assert_eq!(ok.trim(), "Do a thing.");

    // Seam ON: the render path's context builder omits a required field,
    // so `.build()` fails and `build_get_context` wraps it as exit 28.
    // The harness sets + clears the process-global flag WITHIN the render
    // serialisation lock, so this never races a concurrent sibling render.
    let err = harness
        .prompts_get_forcing_context_failure("plug__plain", None)
        .expect_err("context-build failure must surface SubstitutionFailed");

    assert_eq!(
        mcp_error_slug(&err),
        "substitution_failed",
        "slug surfaced by the live prompts/get render path; got {err:?}",
    );
    assert_eq!(
        mcp_error_exit_code(&err),
        28,
        "SubstitutionFailed must map to exit 28",
    );
}

// ===========================================================================
// Code 29 — InvalidArgumentFrontmatter.
//
// Illegal `arguments` names (e.g. `Bad-Name`, which violates
// `^[a-z_][a-z0-9_]*$`) are a parse-class failure rejected at
// `tome plugin enable` time (`lifecycle::enable` → exit 29) — the gate
// that BARS such an entry from ever reaching the index, and therefore
// from ever reaching the MCP registry / `prompts/get` render path. The
// genuine end-to-end path to code 29 is thus the indexing gate the MCP
// registry sits on top of; we drive `lifecycle::enable` directly (the
// real production entry point the in-process server's registry build
// depends on).
// ===========================================================================

#[test]
fn plugin_enable_with_illegal_argument_name_exits_29() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    std::fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);

    // A command whose `arguments` carries an illegal name. The enable
    // pipeline parses frontmatter then runs `validate_argument_names`.
    let bad_cmd =
        "---\nname: bad\ndescription: illegal arg name.\narguments: [Bad-Name]\n---\nbody $1\n";
    write_plugin(&catalog_root, "plug", &[], &[("bad", bad_cmd)]);

    // FF1: `lifecycle::enable` resolves the plugin dir from the DB enrolment,
    // so the catalog must be enrolled (and its cache dir symlinked onto the
    // fixture) BEFORE the enable — otherwise the DB lookup fails with
    // `CatalogNotFound` before the frontmatter gate this test targets.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");

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
    let err = lifecycle::enable(&id, &deps)
        .expect_err("illegal argument name must reject at enable time");

    assert_eq!(
        err.category(),
        "invalid_argument_frontmatter",
        "illegal arg name → InvalidArgumentFrontmatter; got {err:?}",
    );
    assert_eq!(
        err.exit_code(),
        29,
        "InvalidArgumentFrontmatter must map to exit 29",
    );

    // The barred entry never reached the index, so the MCP registry built
    // from this workspace is empty — i.e. `prompts/get` can never see a
    // malformed-frontmatter entry. Prove that invariant end-to-end. The
    // catalog was already enrolled above (before the enable).
    let harness = McpHarness::new(&paths);
    // Phase 9 / US3: the always-on reserved `add-tome-conversion-skill` built-in
    // is the only prompt on the surface; the illegal-arg PLUGIN entry must not
    // reach it.
    let plugin_prompts: Vec<String> = harness
        .prompt_names()
        .into_iter()
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();
    assert!(
        plugin_prompts.is_empty(),
        "the illegal-arg entry must not reach the MCP prompt surface; got {:?}",
        harness.prompt_names(),
    );
}

// ===========================================================================
// FR-004 (F-MCP-PROMPT-COLLISION / K4) — single-global-taken-set, e2e.
//
// `Command foo` + user-invocable `Skill foo` + `Command foo2` all derive
// the base prompt name `plug__foo` (the two `foo`s collide) and
// `plug__foo2` (the standalone). The pre-fix per-bucket suffixing minted
// a SECOND `plug__foo2` for the colliding-`foo` loser, and the terminal
// `by_name.insert` silently dropped one entry. The fix assigns names
// against ONE global taken-set, so all three survive under distinct
// names. We verify END-TO-END through the real server: all three appear
// in `prompts/list` (distinct names) AND each resolves via `prompts/get`
// (no `prompt_not_found`). Distinct from the resolver unit in
// `tests/prompt_collision_global.rs`.
// ===========================================================================

#[test]
fn prompt_collision_all_three_entries_resolvable_e2e() {
    // Skill `foo` opts into user-invocability so it joins the prompt
    // surface alongside the two commands.
    let skill_foo = "---\nname: foo\ndescription: a skill named foo.\nuser-invocable: true\n---\nskill foo body\n";
    let cmd_foo = "---\nname: foo\ndescription: a command named foo.\n---\ncommand foo body\n";
    let cmd_foo2 = "---\nname: foo2\ndescription: a command named foo2.\n---\ncommand foo2 body\n";

    let staged = StagedWorkspace::stage(
        &[("foo", skill_foo)],
        &[("foo", cmd_foo), ("foo2", cmd_foo2)],
    );
    let harness = staged.harness();

    // (1) prompts/list — all three present, under three DISTINCT names.
    // Phase 9 / US3: drop the always-on reserved `add-tome-conversion-skill`
    // built-in so this asserts only the three PLUGIN-derived prompts.
    let mut names = harness.prompt_names();
    names.retain(|n| n != "add-tome-conversion-skill");
    names.sort();
    assert_eq!(
        names.len(),
        3,
        "all three user-invocable entries must be advertised (no silent drop); got {names:?}",
    );
    let unique: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(
        unique.len(),
        3,
        "advertised prompt names must be pairwise-distinct; got {names:?}",
    );
    // The two `foo`-derived entries occupy `plug__foo` + a counter suffix;
    // the standalone keeps `plug__foo2`. The exact winner is tie-broken by
    // indexed_at then the identity tuple; we assert the SET shape rather
    // than pin the winner, since both `foo`s share an indexed_at in this
    // fixture and the tuple tie-break is an internal detail.
    assert!(
        names.contains(&"plug__foo".to_owned()),
        "the collision winner must hold the base name `plug__foo`; got {names:?}",
    );
    assert!(
        names.contains(&"plug__foo2".to_owned()),
        "the standalone `foo2` must keep `plug__foo2`; got {names:?}",
    );

    // (2) prompts/get — EVERY advertised name resolves (no prompt_not_found).
    for name in &names {
        let resp = harness
            .prompts_get(name, None)
            .unwrap_or_else(|e| panic!("advertised prompt `{name}` must resolve; got error {e:?}"));
        assert_eq!(
            resp.messages.len(),
            1,
            "prompt `{name}` must render a single user message",
        );
    }

    // (3) The standalone `foo2` command must resolve to ITS OWN body — i.e.
    // the global-taken-set fix didn't alias it onto a collision loser.
    let foo2_text = harness
        .prompts_get_text("plug__foo2", None)
        .expect("plug__foo2 resolves");
    assert!(
        foo2_text.contains("command foo2 body"),
        "plug__foo2 must render the standalone foo2 body, not a collision loser; got {foo2_text:?}",
    );
}

// ---------------------------------------------------------------------------
// Smoke: the harness can also drive the search_skills tool end-to-end.
// Keeps the harness's tool surface exercised (and guards the stub-entry
// drift seam) so a regression there fails here rather than silently.
// ---------------------------------------------------------------------------

#[test]
fn harness_drives_search_skills_end_to_end() {
    let skill_body = "---\nname: searchable\ndescription: a findable skill.\n---\nbody\n";
    let staged = StagedWorkspace::stage(&[("searchable", skill_body)], &[]);
    let harness = staged.harness();

    let out = harness
        .call_search_skills(search_skills::Input {
            query: "findable".into(),
            top_k: 10,
            catalog: None,
            plugin: None,
            description_max_chars: 150,
        })
        .expect("search_skills must succeed against the stub-seeded index");

    assert!(
        out.matches.iter().any(|m| m.name == "searchable"),
        "the indexed skill must be discoverable via the live search_skills tool; got {:?}",
        out.matches.iter().map(|m| &m.name).collect::<Vec<_>>(),
    );
}
