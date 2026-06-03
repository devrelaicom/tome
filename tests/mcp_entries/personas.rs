//! Phase 6 / US4 — agent personas via MCP prompts (T109 / T111).
//!
//! Drives `PromptRegistry::build_for_workspace(.., expose_personas)`
//! against on-disk agent fixtures indexed through `lifecycle::enable`
//! (StubEmbedder, no ONNX models). The CLI binary is not invoked here —
//! the library API is the surface under test, mirroring
//! `tests/mcp_prompts.rs`.
//!
//! Covers `contracts/agent-personas.md` § Tests (the persona toggle, the
//! `<name>-persona` + `drop-persona` shape, the template-wrapped +
//! substituted body, the frontmatter-vs-stem name resolution, the fixed
//! `drop-persona` body) plus the T111 byte-stable JSON wire-pin of the
//! persona `prompts/list` descriptors + the persona `prompts/get`
//! envelope.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::lookup;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::state::McpState;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, WorkspaceName};

use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Fixture helpers.
// ---------------------------------------------------------------------------

fn global() -> WorkspaceName {
    WorkspaceName::global()
}

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
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

/// One plugin's contribution to a fixture catalog: a set of
/// `agents/<name>.md` files and a set of `commands/<name>.md` files. The
/// agent/command tuples are `(file_stem, verbatim_markdown)`.
struct PluginSpec<'a> {
    name: &'a str,
    agents: &'a [(&'a str, &'a str)],
    commands: &'a [(&'a str, &'a str)],
}

/// Lay out a multi-plugin catalog under `catalog_root`, writing a single
/// `tome-catalog.toml` declaring every plugin plus each plugin's
/// `.claude-plugin/plugin.json`, agent files, and command files.
fn write_catalog(catalog_root: &Path, catalog_name: &str, plugins: &[PluginSpec<'_>]) {
    fs::create_dir_all(catalog_root).unwrap();
    let mut manifest = format!("name = \"{catalog_name}\"\nversion = \"0.1.0\"\n");
    for plugin in plugins {
        manifest.push_str(&format!(
            "\n[[plugins]]\nname = \"{0}\"\nsource = \"./{0}\"\n",
            plugin.name
        ));
    }
    fs::write(catalog_root.join("tome-catalog.toml"), manifest).unwrap();

    for plugin in plugins {
        let plugin_dir = catalog_root.join(plugin.name);
        fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            plugin_dir.join("tome-plugin.toml"),
            format!(
                "name = \"{}\"\nversion = \"1.0.0\"\n",
                plugin_dir.file_name().unwrap().to_string_lossy()
            ),
        )
        .unwrap();
        fs::write(
            plugin_dir.join(".claude-plugin").join("plugin.json"),
            format!(r#"{{"name": "{}", "version": "1.0.0"}}"#, plugin.name),
        )
        .unwrap();
        if !plugin.agents.is_empty() {
            let agents_dir = plugin_dir.join("agents");
            fs::create_dir_all(&agents_dir).unwrap();
            for (stem, body) in plugin.agents {
                fs::write(agents_dir.join(format!("{stem}.md")), body).unwrap();
            }
        }
        if !plugin.commands.is_empty() {
            let cmd_dir = plugin_dir.join("commands");
            fs::create_dir_all(&cmd_dir).unwrap();
            for (stem, body) in plugin.commands {
                fs::write(cmd_dir.join(format!("{stem}.md")), body).unwrap();
            }
        }
    }
}

/// Seed the central DB's `workspace_catalogs` enrolment for `global` and
/// symlink the URL-hashed `cache_dir` onto the on-disk fixture so the
/// registry's `resolve_entry_body_path` walk hits the real files. Mirrors
/// `tests/mcp_prompts.rs::seed_catalog_enrolment`.
fn seed_catalog_enrolment(paths: &tome::paths::Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                fs::create_dir_all(dst)?;
                for entry in fs::read_dir(src)? {
                    let entry = entry?;
                    let to = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &to)?;
                    } else {
                        fs::copy(entry.path(), &to)?;
                    }
                }
                Ok(())
            }
            copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
        }
    }
}

