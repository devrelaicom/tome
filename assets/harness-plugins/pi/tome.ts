// Tome session-steering shim for Pi — PLACEHOLDER (Phase 11 T003).
//
// The real implementation lands in US3 (T055): a `before_agent_start` shim
// that shells out to `tome harness session-start --harness pi` and returns
// its stdout, no-opping when the `tome` binary is absent. See ../README.md
// for the shim contract.
//
// This file exists so `build.rs` has a stable entrypoint to embed; it is
// intentionally inert until T055.
export default {};
