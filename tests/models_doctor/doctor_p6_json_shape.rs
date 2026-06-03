//! Phase 6 / US5 — byte-stable JSON wire pins for the five new doctor
//! records (T132). Mirrors the Phase 4/5 pin style in `tests/doctor_json.rs`:
//! each `*Report` is constructed directly and serialised, the exact wire shape
//! asserted. These records are emit-only `Serialize` types consumed by `jq`
//! and by harnesses, so a field rename / order drift must break loudly
//! (NFR-011).

use tome::doctor::report::{
    AgentHarnessEntry, AgentsReport, CatalogPlugin, DroppedFieldEntry, GuardrailsFileEntry,
    GuardrailsReport, HookEventEntry, HookPluginEntry, HooksReport, PersonaEntry, PersonaReport,
    PrivilegeAgentEntry, PrivilegeEscalationReport, PrivilegePluginEntry, RulesCopyState,
    SubsystemHealth,
};

// ---------------------------------------------------------------------------
// HooksReport
// ---------------------------------------------------------------------------

#[test]
fn hooks_report_wire_shape_is_byte_stable() {
    let report = HooksReport {
        plugins: vec![HookPluginEntry {
            catalog: "acme".to_owned(),
            plugin: "plug".to_owned(),
            contributed: vec![HookEventEntry {
                event: "PostToolUse".to_owned(),
                count: 1,
            }],
            missing: vec![HookEventEntry {
                event: "PreToolUse".to_owned(),
                count: 2,
            }],
        }],
    };

    // Top-level field order + nested key order are the contract.
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"plugins":[{"catalog":"acme","plugin":"plug","contributed":[{"event":"PostToolUse","count":1}],"missing":[{"event":"PreToolUse","count":2}]}]}"#,
    );
}

#[test]
fn hooks_report_empty_wire_shape() {
    let report = HooksReport { plugins: vec![] };
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(json, r#"{"plugins":[]}"#);
}

// ---------------------------------------------------------------------------
// GuardrailsReport
// ---------------------------------------------------------------------------

#[test]
fn guardrails_report_wire_shape_is_byte_stable() {
    let report = GuardrailsReport {
        files: vec![GuardrailsFileEntry {
            path: std::path::PathBuf::from("/proj/CLAUDE.md"),
            present: vec![
                CatalogPlugin {
                    catalog: "acme".to_owned(),
                    plugin: "plug".to_owned(),
                },
                CatalogPlugin {
                    catalog: "gone".to_owned(),
                    plugin: "ghost".to_owned(),
                },
            ],
            orphaned: vec![CatalogPlugin {
                catalog: "gone".to_owned(),
                plugin: "ghost".to_owned(),
            }],
            suppressed: vec![CatalogPlugin {
                catalog: "acme".to_owned(),
                plugin: "plug".to_owned(),
            }],
        }],
    };

    // TEST-3: pin via `to_string` against an exact literal — `to_value` is
    // key-order-insensitive and NFR-011 is an ordering property, so the wire
    // order (`path` < `present` < `orphaned` < `suppressed`) must be frozen.
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"files":[{"path":"/proj/CLAUDE.md","present":[{"catalog":"acme","plugin":"plug"},{"catalog":"gone","plugin":"ghost"}],"orphaned":[{"catalog":"gone","plugin":"ghost"}],"suppressed":[{"catalog":"acme","plugin":"plug"}]}]}"#,
    );
}

// ---------------------------------------------------------------------------
// AgentsReport
// ---------------------------------------------------------------------------

#[test]
fn agents_report_wire_shape_is_byte_stable() {
    let report = AgentsReport {
        harnesses: vec![AgentHarnessEntry {
            harness: "claude-code".to_owned(),
            present: vec!["plug__reviewer.md".to_owned()],
            orphaned: vec!["ghost__gone.md".to_owned()],
            dropped_fields: vec![DroppedFieldEntry {
                agent: "plug__reviewer".to_owned(),
                fields: vec!["model".to_owned(), "color".to_owned()],
            }],
        }],
    };

    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"harnesses":[{"harness":"claude-code","present":["plug__reviewer.md"],"orphaned":["ghost__gone.md"],"dropped_fields":[{"agent":"plug__reviewer","fields":["model","color"]}]}]}"#,
    );
}

