# Spec Quality Checklist — Phase 3

**Spec**: [spec.md](../spec.md)
**Plan**: [plan.md](../plan.md)
**Date**: 2026-05-14

Mechanical pass over the spec to confirm every requirement has at least one acceptance scenario, one functional requirement, and one success criterion. Run on each spec / plan / research update; this is the gating artefact for `/sdd:plan` before `/sdd:tasks` runs.

## User stories → requirements coverage

| User Story | Priority | FR coverage | SC coverage |
|---|---|---|---|
| US1 — MCP-driven harness integration | P1 | FR-101 through FR-113 | SC-101, SC-102, SC-103, SC-111, SC-112, SC-114 |
| US2 — Workspace creation + autodetect | P2 | FR-130 through FR-141 | SC-104, SC-105, SC-106 |
| US3 — Every command honours workspace | P2 | FR-137, FR-138 (overlap with US2); plus FR-133, FR-141, FR-201:workspace_* | SC-104, SC-105, SC-106, SC-113 |
| US4 — Diagnose with `tome doctor` | P3 | FR-160 through FR-168 | SC-107, SC-108 |
| US5 — Forward schema migrations | P3 | FR-180 through FR-184 | SC-109, SC-115 |

**Verdict**: PASS. Every user story has at least three FRs and at least one SC.

## Edge cases → handling check

| Edge case (spec §Edge Cases) | Where handled |
|---|---|
| Workspace inside a workspace | FR-131 (CWD walk first-hit-wins) |
| Workspace marker present but no DB | FR-140; "not yet bootstrapped" in WorkspaceInfo §4 |
| Workspace marker corrupt | FR-140 + exit 70 |
| MCP server with both `--workspace` + `--global` | FR-131 (mutually exclusive) + exit 72 |
| MCP server during CLI writer holding lock | FR-110 (read-only); per-workspace lockfile (R-14) |
| MCP server when workspace deleted mid-session | spec edge case; design = serve from open handle until reconnect fails |
| Two MCP processes, two workspaces | FR-102 + R-14 (independent locks) |
| `search_skills` with empty result set | Tool contract — empty `matches` array, not an error |
| `get_skill` with missing file | mcp-tools.md `skill_file_missing` error code |
| Catalog clone refcount race | catalog-extensions-p3.md §Concurrency |
| `--fix` cannot reach upstream | Doctor reports per-subsystem; other fixes proceed |
| `--fix` interruption | spec edge case + atomic-write discipline (NFR-106) |
| Workspace path with special characters | spec edge case; Tome treats as opaque |
| `--global` + `--workspace` | FR-131 + exit 72 |
| MCP tool description summarization | mcp-tools.md (≤350 chars; tested) |
| Stdout used for non-protocol | FR-221 — invariant; tests/mcp_server.rs enforces |
| Doctor against global from workspace | FR-168 |
| Schema-too-new on workspace, on-version on global | spec edge case; FR-181/FR-182 per-DB |
| Schema migration race | schema-migration.md §Concurrency |

**Verdict**: PASS. Every edge case maps to a documented behaviour or contract.

## NFR coverage

| NFR | Where addressed |
|---|---|
| NFR-101 binary-size cap | plan.md §Operational Constraints; research R-2 measurement |
| NFR-102 dependency justification | plan.md §Operational Constraints table |
| NFR-103 MCP startup < 1 s | spec SC-102; verified by integration test in tests/mcp_lifecycle.rs |
| NFR-104 search_skills latency p50 < 300 ms / p99 < 600 ms | spec SC-103; benchmarked in tests/mcp_server.rs |
| NFR-105 scrubbing | log-format.md §Scrubbing; tests/scrubbing.rs extension |
| NFR-106 atomic writes | data-model.md §State transitions; existing Phase 2 patterns extended |
| NFR-107 quality gates carry forward | plan.md §Operational Constraints |

**Verdict**: PASS.

## Out-of-scope statements

Spec §Out of Scope items each have at least one place that names them as explicit non-targets:

- Cross-harness file installation — plan.md §Summary, plan.md §Constitution Check VI, quickstart.md absence of any "install into harness X" instruction.
- HTTP / SSE transport — mcp-server.md fixes stdio only; FR-101.
- Multi-tenant MCP — FR-102.
- Auth — explicit non-feature, no FR.
- Tool annotations beyond plain text — mcp-tools.md.
- Global ↔ workspace migration tooling — FR-141.
- Harness MCP config validation in doctor — FR-167.
- Plugin authoring tools — N/A for Phase 3.
- Second concrete schema migration — research R-8.
- Query / mutation surface beyond search+fetch in MCP — FR-103.

**Verdict**: PASS.

## Assumption statements

Each spec §Assumptions item is either:
- Inherited from Phase 1/2 (carried forward without restating in plan).
- Restated in plan.md as a constraint (e.g., sync-only outside MCP, no async runtime project-wide).
- Documented in research.md when a research decision depends on it.

**Verdict**: PASS.

## Constitution gates

Per plan.md §Constitution Check: PASS. Two documented deviations in §Complexity Tracking:
1. `tokio` inside `src/mcp/` — anticipated forcing function.
2. Schema-migration framework with zero registered migrations — preventive plumbing.

**Verdict**: PASS.

## Open items

- None blocking. The eight research items (R-1 through R-8) are resolved in research.md. R-9 through R-15 are additional decisions; all resolved.

## Re-run conditions

This checklist should be re-run when:
- spec.md gains a new user story or FR.
- plan.md gains a new dependency or amends the Constitution Check.
- A reviewer flags a coverage gap.
- After `/sdd:tasks` if any task surfaces a missing requirement.

---

**Overall verdict**: PASS. Spec and plan are coherent and ready for `/sdd:tasks`.
