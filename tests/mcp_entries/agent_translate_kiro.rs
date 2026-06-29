//! Phase 2 — Kiro IDE native-agent translation pin.
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::kiro::KIRO;

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: "reviewer".into(),
        description: Some("Reviews code".into()),
        body: "You review.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec![
            "Read".into(),
            "Grep".into(),
            "Edit".into(),
            "Bash".into(),
        ]),
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn kiro_slug_name_drops_model_category_tags() {
    let reg = tome::model_registry::test_registry();
    let t = KIRO
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.filename, "midnight-expert__reviewer.md");
    assert_eq!(t.dir, std::path::PathBuf::from(".kiro/agents"));
    // name has no underscores (lowercase+hyphens only). Clean name = "reviewer".
    assert!(t.rendered.contains("name: reviewer"));
    assert!(t.rendered.contains("description: Reviews code"));
    // model is DROPPED + recorded (registry ids rejected; ignored in dispatch).
    assert!(!t.rendered.contains("model:"), "{}", t.rendered);
    assert!(t.dropped_fields.contains(&"model".to_owned()));
    // tools → category tags: read (Read+Grep), write (Edit), shell (Bash).
    assert!(t.rendered.contains("read"));
    assert!(t.rendered.contains("write"));
    assert!(t.rendered.contains("shell"));
}

#[test]
#[ignore = "live-probe: confirm Kiro IDE loads .kiro/agents/<plugin>__<name>.md (model honoring is best-effort, #6637)"]
fn kiro_reads_agent_file_live_probe() {}
