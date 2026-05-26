//! Phase 5 / US1.a — widened lenient frontmatter coverage.
//!
//! Exercises the Phase 5 fields documented in
//! `specs/005-phase-5-commands-prompts/contracts/frontmatter-p5.md`
//! against `tome::plugin::frontmatter::parse_skill_frontmatter_str`.

use std::path::PathBuf;

use tome::plugin::frontmatter::{parse_skill_frontmatter_str, validate_argument_names};
use tome::plugin::identity::EntryKind;

fn parse(src: &str) -> tome::plugin::frontmatter::ParsedSkill {
    parse_skill_frontmatter_str(&PathBuf::from("/tmp/SKILL.md"), src)
        .expect("frontmatter must parse")
}

#[test]
fn arguments_string_or_list_both_parse() {
    let as_string = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments: component from to\n\
         ---\n\
         body\n",
    );
    let as_list = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments:\n  - component\n  - from\n  - to\n\
         ---\n\
         body\n",
    );

    assert_eq!(
        as_string.frontmatter.arguments,
        vec!["component", "from", "to"],
    );
    assert_eq!(
        as_list.frontmatter.arguments,
        vec!["component", "from", "to"],
    );
}

#[test]
fn unknown_field_does_not_fail() {
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         allowed-tools: [Read, Write]\n\
         agent: claude\n\
         arbitrary_key: some value\n\
         ---\n\
         body\n",
    );
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("foo"));
}

#[test]
fn malformed_arguments_field_fails_loudly() {
    // An integer is neither a string nor a YAML list — the custom
    // deserialiser must refuse.
    let err = parse_skill_frontmatter_str(
        &PathBuf::from("/tmp/SKILL.md"),
        "---\n\
         name: foo\n\
         description: d\n\
         arguments: 42\n\
         ---\n",
    )
    .expect_err("integer arguments must fail to parse");
    match err {
        tome::plugin::frontmatter::FrontmatterError::InvalidYaml { .. } => {}
        other => panic!("expected InvalidYaml, got {other:?}"),
    }
}

#[test]
fn illegal_argument_name_fails() {
    // Pre-deserialiser leniency: parsing succeeds, but the
    // validate_argument_names helper refuses.
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments: 1foo bar\n\
         ---\n\
         body\n",
    );
    let err = validate_argument_names(&parsed.frontmatter.arguments)
        .expect_err("`1foo` must be rejected");
    assert!(
        err.contains("1foo"),
        "error must name the offending value: {err}"
    );
}

#[test]
fn default_user_invocable_per_kind() {
    let parsed = parse("---\nname: foo\ndescription: d\n---\n");
    assert!(
        !parsed.frontmatter.resolved_user_invocable(EntryKind::Skill),
        "skills default to user_invocable=false",
    );
    assert!(
        parsed
            .frontmatter
            .resolved_user_invocable(EntryKind::Command),
        "commands default to user_invocable=true",
    );
}

#[test]
fn explicit_user_invocable_overrides_default() {
    let force_true = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         user-invocable: true\n\
         ---\n",
    );
    assert!(
        force_true
            .frontmatter
            .resolved_user_invocable(EntryKind::Skill),
        "explicit `user-invocable: true` must flip a skill",
    );

    let force_false = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         user-invocable: false\n\
         ---\n",
    );
    assert!(
        !force_false
            .frontmatter
            .resolved_user_invocable(EntryKind::Command),
        "explicit `user-invocable: false` must flip a command",
    );
}

#[test]
fn description_fallback_to_body_prefix() {
    // No `description` field → falls back to the first 500 chars of the
    // body (and reports the fallback bit).
    let parsed = parse(
        "---\n\
         name: foo\n\
         ---\n\
         this is the body text\n",
    );
    let (desc, fallback) = parsed.resolved_description();
    assert!(fallback, "fallback bit must report TRUE");
    assert!(desc.contains("this is the body text"));
}

#[test]
fn backwards_compat_phase4_only_frontmatter() {
    // A skill file produced before Phase 5 carries only `name` +
    // `description`. The widened deserialiser MUST accept this verbatim.
    let parsed = parse(
        "---\n\
         name: legacy\n\
         description: I shipped with Phase 4\n\
         ---\n\
         body\n",
    );
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("legacy"));
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("I shipped with Phase 4"),
    );
    assert!(parsed.frontmatter.arguments.is_empty());
    assert!(parsed.frontmatter.when_to_use.is_none());
    assert!(parsed.frontmatter.disable_model_invocation.is_none());
    assert!(parsed.frontmatter.user_invocable.is_none());
    assert!(
        parsed.frontmatter.resolved_searchable(),
        "searchable defaults TRUE",
    );
}

#[test]
fn when_to_use_round_trips() {
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         when_to_use: \"When the user asks about widgets.\"\n\
         ---\n\
         body\n",
    );
    assert_eq!(
        parsed.frontmatter.when_to_use.as_deref(),
        Some("When the user asks about widgets."),
    );
}

#[test]
fn disable_model_invocation_flips_resolved_searchable() {
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         disable-model-invocation: true\n\
         ---\n",
    );
    assert!(
        !parsed.frontmatter.resolved_searchable(),
        "`disable-model-invocation: true` must flip resolved_searchable to false",
    );
}
