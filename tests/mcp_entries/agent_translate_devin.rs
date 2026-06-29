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
#[ignore = "live-probe: confirm Devin reads .devin/agents/<plugin>__<name>/AGENT.md (dir-per-agent)"]
fn devin_reads_dir_per_agent_live_probe() {}
