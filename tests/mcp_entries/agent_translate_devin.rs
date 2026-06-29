//! Phase 2 — Devin CLI dir-per-agent translation pin.
use tome::harness::agents::CanonicalAgent;
use tome::harness::devin::DEVIN;
use tome::harness::{AgentFormat, AgentPathStrategy, HarnessModule};

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "reviewer".into(),
        description: Some("Reviews code".into()),
        body: "You review.\n".into(),
        model: Some("sonnet".into()),
        tools: Some(vec!["Read".into(), "Bash".into(), "Edit".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn devin_dir_per_agent_allowed_tools_alias_model() {
    let reg = tome::model_registry::test_registry();
    assert_eq!(
        DEVIN.agent_path_strategy(),
        AgentPathStrategy::DirPerAgent {
            inner_filename: "AGENT.md"
        }
    );
    let t = DEVIN
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.format, AgentFormat::MarkdownYaml);
    // filename is the SUBDIR name (no extension) — the reconciler appends /AGENT.md.
    assert_eq!(t.filename, "myplugin__reviewer");
    assert_eq!(t.dir, std::path::PathBuf::from(".devin/agents"));
    assert!(t.rendered.contains("name: reviewer"));
    assert!(t.rendered.contains("description: Reviews code"));
    // sonnet alias passes through verbatim.
    assert!(t.rendered.contains("model: sonnet"), "{}", t.rendered);
    // allowed-tools (renamed), Devin-lowercase, Bash→exec.
    assert!(t.rendered.contains("allowed-tools"), "{}", t.rendered);
    assert!(t.rendered.contains("read"));
    assert!(t.rendered.contains("exec"));
    assert!(t.rendered.contains("edit"));
}

#[test]
fn devin_no_description_haiku_dropped_privileged_dropped() {
    let reg = tome::model_registry::test_registry();
    let canonical = CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "helper".into(),
        description: None,
        body: "You help.\n".into(),
        model: Some("haiku".into()),
        tools: None,
        disallowed_tools: None,
        hooks: Some(serde_json::json!({"x": 1})),
        mcp_servers: None,
        permission_mode: Some("ask".into()),
    };
    let t = DEVIN
        .translate_agent(&canonical, false, &reg)
        .expect("translate");
    // description omitted when absent — Devin does NOT synthesize one.
    assert!(!t.rendered.contains("description:"), "{}", t.rendered);
    // haiku has no Devin alias → model is dropped, not emitted.
    assert!(t.dropped_fields.contains(&"model".to_owned()));
    assert!(!t.rendered.contains("model:"), "{}", t.rendered);
    // privileged fields recorded as dropped.
    assert!(t.dropped_fields.contains(&"hooks".to_owned()));
    assert!(t.dropped_fields.contains(&"permissionMode".to_owned()));
}

#[test]
#[ignore = "live-probe: confirm Devin reads .devin/agents/<plugin>__<name>/AGENT.md (dir-per-agent)"]
fn devin_reads_dir_per_agent_live_probe() {}
