//! Phase 2 — Goose Custom Agent translation pin.
use tome::harness::agents::CanonicalAgent;
use tome::harness::goose::GOOSE;
use tome::harness::{AgentFormat, HarnessModule};

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "reviewer".into(),
        description: Some("Reviews code".into()),
        body: "You review.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn goose_emits_name_desc_resolved_model_drops_tools() {
    let reg = tome::model_registry::test_registry();
    let t = GOOSE
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.format, AgentFormat::MarkdownYaml);
    assert_eq!(t.filename, "myplugin__reviewer.md");
    assert_eq!(t.dir, std::path::PathBuf::from(".agents/agents"));
    assert!(t.rendered.contains("name: reviewer"));
    assert!(t.rendered.contains("description: Reviews code"));
    // opus → registry bare anthropic id (Goose passes it to the Anthropic provider).
    assert!(
        t.rendered.contains("model: claude-opus-4-5"),
        "{}",
        t.rendered
    );
    // Goose has no per-agent tools → dropped + recorded; never emitted.
    assert!(!t.rendered.contains("tools:"));
    assert!(t.dropped_fields.contains(&"tools".to_owned()));
}

#[test]
#[ignore = "live-probe: confirm Goose loads .agents/agents/<plugin>__<name>.md as a Custom Agent"]
fn goose_reads_custom_agent_live_probe() {}