/// Stage a `global` workspace with one fixture catalog enabled. Returns
/// the temp dir (must outlive the test) plus `Paths` rooted in it. Every
/// plugin in `plugins` is enabled via `lifecycle::enable`, then the
/// catalog enrolment is seeded for the registry's path resolution.
fn stage(catalog_name: &str, plugins: &[PluginSpec<'_>]) -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    write_catalog(&catalog_root, catalog_name, plugins);

    let config = config_with_catalog(catalog_name, &catalog_root);
    let embedder = StubEmbedder::new();
    let scope = Scope(global());

    // FF1: enrolment + cache symlink must precede enable — resolve_plugin_dir
    // reads workspace_catalogs now, not the in-memory Config.
    seed_catalog_enrolment(&paths, &catalog_root, catalog_name);

    for plugin in plugins {
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
        let id: PluginId = format!("{catalog_name}/{}", plugin.name).parse().unwrap();
        lifecycle::enable(&id, &deps).unwrap_or_else(|e| panic!("enable {}: {e:?}", plugin.name));
    }

    (tmp, paths)
}

/// Build the registry for `global` at the given persona toggle.
fn build(paths: &tome::paths::Paths, expose_personas: bool) -> PromptRegistry {
    let conn = open_index(paths);
    PromptRegistry::build_for_workspace(&global(), paths, &conn, expose_personas)
        .expect("build registry")
}

/// Final prompt names in the registry, sorted (descriptor order).
fn descriptor_names(registry: &PromptRegistry) -> Vec<String> {
    registry.descriptors().into_iter().map(|p| p.name).collect()
}

/// Build an `Arc<McpState>` carrying a persona-enabled registry so the
/// `prompts/get` handler has a name to look up. Mirrors
/// `tests/mcp_prompts.rs::build_state_for_prompts` with personas ON.
fn build_state(paths: &tome::paths::Paths) -> Arc<McpState> {
    let registry = build(paths, true);
    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());

    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry,
        reranker_entry,
        prompt_registry: Arc::new(registry),
        host_harness: None,
    })
}

