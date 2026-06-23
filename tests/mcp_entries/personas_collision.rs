//! Phase 6 / US4 — persona collision namespace (T110).
//!
//! Per `contracts/agent-personas.md` § Collision namespace (FR-061 /
//! FR-066 / FR-063). Persona derived names join the SINGLE Phase 5
//! prompt-name collision namespace alongside command + skill names. This
//! file pins:
//!
//! - The agent-clash plugin prefix (`<plugin>-<name>-persona`) applies
//!   ONLY to agents whose `<name>` clashes across two or more enabled
//!   plugins; a non-clashing agent stays `<name>-persona`.
//! - The `drop-persona` reservation: any command/skill/persona that would
//!   derive to `drop-persona` is counter-suffixed; the reserved
//!   `drop-persona` keeps the base name.
//! - The union namespace: a persona vs command name collision is resolved
//!   by the same counter-suffix backstop (one shared namespace, not two).
//!
//! Reuses the on-disk fixture discipline from `tests/personas.rs`.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::PromptRegistry;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Fixture helpers (same shape as tests/personas.rs).
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
            profile: None,
        },
    )
    .expect("open index db")
}

struct PluginSpec<'a> {
    name: &'a str,
    agents: &'a [(&'a str, &'a str)],
    commands: &'a [(&'a str, &'a str)],
}

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

fn build(paths: &tome::paths::Paths) -> PromptRegistry {
    let conn = open_index(paths);
    PromptRegistry::build_for_workspace(&global(), paths, &conn, true).expect("build registry")
}

fn names(registry: &PromptRegistry) -> Vec<String> {
    registry.descriptors().into_iter().map(|p| p.name).collect()
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn clash_prefix_only_on_clash() {
    // Two plugins each ship an agent named `reviewer` (the FR-072 clash
    // set). A third plugin ships a non-clashing agent `planner`. The two
    // clashing agents get `<plugin>-reviewer-persona`; the non-clashing one
    // stays `planner-persona`.
    let reviewer_a = "---\nname: reviewer\ndescription: A reviewer.\n---\nReview.\n";
    let reviewer_b = "---\nname: reviewer\ndescription: B reviewer.\n---\nReview.\n";
    let planner = "---\nname: planner\ndescription: Planner.\n---\nPlan.\n";
    let plugins = [
        PluginSpec {
            name: "plug-a",
            agents: &[("reviewer", reviewer_a)],
            commands: &[],
        },
        PluginSpec {
            name: "plug-b",
            agents: &[("reviewer", reviewer_b)],
            commands: &[],
        },
        PluginSpec {
            name: "plug-c",
            agents: &[("planner", planner)],
            commands: &[],
        },
    ];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths);
    let got = names(&registry);

    assert!(
        got.contains(&"plug-a-reviewer-persona".to_owned()),
        "clashing agent gets the plugin-prefixed persona slug; got {got:?}",
    );
    assert!(
        got.contains(&"plug-b-reviewer-persona".to_owned()),
        "clashing agent gets the plugin-prefixed persona slug; got {got:?}",
    );
    assert!(
        !got.contains(&"reviewer-persona".to_owned()),
        "no bare `reviewer-persona` survives the clash prefix; got {got:?}",
    );
    assert!(
        got.contains(&"planner-persona".to_owned()),
        "the non-clashing agent keeps the bare `<name>-persona`; got {got:?}",
    );
}

#[test]
fn drop_persona_reserved() {
    // A command whose `prompt_name` override would derive to `drop-persona`
    // collides with the reserved drop. The reservation wins: `drop-persona`
    // keeps the base name (its seeded empty `indexed_at` + empty
    // catalog/plugin sort it first in the bucket) and the command is
    // counter-suffixed to `drop-persona2`.
    let agent = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let cmd = "---\nname: cmd\ndescription: Tries to grab drop-persona.\nprompt_name: drop-persona\n---\nbody\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[("cmd", cmd)],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths);
    let got = names(&registry);

    assert!(
        got.contains(&"drop-persona".to_owned()),
        "the reserved drop-persona keeps the base name; got {got:?}",
    );
    assert!(
        got.contains(&"drop-persona2".to_owned()),
        "the colliding command is counter-suffixed; got {got:?}",
    );
    // Exactly one entry holds the reserved base name (the drop), so the
    // colliding command must NOT be it.
    let drop_entry = registry
        .lookup("drop-persona")
        .expect("drop-persona present");
    assert_eq!(
        drop_entry.persona,
        tome::mcp::prompts::PersonaRole::Drop,
        "the base `drop-persona` name belongs to the reserved drop prompt, not the command",
    );
    let cmd_entry = registry
        .lookup("drop-persona2")
        .expect("suffixed command present");
    assert_eq!(
        cmd_entry.persona,
        tome::mcp::prompts::PersonaRole::None,
        "the counter-suffixed entry is the Phase 5 command",
    );
}

