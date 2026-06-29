//! Phase 2 — GitHub Copilot native-agent translation pin (both modules).
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::copilot::COPILOT;
use tome::harness::copilot_cli::COPILOT_CLI;

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "reviewer".into(),
        description: None,
        body: "You review code.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into(), "Bash".into()]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn copilot_agent_md_omits_model_and_tools_synth_desc() {
    let reg = tome::model_registry::test_registry();
    let t = COPILOT
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.filename, "myplugin__reviewer.agent.md");
    assert_eq!(t.dir, std::path::PathBuf::from(".github/agents"));
    assert!(t.rendered.contains("name: reviewer"));
    assert!(
        t.rendered.contains("description: You review code."),
        "{}",
        t.rendered
    );
    // model omitted (inherit) — NOT recorded as dropped.
    assert!(!t.rendered.contains("model:"));
    assert!(!t.dropped_fields.contains(&"model".to_owned()));
    // tools omitted (= inherit all); never `*`; recorded dropped.
    assert!(!t.rendered.contains("tools:"));
    assert!(!t.rendered.contains('*'));
    assert!(t.dropped_fields.contains(&"tools".to_owned()));
}

#[test]
fn copilot_and_copilot_cli_emit_identical_bytes() {
    let reg = tome::model_registry::test_registry();
    let a = COPILOT.translate_agent(&agent(), false, &reg).unwrap();
    let b = COPILOT_CLI.translate_agent(&agent(), false, &reg).unwrap();
    assert_eq!(a.filename, b.filename, "co-owners must agree on filename");
    assert_eq!(a.dir, b.dir, "co-owners must agree on dir");
    assert_eq!(
        a.rendered, b.rendered,
        "co-owners must emit byte-identical content"
    );
}

#[test]
#[ignore = "live-probe: confirm Copilot reads .github/agents/<plugin>__<name>.agent.md (double extension + Tome prefix survive its parser; body <=30k)"]
fn copilot_reads_agent_md_live_probe() {}
