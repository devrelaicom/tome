//! Consolidated integration-test binary for the `telemetry` surface, re-homed
//! onto the `gauge-telemetry` kernel.
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/telemetry/`, sharing ONE compiled + linked binary instead of N. Test
//! names gain a `<name>::` module prefix, so `cargo test <name>::` still filters
//! by file and `cargo test --test telemetry` runs the group.
//!
//! After the kernel migration the queue + envelope + delivery are kernel-owned;
//! the retired-internal suites (queue mechanics, transport POST, flush drain, the
//! Envelope wire pins, the `TELEMETRY.md` byte-pin) were dropped — the kernel has
//! its own tests, and the privacy backstop is now the `canary` + `consent`
//! suites here plus the `src/telemetry/event.rs` round-trip tests. Tome RETAINS
//! the allowlist/attribution gate, install-method/upgrade detection, the
//! heartbeat, the `should_spawn` throttle, the `tome telemetry` CLI, and the
//! doctor report — those are what these adapted suites assert.

mod common;

/// Shared kernel-queue + env helpers for the adapted suites.
#[path = "telemetry/queue_util.rs"]
mod queue_util;

// The catalog-attributed stream's integration acceptance guarantees: both-streams
// -one-drain, source-is-the-gate name collision (FR-052), exact `rank` (FR-057),
// emit-time `const` de-allowlist (FR-053) — against the kernel queue.
#[path = "telemetry/attributed.rs"]
mod attributed;
// Per-command anonymous emits: real-binary catalog/workspace/doctor paths +
// an in-process, Unix-only CLI `tome.search` section.
#[path = "telemetry/command_emits.rs"]
mod command_emits;
// PRIVACY CANARY: probe the Tier-2 bounded strings + forbid paths/query/creds.
#[path = "telemetry/canary.rs"]
mod canary;
// CONSENT MATRIX: CI auto-off, opt-out, disabled=no-op, endpoint validation.
#[path = "telemetry/consent.rs"]
mod consent;
// Telemetry CLI exit codes + byte-stable `status --json` pins (real binary).
#[path = "telemetry/exit_codes.rs"]
mod exit_codes;
// End-to-end identity + consent behaviour via the real binary CLI surface.
#[path = "telemetry/identity.rs"]
mod identity;
// `tome telemetry inspect` — read-only queue pretty-print + exit 92.
#[path = "telemetry/inspect.rs"]
mod inspect;
// The MCP-funnel suite stages a catalog via a symlinked cache dir, so it is
// Unix-only like its `mcp_entries` peers.
#[cfg(unix)]
#[path = "telemetry/mcp_funnel.rs"]
mod mcp_funnel;
// Process-start lifecycle: cli_startup install/upgrade/disabled via the real
// binary + a Unix-only MCP cold-start silent-mint section.
#[path = "telemetry/startup.rs"]
mod startup;
// The read-only `tome doctor` telemetry section, end-to-end through the real
// binary (read-only/no-mint + `--fix` no-op).
#[path = "telemetry/telemetry_doctor.rs"]
mod telemetry_doctor;