/// Invoke `prompts::handle_get` on a current-thread runtime, returning the
/// single user-role message text.
fn invoke_get(
    state: Arc<McpState>,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Result<String, rmcp::ErrorData> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let response = rt.block_on(prompts::handle_get(state, name.to_owned(), arguments))?;
    assert_eq!(response.messages.len(), 1, "single user-role message");
    match &response.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => Ok(text.clone()),
        other => panic!("expected text content, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T109 — toggle + shape.
// ---------------------------------------------------------------------------

#[test]
fn off_by_default_no_personas() {
    // The enabled plugin ships an agent AND a user-invocable command. With
    // `expose_personas = false` the prompt surface is exactly Phase 5: the
    // command appears, NO `<name>-persona`, NO `drop-persona`.
    let agent = "---\nname: reviewer\ndescription: A code reviewer.\n---\nReview carefully.\n";
    let cmd = "---\nname: fix-issue\ndescription: Fix it.\n---\nFix $ARGUMENTS\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[("fix-issue", cmd)],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths, false);
    let names = descriptor_names(&registry);

    // Phase 9 / US3: the always-on reserved `add-tome-conversion-skill`
    // built-in is present regardless of the persona toggle. Filter it out
    // before pinning the persona-relevant surface.
    assert!(
        names.iter().any(|n| n == "add-tome-conversion-skill"),
        "reserved built-in present regardless of persona toggle; got {names:?}",
    );
    let names: Vec<String> = names
        .into_iter()
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();

    assert_eq!(
        names,
        vec!["plug__fix-issue"],
        "personas off: only the Phase 5 command prompt is present; got {names:?}",
    );
    assert!(
        !names.iter().any(|n| n.ends_with("-persona")),
        "no persona prompt when toggle is off; got {names:?}",
    );
    assert!(
        !names.iter().any(|n| n == prompts::DROP_PERSONA_NAME),
        "no drop-persona when toggle is off; got {names:?}",
    );
}

#[test]
fn on_exposes_personas_and_drop() {
    // Two enabled agents across two plugins (non-clashing names) plus one
    // command. Personas on → exactly one `<name>-persona` per agent + one
    // `drop-persona`, and the Phase 5 command prompt is unchanged.
    let reviewer = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let planner = "---\nname: planner\ndescription: Planner.\n---\nPlan.\n";
    let cmd = "---\nname: fix-issue\ndescription: Fix it.\n---\nFix $ARGUMENTS\n";
    let plugins = [
        PluginSpec {
            name: "plug-a",
            agents: &[("reviewer", reviewer)],
            commands: &[("fix-issue", cmd)],
        },
        PluginSpec {
            name: "plug-b",
            agents: &[("planner", planner)],
            commands: &[],
        },
    ];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths, true);
    let names = descriptor_names(&registry);

    let persona_count = names.iter().filter(|n| n.ends_with("-persona")).count();
    // Two agent personas + the one reserved drop-persona = 3 `-persona`
    // suffixes; the drop is counted via the suffix too, so check both.
    let agent_personas: Vec<&String> = names
        .iter()
        .filter(|n| n.ends_with("-persona") && *n != prompts::DROP_PERSONA_NAME)
        .collect();
    assert_eq!(
        agent_personas,
        vec!["planner-persona", "reviewer-persona"],
        "exactly one `<name>-persona` per enabled agent; got {names:?}",
    );
    assert_eq!(
        names
            .iter()
            .filter(|n| *n == prompts::DROP_PERSONA_NAME)
            .count(),
        1,
        "exactly one reserved drop-persona regardless of agent count; got {names:?}",
    );
    assert_eq!(
        persona_count, 3,
        "two agent personas + one drop; got {names:?}"
    );
    assert!(
        names.contains(&"plug-a__fix-issue".to_owned()),
        "the Phase 5 command prompt is unchanged; got {names:?}",
    );
}

#[test]
fn get_wraps_and_substitutes() {
    // The agent body references a Phase 5 built-in (`${TOME_PLUGIN_NAME}`)
    // so we can prove the shared substitution sweep ran inside the wrapper.
    let agent =
        "---\nname: reviewer\ndescription: Reviewer.\n---\nYou belong to ${TOME_PLUGIN_NAME}.\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);
    let state = build_state(&paths);

    let mut args = Map::new();
    args.insert("args".into(), json!("be thorough"));

    let text = invoke_get(state, "reviewer-persona", Some(args)).expect("prompts/get ok");

    // (1) Role-assumption preamble, display name = frontmatter `name`.
    assert!(
        text.starts_with("Assume the following reviewer persona until instructed otherwise."),
        "persona preamble; got: {text:?}",
    );
    // (2) The frontmatter-stripped body, substituted, inside the wrapping
    //     tag named after the derived persona slug.
    assert!(
        text.contains("<reviewer-persona>"),
        "open wrapping tag uses the persona slug; got: {text:?}",
    );
    assert!(
        text.contains("</reviewer-persona>"),
        "close wrapping tag uses the persona slug; got: {text:?}",
    );
    assert!(
        text.contains("You belong to plug."),
        "Phase 5 built-in `${{TOME_PLUGIN_NAME}}` substituted inside the wrapper; got: {text:?}",
    );
    // (3) The caller's free-form `args` substitutes into the trailing
    //     `$ARGUMENTS` reference of the template.
    assert!(
        text.contains("While acting as the reviewer persona, you must: be thorough"),
        "free-form args resolved into the template's $ARGUMENTS; got: {text:?}",
    );
    // (4) T4-2: the agent frontmatter must be STRIPPED — a regression that
    //     wrapped the raw file would leak the `name:`/`description:` lines
    //     and the YAML fences into the conversational body.
    assert!(
        !text.contains("name: reviewer"),
        "frontmatter `name:` line absent from the rendered body; got: {text:?}",
    );
    assert!(
        !text.contains("description: Reviewer."),
        "frontmatter `description:` line absent from the rendered body; got: {text:?}",
    );
    assert!(
        !text.contains("---"),
        "the YAML frontmatter fences are stripped; got: {text:?}",
    );
}

#[test]
fn get_resolves_plugin_version_builtin() {
    // C4-1 regression: `${TOME_PLUGIN_VERSION}` must resolve to the agent's
    // real plugin version (from the `plugin.json` `version` field threaded
    // through `EnabledAgent.plugin_version`), NOT an empty string. The
    // fixture's plugin.json declares `"version": "1.0.0"`.
    let agent = "---\nname: reviewer\ndescription: Reviewer.\n---\nRunning under v${TOME_PLUGIN_VERSION}.\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);
    let state = build_state(&paths);

    let text = invoke_get(state, "reviewer-persona", None).expect("prompts/get ok");

    assert!(
        text.contains("Running under v1.0.0."),
        "${{TOME_PLUGIN_VERSION}} resolves to the real version inside the persona body; got: {text:?}",
    );
    assert!(
        !text.contains("Running under v."),
        "the version built-in is not empty; got: {text:?}",
    );
}

#[test]
fn long_agent_name_persona_suffix_preserved() {
    // C4-2 / T4-4: an agent whose name is longer than the 48-char override
    // budget must still derive a persona slug that ENDS in `-persona` (the
    // suffix is load-bearing for the user-facing `<name>-persona` shape).
    let long_name = "a".repeat(80);
    let agent = format!("---\nname: {long_name}\ndescription: Long.\n---\nBody.\n");
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("long-agent", agent.as_str())],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths, true);
    let names = descriptor_names(&registry);

    let persona = names
        .iter()
        .find(|n| n.ends_with("-persona") && *n != prompts::DROP_PERSONA_NAME)
        .unwrap_or_else(|| panic!("an agent persona is present; got {names:?}"));
    assert!(
        persona.ends_with("-persona"),
        "the persona slug terminates in `-persona` even for a long name; got {persona:?}",
    );
    assert!(
        persona.chars().count() <= 48,
        "the persona slug stays within the override cap; got {} chars: {persona:?}",
        persona.chars().count(),
    );
}

