//! Phase 2 — Gemini CLI native-agent translation pin.
use tome::harness::AgentFormat;
use tome::harness::HarnessModule;
use tome::harness::agents::CanonicalAgent;
use tome::harness::gemini::GEMINI;

fn agent() -> CanonicalAgent {
    CanonicalAgent {
        catalog: "cat".into(),
        plugin: "midnight-expert".into(),
        name: "Reviewer".into(),
        description: None,
        body: "You review code.\nBe thorough.\n".into(),
        model: Some("opus".into()),
        tools: Some(vec!["Read".into(), "Edit".into(), "Bash".into()]),
        disallowed_tools: None,
        hooks: Some(serde_json::json!({"PreToolUse": []})),
        mcp_servers: None,
        permission_mode: None,
    }
}

#[test]
fn gemini_emits_slug_name_synth_desc_inherit_model_translated_tools() {
    let reg = tome::model_registry::test_registry();
    let t = GEMINI
        .translate_agent(&agent(), false, &reg)
        .expect("translate");
    assert_eq!(t.format, AgentFormat::MarkdownYaml);
    assert_eq!(t.filename, "midnight-expert__Reviewer.md");
    // Filename preserves the on-disk provenance; the `name` FIELD is slugged.
    assert!(t.rendered.contains("name: reviewer"), "{}", t.rendered);
    // No frontmatter description → first body line.
    assert!(
        t.rendered.contains("description: You review code."),
        "{}",
        t.rendered
    );
    // opus → inherit literal.
    assert!(t.rendered.contains("model: inherit"), "{}", t.rendered);
    // Tools → Gemini names (replace, not edit_file); Bash→run_shell_command.
    assert!(t.rendered.contains("run_shell_command"), "{}", t.rendered);
    assert!(t.rendered.contains("read_file"));
    assert!(t.rendered.contains("replace"));
    // Privileged hooks have no carrier and are recorded dropped.
    assert!(t.dropped_fields.contains(&"hooks".to_owned()));
}

/// Live-probe (NOT run in CI): confirm Gemini reads the emitted file.
#[test]
#[ignore = "live-probe: confirm Gemini CLI loads .gemini/agents/<plugin>__<name>.md and auto-delegates (enableAgents on)"]
fn gemini_reads_agent_file_live_probe() {}
