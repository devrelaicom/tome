// Tome session-steering shim for OpenCode — PLACEHOLDER (Phase 11 T003).
//
// The real implementation lands in US3 (T056): an
// `experimental.chat.system.transform` shim that shells out to
// `tome harness session-start --harness opencode` and injects its stdout,
// no-opping when the `tome` binary is absent, with zero npm imports. See
// ../README.md for the shim contract.
//
// This file exists so `build.rs` has a stable entrypoint to embed; it is
// intentionally inert until T056.
export default {};