// ---------------------------------------------------------------------------
// PrivilegeEscalationReport
// ---------------------------------------------------------------------------

#[test]
fn privilege_escalation_report_wire_shape_is_byte_stable() {
    let report = PrivilegeEscalationReport {
        plugins: vec![PrivilegePluginEntry {
            catalog: "acme".to_owned(),
            plugin: "plug".to_owned(),
            agents: vec![PrivilegeAgentEntry {
                name: "reviewer".to_owned(),
                fields: vec![
                    "hooks".to_owned(),
                    "mcpServers".to_owned(),
                    "permissionMode".to_owned(),
                ],
            }],
        }],
    };

    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"plugins":[{"catalog":"acme","plugin":"plug","agents":[{"name":"reviewer","fields":["hooks","mcpServers","permissionMode"]}]}]}"#,
    );
}

// ---------------------------------------------------------------------------
// PersonaReport
// ---------------------------------------------------------------------------

#[test]
fn persona_report_wire_shape_is_byte_stable() {
    let report = PersonaReport {
        personas: vec![
            PersonaEntry {
                catalog: "acme".to_owned(),
                plugin: "plug".to_owned(),
                agent_name: "reviewer".to_owned(),
                resolved_persona_name: "reviewer-persona".to_owned(),
                clash_prefixed: false,
            },
            PersonaEntry {
                catalog: "acme".to_owned(),
                plugin: "plug-b".to_owned(),
                agent_name: "reviewer".to_owned(),
                resolved_persona_name: "plug-b-reviewer-persona".to_owned(),
                clash_prefixed: true,
            },
        ],
        drop_persona: "drop-persona".to_owned(),
    };

    // TEST-3: pin via `to_string` against an exact literal — `to_value` is
    // key-order-insensitive and NFR-011 is an ordering property, so the wire
    // order (each entry's `catalog` < `plugin` < `agent_name` <
    // `resolved_persona_name` < `clash_prefixed`, then top-level `personas` <
    // `drop_persona`) must be frozen.
    let json = serde_json::to_string(&report).expect("serialise");
    assert_eq!(
        json,
        r#"{"personas":[{"catalog":"acme","plugin":"plug","agent_name":"reviewer","resolved_persona_name":"reviewer-persona","clash_prefixed":false},{"catalog":"acme","plugin":"plug-b","agent_name":"reviewer","resolved_persona_name":"plug-b-reviewer-persona","clash_prefixed":true}],"drop_persona":"drop-persona"}"#,
    );
}

// ---------------------------------------------------------------------------
// TEST-4: byte-stable variant pins for two Phase 4 doctor enums that derive
// `Serialize` independently. `ProjectBindingState` is intentionally NOT pinned
// here — it is a struct (not a simple enum) carrying a `WorkspaceName` newtype
// and a nested `RulesCopyState`, so it is carried forward as backlog rather
// than forced. Both enums use `#[serde(rename_all = "snake_case")]`, so each
// unit variant is a quoted snake_case scalar; a rename drift breaks loudly.
// ---------------------------------------------------------------------------

#[test]
fn subsystem_health_variants_wire_shape_is_byte_stable() {
    let s = |h: SubsystemHealth| serde_json::to_string(&h).expect("serialise");
    assert_eq!(s(SubsystemHealth::Ok), r#""ok""#);
    assert_eq!(s(SubsystemHealth::Drift), r#""drift""#);
    assert_eq!(s(SubsystemHealth::Broken), r#""broken""#);
    assert_eq!(s(SubsystemHealth::UserOwned), r#""user_owned""#);
    assert_eq!(s(SubsystemHealth::NotApplicable), r#""not_applicable""#);
}

#[test]
fn rules_copy_state_variants_wire_shape_is_byte_stable() {
    let s = |r: RulesCopyState| serde_json::to_string(&r).expect("serialise");
    assert_eq!(s(RulesCopyState::Match), r#""match""#);
    assert_eq!(s(RulesCopyState::Missing), r#""missing""#);
    assert_eq!(s(RulesCopyState::Drift), r#""drift""#);
    assert_eq!(s(RulesCopyState::SourceMissing), r#""source_missing""#);
}