#[test]
fn persona_path_unresolvable_warns_and_skips() {
    // T4-3: with personas ON, if one agent's `.md` is deleted after enable,
    // the registry still builds; the surviving agent's persona + the
    // reserved drop-persona are present, and the broken one is absent.
    let reviewer = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let planner = "---\nname: planner\ndescription: Planner.\n---\nPlan.\n";
    let plugins = [
        PluginSpec {
            name: "plug-a",
            agents: &[("reviewer", reviewer)],
            commands: &[],
        },
        PluginSpec {
            name: "plug-b",
            agents: &[("planner", planner)],
            commands: &[],
        },
    ];
    let (tmp, paths) = stage("acme", &plugins);

    // Corrupt `planner`'s source on disk (truncate the frontmatter so the
    // parse fails) after it was indexed + enrolled. The registry build
    // re-parses from disk, so this triggers the warn-and-skip path.
    let planner_md = tmp
        .path()
        .join("catalog")
        .join("plug-b")
        .join("agents")
        .join("planner.md");
    fs::write(
        &planner_md,
        "---\nname: planner\nThis is broken: no closing fence\n",
    )
    .expect("corrupt planner frontmatter");

    let registry = build(&paths, true);
    let names = descriptor_names(&registry);

    assert!(
        names.contains(&"reviewer-persona".to_owned()),
        "the surviving agent's persona is present; got {names:?}",
    );
    assert!(
        names.contains(&prompts::DROP_PERSONA_NAME.to_owned()),
        "the reserved drop-persona is still present; got {names:?}",
    );
    assert!(
        !names.contains(&"planner-persona".to_owned()),
        "the broken agent's persona is skipped; got {names:?}",
    );
}

#[test]
fn name_from_frontmatter_else_stem() {
    // Plugin `p1` ships `agents/file-stem.md` WITH frontmatter `name:
    // declared-name`; plugin `p2` ships `agents/stem-only.md` with NO `name`
    // field. The persona slug + the wrapper display name follow the rule:
    // frontmatter `name` when present, else the filename stem.
    let with_name = "---\nname: declared-name\ndescription: Has a name.\n---\nBody.\n";
    let stem_only = "---\ndescription: No name field.\n---\nBody.\n";
    let plugins = [
        PluginSpec {
            name: "p1",
            agents: &[("file-stem", with_name)],
            commands: &[],
        },
        PluginSpec {
            name: "p2",
            agents: &[("stem-only", stem_only)],
            commands: &[],
        },
    ];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths, true);
    let names = descriptor_names(&registry);

    assert!(
        names.contains(&"declared-name-persona".to_owned()),
        "frontmatter `name` drives the persona slug; got {names:?}",
    );
    assert!(
        names.contains(&"stem-only-persona".to_owned()),
        "filename stem is the fallback when `name` is absent; got {names:?}",
    );

    // The wrapper display name also follows frontmatter-else-stem: get the
    // stem-only persona and confirm the preamble uses the stem.
    let state = build_state(&paths);
    let text = invoke_get(state, "stem-only-persona", None).expect("prompts/get ok");
    assert!(
        text.starts_with("Assume the following stem-only persona until instructed otherwise."),
        "display name falls back to the filename stem; got: {text:?}",
    );
}

