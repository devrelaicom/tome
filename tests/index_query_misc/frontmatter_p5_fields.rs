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
        as_string.frontmatter.argument_names(),
        vec!["component", "from", "to"],
    );
    assert_eq!(
        as_list.frontmatter.argument_names(),
        vec!["component", "from", "to"],
    );
    // String / bare-list forms carry no per-argument description.
    assert!(
        as_list
            .frontmatter
            .arguments
            .iter()
            .all(|a| a.description.is_none()),
        "bare-list arguments must have no description",
    );
}

#[test]
fn arguments_object_form_carries_description() {
    // Issue #312: a `{ name, description }` mapping entry threads the
    // description through while keeping the name.
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments:\n\
        \x20 - name: issue_url\n\
        \x20   description: the GitHub issue URL, e.g. https://github.com/org/repo/issues/1\n\
         ---\n\
         body\n",
    );
    assert_eq!(parsed.frontmatter.argument_names(), vec!["issue_url"]);
    assert_eq!(
        parsed.frontmatter.arguments[0].description.as_deref(),
        Some("the GitHub issue URL, e.g. https://github.com/org/repo/issues/1"),
    );
}

#[test]
fn arguments_mixed_string_and_object_forms() {
    // Issue #312: a single list may freely mix bare-name strings and
    // `{ name, description }` mappings.
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments:\n\
        \x20 - plain\n\
        \x20 - name: described\n\
        \x20   description: has a hint\n\
        \x20 - also_plain\n\
         ---\n\
         body\n",
    );
    assert_eq!(
        parsed.frontmatter.argument_names(),
        vec!["plain", "described", "also_plain"],
    );
    assert!(parsed.frontmatter.arguments[0].description.is_none());
    assert_eq!(
        parsed.frontmatter.arguments[1].description.as_deref(),
        Some("has a hint"),
    );
    assert!(parsed.frontmatter.arguments[2].description.is_none());
}

#[test]
fn arguments_object_unknown_key_is_tolerated() {
    // Third-party frontmatter parses leniently: an unrecognised key inside
    // the mapping is dropped, not rejected.
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments:\n\
        \x20 - name: issue_url\n\
        \x20   description: a hint\n\
        \x20   required: true\n\
        \x20   whatever: ignored\n\
         ---\n\
         body\n",
    );
    assert_eq!(parsed.frontmatter.argument_names(), vec!["issue_url"]);
    assert_eq!(
        parsed.frontmatter.arguments[0].description.as_deref(),
        Some("a hint"),
    );
}

#[test]
fn arguments_object_missing_name_is_skipped() {
    // A mapping with no `name` is malformed; it degrades to being skipped
    // rather than aborting the whole list (lenient third-party parse).
    let parsed = parse(
        "---\n\
         name: foo\n\
         description: d\n\
         arguments:\n\
        \x20 - name: good\n\
        \x20 - description: orphan hint, no name\n\
         ---\n\
         body\n",
    );
    assert_eq!(parsed.frontmatter.argument_names(), vec!["good"]);
}

