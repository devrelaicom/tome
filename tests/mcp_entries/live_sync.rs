//! Phase 11 / Task 1.4 — the background live-sync watcher.
//!
//! The async `watch`/`watch_turn` loop is not deterministically testable
//! (it is driven by a 60s `tokio` interval), so this exercises the PURE
//! `probe` + `recompute` seam the loop wraps, against a REAL staged
//! workspace and the harness's ACTUAL swappable cells (the same
//! `Arc<RwLock<..>>` the running server reads on every `tools/list`).
//!
//! The asserted behaviour is the description swap: a CLI regenerating a
//! workspace's `[summaries].short` out-of-process changes the composed
//! `search_skills` description, `recompute` detects it, swaps the
//! `desc_cell` in place, and reports `changed.tools` so the watcher emits
//! `tools/list_changed`. This needs only a `settings.toml` write — no skill
//! re-enablement — and is the meaningful seam (the live loop's notify is
//! the only untested wrapper, and it is a one-line `peer.notify_*` call).

use crate::common::mcp_harness::StagedWorkspace;
use tome::mcp::live_sync::{self, DriftSignal};
use tome::mcp::tool_description::SCAFFOLD;
use tome::workspace::WorkspaceName;

const SKILL: &str = "---\nname: alpha\ndescription: A\n---\nBody.\n";

const NEW_BLURB: &str = "focuses on payment integrations and webhook plumbing";

#[test]
fn recompute_updates_tool_description_after_blurb_change() {
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();
    let state = harness.state();
    // `prompt_cell` is passed to recompute so the prompt-rebuild branch is
    // reachable; this test asserts the tool-description swap, not a prompt swap.
    let (prompt_cell, desc_cell) = harness.server().live_sync_cells();

    // Seed the description cell with the startup-composed value, exactly as
    // `mcp::run` does after building the server. The staged workspace has no
    // `[summaries].short` yet, so this is the bare scaffold.
    let startup_desc = tome::mcp::tool_description::compose(state.scope.scope.name(), &state.paths);
    *desc_cell.write().unwrap() = startup_desc.clone();
    assert_eq!(
        startup_desc, SCAFFOLD,
        "test setup: a fresh workspace's description is the bare scaffold",
    );

    // Capture the baseline drift signal BEFORE the out-of-process change.
    let prev = live_sync::probe(&state.scope, &state.paths).expect("probe baseline");
    assert!(
        prev.short_blurb.is_empty(),
        "test setup: no cached short summary yet",
    );

    // Simulate a CLI regenerating the workspace summary: write a new
    // `[summaries].short` into the GLOBAL workspace's settings.toml (the
    // staged workspace is the privileged `global` scope).
    let ws = WorkspaceName::global();
    let settings_path = state.paths.workspace_settings_file(&ws);
    std::fs::create_dir_all(state.paths.workspace_dir(&ws)).unwrap();
    std::fs::write(
        &settings_path,
        format!(
            "name = \"global\"\n[summaries]\nshort = \"{NEW_BLURB}\"\nlong = \"long body\"\n\
             generated_at = \"2026-06-17T00:00:00Z\"\n"
        ),
    )
    .unwrap();

    // Re-probe: the signal now carries the new blurb, so it differs.
    let next = live_sync::probe(&state.scope, &state.paths).expect("probe after write");
    assert_eq!(next.short_blurb, NEW_BLURB, "probe picks up the new blurb");
    assert_ne!(prev, next, "the drift signal changed");

    // Recompute against the harness's REAL cells.
    let changed = live_sync::recompute(&prev, &next, &state, &prompt_cell, &desc_cell);

    // The entry set + freshness did not move (same enabled skill, same
    // indexed_at), so prompts did not drift — only the description did.
    assert!(!changed.prompts, "no entry/freshness drift this turn");
    assert!(changed.tools, "the description drifted, so tools changed");

    // The cell now holds the NEW composed description: scaffold prefix + the
    // imperative routing line wrapping the new blurb.
    let updated = desc_cell.read().unwrap().clone();
    assert_ne!(updated, startup_desc, "the description cell was swapped");
    assert!(
        updated.starts_with(SCAFFOLD),
        "the swapped description retains the scaffold prefix",
    );
    assert!(
        updated.contains(NEW_BLURB),
        "the swapped description embeds the new blurb, got:\n{updated}",
    );
}

#[test]
fn recompute_is_a_noop_when_nothing_drifts() {
    // A turn where the probe is unchanged must report no drift and leave the
    // description cell untouched — the watcher emits no spurious list_changed.
    let staged = StagedWorkspace::stage(&[("alpha", SKILL)], &[]);
    let harness = staged.harness();
    let state = harness.state();
    let (prompt_cell, desc_cell) = harness.server().live_sync_cells();

    let startup_desc = tome::mcp::tool_description::compose(state.scope.scope.name(), &state.paths);
    *desc_cell.write().unwrap() = startup_desc.clone();

    let signal = live_sync::probe(&state.scope, &state.paths).expect("probe");

    // prev == next: no inputs moved since startup.
    let changed = live_sync::recompute(&signal, &signal, &state, &prompt_cell, &desc_cell);
    assert!(!changed.prompts, "no prompt drift");
    assert!(!changed.tools, "no description drift");
    assert_eq!(
        *desc_cell.read().unwrap(),
        startup_desc,
        "the description cell is untouched on a no-op turn",
    );
    // A default baseline used as `prev` here would be a contrived case; the
    // realistic invariant is the startup baseline == current probe.
    let _ = DriftSignal::default();
}
