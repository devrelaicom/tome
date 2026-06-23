//! F8 — layered settings parser + composition resolver skeleton tests.
//!
//! Covers:
//! - `CompositionRef::parse` parse ladder per FR-443 / research §R-9
//! - `parse_workspace` / `parse_project_marker` round-trips
//! - `HarnessConfig` (global layer) round-trips via `toml::from_str`
//! - `#[serde(deny_unknown_fields)]` rejection on every settings layer
//!
//! The resolver itself is exercised by the in-module unit tests in
//! `src/settings/resolver.rs` and (in US3) by a dedicated `tests/settings_*.rs`
//! suite per `contracts/settings-composition.md` §Test coverage.
//!
//! Note: `parse_global` was removed in Task 2 / fix-4. The global layer
//! now lives in `config.toml [harness]` and is loaded via `crate::config::load`.
//! Tests that previously called `parse_global` now parse `HarnessConfig`
//! directly via `toml::from_str`.

use tome::config::HarnessConfig;
use tome::error::CompositionErrorKind;
use tome::settings::{CachedSummaries, CompositionRef, ProjectMarkerConfig, parser};
use tome::workspace::WorkspaceName;

// ---------------------------------------------------------------------------
// Composition reference parse ladder
// ---------------------------------------------------------------------------

#[test]
fn composition_ref_parses_current_workspace() {
    let parsed = CompositionRef::parse("[workspace]").unwrap();
    assert_eq!(parsed, CompositionRef::CurrentWorkspace);
}

#[test]
fn composition_ref_parses_global() {
    let parsed = CompositionRef::parse("[global]").unwrap();
    assert_eq!(parsed, CompositionRef::Global);
}

#[test]
fn composition_ref_parses_named_workspace() {
    let parsed = CompositionRef::parse("[workspaces.foo]").unwrap();
    assert_eq!(
        parsed,
        CompositionRef::NamedWorkspace(WorkspaceName::parse("foo").unwrap())
    );
}

#[test]
fn composition_ref_parses_exclusion() {
    let parsed = CompositionRef::parse("!bar").unwrap();
    assert_eq!(parsed, CompositionRef::Exclude("bar".to_owned()));
}

#[test]
fn composition_ref_parses_plain_inclusion() {
    let parsed = CompositionRef::parse("claude-code").unwrap();
    assert_eq!(parsed, CompositionRef::Include("claude-code".to_owned()));
}

#[test]
fn composition_ref_rejects_bracketed_exclusion_global() {
    // FR-448: `![global]` is not a defined operation — exclusions
    // describe individual harnesses, not whole scopes.
    let err = CompositionRef::parse("![global]").expect_err("must reject `![global]`");
    match err {
        CompositionErrorKind::BadExclusion(token) => assert_eq!(token, "![global]"),
        other => panic!("expected BadExclusion, got {other:?}"),
    }
}

#[test]
fn composition_ref_rejects_bracketed_exclusion_workspace() {
    let err = CompositionRef::parse("![workspace]").expect_err("must reject `![workspace]`");
    assert!(matches!(err, CompositionErrorKind::BadExclusion(_)));
}

#[test]
fn composition_ref_rejects_bracketed_exclusion_named_workspace() {
    let err =
        CompositionRef::parse("![workspaces.foo]").expect_err("must reject `![workspaces.foo]`");
    assert!(matches!(err, CompositionErrorKind::BadExclusion(_)));
}

#[test]
fn composition_ref_validates_named_workspace_inner_name() {
    // F10: the named-workspace inner string is validated by
    // `WorkspaceName::parse` at the parse boundary. An invalid name
    // surfaces as `BadExclusion` (collapsing onto exit 17) — we don't
    // thread `TomeError` through every composition surface for this one
    // failure mode.
    let err =
        CompositionRef::parse("[workspaces.bad-name-]").expect_err("must reject hyphen-suffix");
    match err {
        CompositionErrorKind::BadExclusion(msg) => {
            assert!(msg.contains("bad-name-"), "{msg}");
            assert!(msg.contains("ends with"), "{msg}");
        }
        other => panic!("expected BadExclusion, got {other:?}"),
    }
}