#[test]
fn union_namespace() {
    // A persona slug and a command's derived name collide in the SINGLE
    // shared namespace. Agent `reviewer` → persona slug `reviewer-persona`;
    // a command with `prompt_name: reviewer-persona` derives to the same
    // name. The agent + command are enabled in the same `lifecycle::enable`
    // call so they share an `indexed_at`; the tie-break then falls to the
    // identity tuple `(catalog, plugin, kind, name)` where kind `agent`
    // sorts before `command` — so the persona keeps the base name and the
    // command is counter-suffixed. R4-1: this is the REAL `indexed_at`
    // tie-break (NOT the persona's old empty-seed always-wins behaviour);
    // the `indexed_at_decides_persona_vs_command_collision` test below
    // pins the case where an earlier command beats the persona. The point
    // here is the SHARED namespace: one bucket, two members.
    let agent = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let cmd = "---\nname: cmd\ndescription: Collides with the persona slug.\nprompt_name: reviewer-persona\n---\nbody\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[("cmd", cmd)],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    let registry = build(&paths);
    let got = names(&registry);

    assert!(
        got.contains(&"reviewer-persona".to_owned()),
        "the persona keeps the base name in the shared namespace; got {got:?}",
    );
    assert!(
        got.contains(&"reviewer-persona2".to_owned()),
        "the colliding command is counter-suffixed in the SAME namespace; got {got:?}",
    );

    let base = registry
        .lookup("reviewer-persona")
        .expect("base reviewer-persona present");
    assert_eq!(
        base.persona,
        tome::mcp::prompts::PersonaRole::Agent,
        "the agent persona wins the base name (same indexed_at → kind `agent` < `command` tuple tie-break)",
    );
    let suffixed = registry
        .lookup("reviewer-persona2")
        .expect("suffixed entry present");
    assert_eq!(
        suffixed.persona,
        tome::mcp::prompts::PersonaRole::None,
        "the counter-suffixed entry is the Phase 5 command",
    );

    // The collision was recorded over the union (one bucket, two members).
    assert!(
        registry
            .collisions
            .iter()
            .any(|c| c.base_name == "reviewer-persona" && c.entries.len() == 2),
        "a single collision bucket spans the persona + command; got {:?}",
        registry.collisions,
    );
}

#[test]
fn indexed_at_decides_persona_vs_command_collision() {
    // R4-1: a `<name>-persona` is NOT automatically the collision winner —
    // it carries the agent's REAL `indexed_at` and tie-breaks by
    // `indexed_at ASC` like every other entry (FR-062). Here a command's
    // `indexed_at` is backdated EARLIER than the agent's, so the COMMAND
    // wins the base `reviewer-persona` name and the persona is
    // counter-suffixed — the case the old empty-seed bug made impossible.
    let agent = "---\nname: reviewer\ndescription: Reviewer.\n---\nReview.\n";
    let cmd = "---\nname: cmd\ndescription: Collides with the persona slug.\nprompt_name: reviewer-persona\n---\nbody\n";
    let plugins = [PluginSpec {
        name: "plug",
        agents: &[("reviewer", agent)],
        commands: &[("cmd", cmd)],
    }];
    let (_tmp, paths) = stage("acme", &plugins);

    // Force the timestamps: command earlier, agent later. Both rows were
    // indexed in the same enable call (identical `indexed_at`), so we
    // separate them explicitly to exercise the `indexed_at ASC` arm rather
    // than the tuple fallback.
    {
        let conn = open_index(&paths);
        conn.execute(
            "UPDATE skills SET indexed_at = '2026-01-01T00:00:00Z' WHERE kind = 'command' AND name = 'cmd'",
            [],
        )
        .expect("backdate command");
        conn.execute(
            "UPDATE skills SET indexed_at = '2026-06-01T00:00:00Z' WHERE kind = 'agent' AND name = 'reviewer'",
            [],
        )
        .expect("forward-date agent");
    }

    let registry = build(&paths);
    let got = names(&registry);

    assert!(
        got.contains(&"reviewer-persona".to_owned()),
        "the earlier-indexed command keeps the base name; got {got:?}",
    );
    assert!(
        got.contains(&"reviewer-persona2".to_owned()),
        "the later-indexed persona is counter-suffixed; got {got:?}",
    );

    let base = registry
        .lookup("reviewer-persona")
        .expect("base reviewer-persona present");
    assert_eq!(
        base.persona,
        tome::mcp::prompts::PersonaRole::None,
        "the EARLIER command wins the base name — the persona does NOT automatically win (R4-1)",
    );
    let suffixed = registry
        .lookup("reviewer-persona2")
        .expect("suffixed persona present");
    assert_eq!(
        suffixed.persona,
        tome::mcp::prompts::PersonaRole::Agent,
        "the later-indexed agent persona is the counter-suffixed entry",
    );
}
