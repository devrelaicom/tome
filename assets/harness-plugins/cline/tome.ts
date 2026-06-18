// Tome session-steering shim for Cline — PLACEHOLDER (Phase 11 T003).
//
// The real implementation lands in US3 (T054): a `registerMessageBuilder`
// shim that shells out to `tome harness session-start --harness cline` and
// injects its stdout into the session, no-opping when the `tome` binary is
// absent. See ../README.md for the shim contract.
//
// This file exists so `build.rs` has a stable entrypoint to embed; it is
// intentionally inert until T054.
export default {};