#[test]
fn composition_ref_accepts_valid_named_workspace_inner_name() {
    let parsed = CompositionRef::parse("[workspaces.foo-bar]").unwrap();
    match parsed {
        CompositionRef::NamedWorkspace(name) => assert_eq!(name.as_str(), "foo-bar"),
        other => panic!("expected NamedWorkspace, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Settings file parsing — workspace / project marker / global
// ---------------------------------------------------------------------------

#[test]
fn parse_workspace_minimal_round_trip() {
    let toml = r#"
name = "global"
"#;
    let parsed = parser::parse_workspace(toml).unwrap();
    assert_eq!(parsed.name, WorkspaceName::global());
    assert!(parsed.summaries.is_none());
    assert!(parsed.catalogs.is_empty());
    assert!(parsed.harnesses.is_none());
}

#[test]
fn parse_workspace_full_round_trip() {
    // TOML scoping: top-level keys MUST appear before any `[table]` or
    // `[[array-of-tables]]` header, else they're interpreted as fields
    // of the last header. Put `name` + `harnesses` first, then nested
    // tables.
    let toml = r#"
name = "my-project"
harnesses = ["claude-code", "[global]", "!cursor"]

[summaries]
short = "short summary"
long = "long summary"
generated_at = 2026-05-14T15:00:00Z

[[catalogs]]
name = "midnight-expert"
url  = "https://github.com/devrelaicom/midnight-expert"
ref  = "main"
"#;
    let parsed = parser::parse_workspace(toml).unwrap();
    assert_eq!(parsed.name.as_str(), "my-project");

    let summaries: CachedSummaries = parsed.summaries.expect("summaries declared");
    assert_eq!(summaries.short, "short summary");
    assert_eq!(summaries.long, "long summary");

    assert_eq!(parsed.catalogs.len(), 1);
    assert_eq!(parsed.catalogs[0].name, "midnight-expert");
    assert_eq!(parsed.catalogs[0].r#ref, "main");

    let harnesses = parsed.harnesses.expect("declared");
    assert_eq!(
        harnesses,
        vec![
            "claude-code".to_owned(),
            "[global]".to_owned(),
            "!cursor".to_owned()
        ]
    );
}

#[test]
fn parse_workspace_rejects_unknown_field() {
    let toml = r#"
name = "global"
spurious_field = "bad"
"#;
    let err = parser::parse_workspace(toml).expect_err("must reject unknown field");
    let rendered = err.to_string();
    assert!(
        rendered.contains("spurious_field") || rendered.contains("unknown field"),
        "error must surface unknown field: {rendered}"
    );
}

#[test]
fn parse_project_marker_minimal_round_trip() {
    let toml = r#"
workspace = "my-project"
"#;
    let parsed = parser::parse_project_marker(toml).unwrap();
    assert_eq!(parsed.workspace.as_str(), "my-project");
    assert!(parsed.harnesses.is_none());
}

#[test]
fn parse_project_marker_with_harness_composition() {
    let toml = r#"
workspace = "my-project"
harnesses = ["[workspace]", "!cursor", "claude-code"]
"#;
    let parsed: ProjectMarkerConfig = parser::parse_project_marker(toml).unwrap();
    let harnesses = parsed.harnesses.expect("declared");
    assert_eq!(harnesses.len(), 3);
    assert_eq!(harnesses[0], "[workspace]");
}

#[test]
fn parse_project_marker_rejects_unknown_field() {
    let toml = r#"
workspace = "x"
nope = true
"#;
    let err = parser::parse_project_marker(toml).expect_err("must reject unknown field");
    assert!(err.to_string().contains("nope") || err.to_string().contains("unknown field"));
}

// ---------------------------------------------------------------------------
// Global harness layer (Task 2 / fix-4): `parse_global` is gone.
// The global layer now lives in `config.toml [harness]` and is parsed as
// `HarnessConfig` directly via `toml::from_str`.
// ---------------------------------------------------------------------------

#[test]
fn global_harness_config_empty_is_default() {
    // An empty [harness] section → all fields are None / default.
    let parsed: HarnessConfig = toml::from_str("").unwrap();
    assert_eq!(parsed, HarnessConfig::default());
}

#[test]
fn global_harness_config_with_enabled() {
    // The flat `enabled = [...]` shape under `[harness]` in config.toml.
    let toml = r#"
enabled = ["claude-code", "codex"]
"#;
    let parsed: HarnessConfig = toml::from_str(toml).unwrap();
    let harnesses = parsed.enabled.expect("declared");
    assert_eq!(
        harnesses,
        vec!["claude-code".to_owned(), "codex".to_owned()]
    );
}

#[test]
fn global_harness_config_rejects_unknown_field() {
    let toml = r#"
enabled = []
mystery = 42
"#;
    let err = toml::from_str::<HarnessConfig>(toml).expect_err("must reject unknown field");
    let rendered = err.to_string();
    assert!(
        rendered.contains("mystery") || rendered.contains("unknown field"),
        "error must surface unknown field: {rendered}"
    );
}