#[test]
fn arguments_object_form_capped_at_256() {
    // The 256-entry cap applies uniformly to the object form too.
    let mut src = String::from("---\nname: foo\ndescription: d\narguments:\n");
    for i in 0..300 {
        use std::fmt::Write;
        writeln!(src, "  - name: arg{i}\n    description: d{i}").unwrap();
    }
    src.push_str("---\nbody\n");

    let err = parse_skill_frontmatter_str(&PathBuf::from("/tmp/SKILL.md"), &src)
        .expect_err("oversized object-form arguments list must reject");
    assert!(
        err.to_string().contains("256"),
        "rejection message must cite the 256-entry cap",
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
    let err = validate_argument_names(&parsed.frontmatter.argument_names())
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

// ---- S-M1 (US1.d reviewer pass): arguments cap at 256 ------------------

#[test]
fn arguments_list_capped_at_256() {
    // 300-element YAML sequence: must reject with a parser error citing
    // the cap. Built as a single multiline string the parser will see.
    let mut src = String::from("---\nname: foo\ndescription: d\narguments:\n");
    for i in 0..300 {
        use std::fmt::Write;
        writeln!(src, "  - arg{i}").unwrap();
    }
    src.push_str("---\nbody\n");

    let err = parse_skill_frontmatter_str(&PathBuf::from("/tmp/SKILL.md"), &src)
        .expect_err("oversized arguments list must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("256"),
        "rejection message must cite the 256-entry cap; got: {msg}",
    );
}

#[test]
fn arguments_string_capped_at_256_tokens() {
    // 300 space-separated tokens: same cap applies to the string form.
    let tokens: String = (0..300)
        .map(|i| format!("arg{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("---\nname: foo\ndescription: d\narguments: {tokens}\n---\nbody\n");

    let err = parse_skill_frontmatter_str(&PathBuf::from("/tmp/SKILL.md"), &src)
        .expect_err("oversized space-separated arguments must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("256"),
        "rejection message must cite the 256-entry cap; got: {msg}",
    );
}

#[test]
fn arguments_at_cap_still_parses() {
    // Exactly 256 entries: must pass cleanly. Defends against off-by-one.
    let mut src = String::from("---\nname: foo\ndescription: d\narguments:\n");
    for i in 0..256 {
        use std::fmt::Write;
        writeln!(src, "  - arg{i}").unwrap();
    }
    src.push_str("---\nbody\n");

    let parsed = parse_skill_frontmatter_str(&PathBuf::from("/tmp/SKILL.md"), &src)
        .expect("256 entries is at the cap and must parse");
    assert_eq!(parsed.frontmatter.arguments.len(), 256);
}

// ---- Issue #312: ArgumentSpec serialisation round-trip ------------------

#[test]
fn argument_spec_without_description_serialises_as_bare_string() {
    // Back-compat: a name-only spec must serialise byte-identically to the
    // legacy `Vec<String>` form (a bare YAML scalar), so `convert`'s
    // round-trip and every wire pin stays stable.
    use tome::plugin::frontmatter::ArgumentSpec;
    let specs = vec![
        ArgumentSpec {
            name: "one".to_owned(),
            description: None,
        },
        ArgumentSpec {
            name: "two".to_owned(),
            description: None,
        },
    ];
    let yaml = serde_yaml::to_string(&specs).unwrap();
    // Legacy `Vec<String>` renders identically.
    let legacy = serde_yaml::to_string(&vec!["one", "two"]).unwrap();
    assert_eq!(yaml, legacy, "name-only specs must match the legacy form");
}

#[test]
fn argument_spec_with_description_serialises_as_mapping() {
    use tome::plugin::frontmatter::ArgumentSpec;
    let specs = vec![ArgumentSpec {
        name: "issue_url".to_owned(),
        description: Some("the issue URL".to_owned()),
    }];
    let yaml = serde_yaml::to_string(&specs).unwrap();
    assert!(yaml.contains("name: issue_url"), "got: {yaml}");
    assert!(yaml.contains("description: the issue URL"), "got: {yaml}");
}

#[test]
fn argument_spec_round_trips_through_parse() {
    // Emit a mixed list, re-parse it, and confirm names + descriptions
    // survive intact.
    use tome::plugin::frontmatter::ArgumentSpec;
    let specs = vec![
        ArgumentSpec {
            name: "plain".to_owned(),
            description: None,
        },
        ArgumentSpec {
            name: "described".to_owned(),
            description: Some("a hint".to_owned()),
        },
    ];
    let list_yaml = serde_yaml::to_string(&specs).unwrap();
    let src = format!("---\nname: foo\ndescription: d\narguments:\n{list_yaml}---\nbody\n");
    let parsed = parse(&src);
    assert_eq!(
        parsed.frontmatter.argument_names(),
        vec!["plain", "described"]
    );
    assert!(parsed.frontmatter.arguments[0].description.is_none());
    assert_eq!(
        parsed.frontmatter.arguments[1].description.as_deref(),
        Some("a hint"),
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