#[test]
fn drop_persona_get_returns_fixed_body() {
    // The reserved drop-persona returns the fixed body verbatim — no
    // wrapper, no substitution, no on-disk file read. Supplying args has no
    // effect (the drop prompt declares none).
    let agent = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);
    let state = build_state(&paths);

    // Verbatim from PRD §2.4 / `contracts/agent-personas.md` §
    // `drop-persona` (mirrors the private `DROP_PERSONA_BODY` const —
    // inlined so this pins the contract text, not just the const).
    const DROP_BODY: &str =
        "Stop acting as any assumed persona and return to your default behaviour\nand personality.";

    let text = invoke_get(state, prompts::DROP_PERSONA_NAME, None).expect("prompts/get ok");
    assert_eq!(
        text, DROP_BODY,
        "drop-persona returns the fixed body verbatim",
    );
    assert!(
        !text.contains("<drop-persona>"),
        "the fixed drop body is not template-wrapped; got: {text:?}",
    );
}

// ---------------------------------------------------------------------------
// T111 — byte-stable JSON wire-pins.
// ---------------------------------------------------------------------------

#[test]
fn persona_list_descriptors_are_byte_stable() {
    // Pins the persona `<name>-persona` + `drop-persona` entries in the
    // `prompts/list` `Vec<Prompt>` shape (mirrors
    // `tests/mcp_prompts_list_json_shape.rs`). A single agent keeps the
    // expected array small and deterministic. The agent-persona descriptor
    // carries the catch-all optional `args` schema (Case B); drop-persona
    // carries no argument schema.
    let agent = "---\nname: reviewer\ndescription: Reviews code.\n---\nReview.\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths, true);
    // Phase 9 / US3: the always-on reserved `add-tome-conversion-skill`
    // built-in sorts first in the descriptor list. Assert its presence,
    // then filter it out so this pin stays decoupled from its text and
    // pins only the persona-derived descriptors.
    let descriptors = registry.descriptors();
    assert!(
        descriptors
            .iter()
            .any(|d| d.name == "add-tome-conversion-skill"),
        "reserved built-in must be advertised; got {descriptors:?}",
    );
    let persona_descriptors: Vec<_> = descriptors
        .into_iter()
        .filter(|d| d.name != "add-tome-conversion-skill")
        .collect();
    let serialised = serde_json::to_value(persona_descriptors).expect("serialise");

    let expected: Value = json!([
        {
            "name": "drop-persona",
            "description": "Stop acting as any assumed agent persona and return to default behaviour."
        },
        {
            "name": "reviewer-persona",
            "description": "Assume the `reviewer` agent persona (advisory conversational context, not enforced configuration — the agent may drift or ignore it; not the isolation a native subagent provides).",
            "arguments": [
                {
                    "name": "args",
                    "description": "Optional free-form input passed to the entry as a single positional argument.",
                    "required": false
                }
            ]
        }
    ]);

    assert_eq!(
        serialised,
        expected,
        "persona prompts/list wire-shape drift detected;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}

#[test]
fn persona_get_envelope_is_byte_stable() {
    // Pins the persona `prompts/get` single-`user`-message envelope. The
    // body is kept free of substitution markers so the rendered text is the
    // template-wrapped agent body verbatim. With NO args supplied, Stage 3
    // is skipped, so the template's trailing `$ARGUMENTS` passes through
    // unchanged (the documented Phase 5 no-args behaviour).
    let agent = "---\nname: reviewer\ndescription: Reviews code.\n---\nReview carefully.\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[],
    }];
    let (_tmp, paths) = stage("acme", &plugins);
    let state = build_state(&paths);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let response = rt
        .block_on(prompts::handle_get(state, "reviewer-persona".into(), None))
        .expect("prompts/get ok");
    let serialised = serde_json::to_value(&response).expect("serialise");

    let expected: Value = json!({
        "description": "Assume the `reviewer` agent persona (advisory conversational context, not enforced configuration — the agent may drift or ignore it; not the isolation a native subagent provides).",
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": "Assume the following reviewer persona until instructed otherwise.\n\n<reviewer-persona>\nReview carefully.\n\n</reviewer-persona>\n\nWhile acting as the reviewer persona, you must: $ARGUMENTS"
                }
            }
        ]
    });

    assert_eq!(
        serialised,
        expected,
        "persona prompts/get wire-shape drift detected;\n  got:      {}\n  expected: {}",
        serde_json::to_string_pretty(&serialised).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap(),
    );
}
