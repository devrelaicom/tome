//! Phase 6 / US4 — the `expose_agents_as_personas` setting (settings-p6.md
//! § Tests).
//!
//! Pins the Phase 6 scalar's parse + layering behaviour:
//! - Defaults to `false` when the key is absent at every scope.
//! - The strict (`deny_unknown_fields`) struct still rejects an unknown
//!   key (NFR-010 — Phase 6 does not loosen the strictness boundary).
//! - First-declarer-wins priority walk (project → workspace → global): a
//!   project `false` overrides a global `true`.
//! - Fall-through to global when project + workspace leave the key absent.
//!
//! US5's `strip_plugin_agent_privileges` is NOT exercised here — that field
//! does not exist yet (it lands with US5). Only `expose_agents_as_personas`
//! is in scope for US4.

use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings, resolve_scalar_with};
use tome::workspace::WorkspaceName;

// ---------------------------------------------------------------------------
// Constructors (the three structs carry several non-Phase-6 fields).
// ---------------------------------------------------------------------------

fn global(expose: Option<bool>) -> GlobalSettings {
    GlobalSettings {
        harnesses: None,
        expose_agents_as_personas: expose,
    }
}

fn workspace(name: &str, expose: Option<bool>) -> WorkspaceSettings {
    WorkspaceSettings {
        name: WorkspaceName::parse(name).expect("workspace name parses"),
        summaries: None,
        catalogs: Vec::new(),
        harnesses: None,
        expose_agents_as_personas: expose,
    }
}

fn project(name: &str, expose: Option<bool>) -> ProjectMarkerConfig {
    ProjectMarkerConfig {
        workspace: WorkspaceName::parse(name).expect("workspace name parses"),
        harnesses: None,
        expose_agents_as_personas: expose,
    }
}

/// Resolve the `expose_agents_as_personas` scalar across the three scopes
/// using the production closure resolver — the same accessor wiring a
/// production call site uses.
fn resolve(
    proj: Option<&ProjectMarkerConfig>,
    ws: Option<&WorkspaceSettings>,
    glob: &GlobalSettings,
) -> bool {
    resolve_scalar_with(
        proj,
        ws,
        glob,
        |p| p.expose_agents_as_personas,
        |w| w.expose_agents_as_personas,
        |g| g.expose_agents_as_personas,
    )
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn defaults_false_when_absent() {
    // A global settings file omitting the key parses with the field as
    // `None`, and the resolver returns `false` when no scope declares it.
    let parsed: GlobalSettings =
        toml::from_str("").expect("empty global settings parses (all fields optional)");
    assert_eq!(
        parsed.expose_agents_as_personas, None,
        "absent key parses to None (fall-through marker)",
    );

    let resolved = resolve(None, None, &parsed);
    assert!(!resolved, "default is false when nowhere declared",);
}

#[test]
fn deny_unknown_fields_preserved() {
    // The strict struct rejects an unknown key — Phase 6 leaves the
    // strictness boundary in place (NFR-010). A typo / unsupported key is a
    // hard parse error, not a silent ignore.
    let err = toml::from_str::<GlobalSettings>("expose_agents_as_persona = true\n")
        .expect_err("unknown key must be rejected by deny_unknown_fields");
    let msg = err.to_string();
    assert!(
        msg.contains("unknown field") || msg.contains("expose_agents_as_persona"),
        "error names the offending unknown key; got: {msg}",
    );

    // Sanity: the correctly-spelled key parses fine.
    let ok: GlobalSettings =
        toml::from_str("expose_agents_as_personas = true\n").expect("correct key parses");
    assert_eq!(ok.expose_agents_as_personas, Some(true));
}

#[test]
fn project_false_overrides_global_true() {
    // First-declarer-wins: project declares `false`, global declares
    // `true`. The nearest declarer (project) wins → resolves false. This is
    // the defining behaviour of the scalar layering (vs the harnesses list,
    // which would union).
    let proj = project("global", Some(false));
    let glob = global(Some(true));
    assert!(
        !resolve(Some(&proj), None, &glob),
        "project false overrides global true",
    );

    // And the symmetric case: project true over global false → true.
    let proj_true = project("global", Some(true));
    let glob_false = global(Some(false));
    assert!(
        resolve(Some(&proj_true), None, &glob_false),
        "project true overrides global false",
    );
}

#[test]
fn falls_through_to_global() {
    // Project + workspace leave the key absent (`None`); global declares
    // `true`. The walk falls through to global's declared value.
    let ws = workspace("ws", None);
    let glob = global(Some(true));
    assert!(
        resolve(None, Some(&ws), &glob),
        "absent project + workspace → global's declared value wins",
    );

    // Symmetric: global declares false → resolves false (still a decision,
    // not the implicit default).
    let glob_false = global(Some(false));
    assert!(
        !resolve(None, Some(&ws), &glob_false),
        "global's declared false is honoured on fall-through",
    );
}
