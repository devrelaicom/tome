//! Phase 2 — Pi subagent translation pin.
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::pi::PI;

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "reviewer".into(),
        description: None,
        body: "First body line.\n".into(),
        model: Some("sonnet".into()),
        tools: Some(vec!["Read".into(), "Grep".into(), "Bash".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn pi_emits_required_name_desc_commastring_tools_passthrough_model() {
    let reg = tome::model_registry::test_registry();
    let t = PI
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.filename, "myplugin__reviewer.md");
    assert_eq!(t.dir, std::path::PathBuf::from(".pi/agents"));
    assert!(t.rendered.contains("name: reviewer"));
    // description required → synthesized from first body line.
    assert!(
        t.rendered.contains("description: First body line."),
        "{}",
        t.rendered
    );
    // sonnet → registry bare id (Pi passes it through).
    assert!(
        t.rendered.contains("model: claude-sonnet-4-5"),
        "{}",
        t.rendered
    );
    // comma-string tools.
    assert!(
        t.rendered.contains("tools: 'read, grep, bash'")
            || t.rendered.contains("tools: read, grep, bash"),
        "{}",
        t.rendered
    );
}

#[test]
#[ignore = "live-probe: confirm Pi loads a project agent under agentScope=project/both + subagent extension"]
fn pi_reads_project_agent_live_probe() {}
