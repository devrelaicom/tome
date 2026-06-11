//! Consolidated integration-test binary for the `telemetry` surface (Phase 10).
//!
//! The former top-level `tests/<name>.rs` files are submodules under
//! `tests/telemetry/`, sharing ONE compiled + linked binary instead of N. This
//! collapses the per-file static-link + process-spawn overhead to one per
//! group. Test names gain a `<name>::` module prefix, so `cargo test
//! <name>::` still filters by file and `cargo test --test telemetry` runs the
//! group. More members get added by later slices.

mod common;

#[path = "telemetry/exit_codes.rs"]
mod exit_codes;
#[path = "telemetry/identity.rs"]
mod identity;
