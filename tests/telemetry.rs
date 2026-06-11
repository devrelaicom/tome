//! Consolidated integration-test binary for the `telemetry` surface (Phase 10).
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/telemetry/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test telemetry` runs the
//! group. More members get added by later slices.

mod common;

// Per-command anonymous emits: real-binary catalog/workspace/doctor paths
// (cross-platform) + an in-process, Unix-only CLI `tome.search` section.
#[path = "telemetry/command_emits.rs"]
mod command_emits;
// US3 part 2: the `telemetry flush` foreground drain (exit 90 vs --quiet 0) and
// the exit-hook spawn-decision helpers (throttle + threshold).
#[path = "telemetry/delivery.rs"]
mod delivery;
#[path = "telemetry/events.rs"]
mod events;
#[path = "telemetry/exit_codes.rs"]
mod exit_codes;
#[path = "telemetry/identity.rs"]
mod identity;
#[path = "telemetry/inspect.rs"]
mod inspect;
// The MCP-funnel suite stages a catalog via a symlinked cache dir (the standard
// in-process MCP staging shape), so it is Unix-only like its `mcp_entries` peers.
#[cfg(unix)]
#[path = "telemetry/mcp_funnel.rs"]
mod mcp_funnel;
#[path = "telemetry/queue_behavior.rs"]
mod queue_behavior;
// Process-start lifecycle: cli_startup install/upgrade/disabled (cross-platform)
// + a Unix-only MCP cold-start silent-mint section.
#[path = "telemetry/startup.rs"]
mod startup;
// US3 (T054): the delivery DRAIN through the public flush seams — SC-002/008/009/
// 010 + FR-040..046, the `?stream=` split, endpoint scrub, and the falsifiable
// network-counter increment.
#[path = "telemetry/transport.rs"]
mod transport;
