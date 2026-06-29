//! Phase 4 / US5.a (T375) — Subsystem enum round-trip test.
//!
//! Locks the wire format of [`tome::doctor::Subsystem`]: every variant
//! must serialise to its documented colon-separated string, and
//! deserialising the string back must produce the same variant. The
//! Phase 3 variants (`Embedder`, `Reranker`, `Index`, `Drift`,
//! `Catalog`, `Schema`) MUST emit the byte-exact wire form they used as
//! free-form strings prior to the typed promotion, so external
//! `--json` consumers don't observe a breaking change.

use tome::doctor::Subsystem;

#[test]
fn every_variant_round_trips_via_documented_wire_string() {
    let cases: Vec<(Subsystem, &str)> = vec![
        (Subsystem::Embedder, "\"embedder\""),
        (Subsystem::Reranker, "\"reranker\""),
        (Subsystem::Index, "\"index\""),
        (Subsystem::Drift, "\"drift\""),
        (
            Subsystem::Catalog("upstream".into()),
            "\"catalog:upstream\"",
        ),
        (Subsystem::Schema, "\"schema\""),
        (Subsystem::Summariser, "\"summariser\""),
        (Subsystem::Binding, "\"binding\""),
        (Subsystem::BindingRulesCopy, "\"binding-rules-copy\""),
        (
            Subsystem::HarnessRules("claude-code".into()),
            "\"harness-rules:claude-code\"",
        ),
        (
            Subsystem::HarnessMcp("codex".into()),
            "\"harness-mcp:codex\"",
        ),
        // Phase 13 (native-agent model-registry):
        (Subsystem::ModelRegistry, "\"model-registry\""),
    ];
    for (variant, wire) in cases {
        let serialised = serde_json::to_string(&variant).unwrap();
        assert_eq!(serialised, wire, "wire shape for {variant:?}");
        let parsed: Subsystem = serde_json::from_str(wire).unwrap();
        assert_eq!(parsed, variant, "round-trip from {wire}");
    }
}

#[test]
fn unknown_wire_string_fails_to_deserialise() {
    let err: Result<Subsystem, _> = serde_json::from_str("\"not-a-real-subsystem\"");
    assert!(err.is_err(), "unknown subsystem must reject deserialise");
}

#[test]
fn catalog_subsystem_preserves_name_with_special_chars() {
    // Catalog names are validated upstream but the Subsystem wire shape
    // is just `catalog:<name>` — any characters after the colon belong
    // to the inner String. Test that the round-trip survives a name
    // with dashes and digits (the realistic case).
    let s = Subsystem::Catalog("acme-2025".into());
    let wire = serde_json::to_string(&s).unwrap();
    assert_eq!(wire, "\"catalog:acme-2025\"");
    let parsed: Subsystem = serde_json::from_str(&wire).unwrap();
    assert_eq!(parsed, s);
}

#[test]
fn harness_subsystems_preserve_kebab_case_names() {
    let rules = Subsystem::HarnessRules("claude-code".into());
    assert_eq!(
        serde_json::to_string(&rules).unwrap(),
        "\"harness-rules:claude-code\""
    );
    let mcp = Subsystem::HarnessMcp("opencode".into());
    assert_eq!(
        serde_json::to_string(&mcp).unwrap(),
        "\"harness-mcp:opencode\""
    );
}
