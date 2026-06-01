# Contract: In-Process MCP Test Harness (FR-012)

**FR**: FR-012 · **SC**: SC-010 · **Research**: §R-11 · **Backlog**: `CONCERNS.md` GAP-1

Give the MCP surface — the integration story external users exercise first — real end-to-end exit-code coverage, and verify the FR-004 prompt-collision fix end-to-end. Closes the pre-named `GAP-1` backlog item.

## The harness

A test-side driver (in `tests/common/`) that constructs and drives a **real MCP server instance in-process via the library API** (no real model load — `StubEmbedder`), able to issue `initialize`, `prompts/list`, `prompts/get`, and tool calls and observe the resulting outcome/exit code.

- **Library side, not subprocess** (§R-11): the MCP-internal codes are reachable in-process; an in-process driver is lighter and faster than piping JSON-RPC over stdio, and matches the established library-vs-CLI-binary test boundary (Phase 4 P3) — the CLI binary cannot reach these codes because the MCP server is the surface.
- Built on the established `#[doc(hidden)] pub static` override-slot + RAII-guard seam pattern; reuses `EnvVarGuard`/`ENV_MUTEX` (promote it to `tests/common/mod.rs` if this is the 5th consumer). Reuses the existing `tests/mcp_server.rs` / `mcp_lifecycle.rs` / `mcp_prompts.rs` infrastructure.

## Coverage obligations

**`tests/exit_codes_e2e_mcp.rs`** exercises the MCP-internal exit codes that previously lacked end-to-end CLI coverage (GAP-1):

| Code | Variant | Driven via |
|---|---|---|
| **9** | `PluginDataDirWriteFailed` | a prompt/tool path that triggers a data-dir write failure |
| **26** | `PromptArgumentMismatch` | `prompts/get` with mismatched arguments |
| **27** | `EntryNotFound` | `prompts/get`/tool for a missing entry |
| **28** | `SubstitutionFailed` | a prompt whose substitution fails |
| **29** | `InvalidArgumentFrontmatter` | an entry with malformed argument frontmatter |

**FR-004 verification** (the gate FR-012 owns): drive the Command `foo` + user-invocable Skill `foo` + Command `foo2` fixture through `prompts/list` + `prompts/get`; assert **all three entries are present and resolvable** (no `prompt_not_found`), proving the K4 single-global-taken-set fix end-to-end.

## Sequencing

- FR-004's **fix** (K4) may land first; FR-012 gates its **verification**, not its implementation (spec Dependencies).
- The harness is built in the US3 test-foundation slice (T1), after the correctness fixes land.

## Test obligations / SC

- SC-010: the previously-uncovered MCP exit codes are exercised against a real server instance (coverage gap closed).
- The harness itself stays on the **sync boundary** rules — it drives the existing single-threaded `src/mcp/` island; `tests/sync_boundary.rs` stays green.

## Anti-requirements

- MUST NOT load real ONNX models (use `StubEmbedder`) — keeps the suite fast and CI-safe.
- MUST NOT introduce a second async runtime or violate the `src/mcp/`-only async boundary.
- MUST NOT duplicate the existing MCP test helpers — extend/reuse them.
