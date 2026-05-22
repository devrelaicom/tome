# Specification Quality Checklist: Phase 4 — Central Architecture Refactor and Cross-Harness Integration

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-05-14
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
  - Notes: Tome-specific domain entities (catalog, plugin, workspace, embedder, reranker, MCP) appear because they are the *features* this product manages, not implementation choices. The spec deliberately names two third-party libraries by category — an order-preserving JSON map (FR-349 mentions `serde_json` with `preserve_order`) and a comment-preserving TOML editor (`toml_edit`) — because the *behavioural correctness* of preserving developer-authored content in harness configs depends on the library category, and stating it now prevents the implementer from silently picking a destructive round-trip path. Specific framework versions and modules are left to the plan.
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
  - Notes: Some FRs reference concepts (advisory lockfile, SQLite WAL, atomic-rename-on-the-same-filesystem) that a non-technical reader will not parse without the codebase context. This is consistent with the Phase 3 spec's voice — the Tome project's stakeholders include the maintainer team, whose comprehension floor is Rust-and-systems-aware.
- [x] All mandatory sections completed
  - User Scenarios & Testing: 5 user stories with priorities + acceptance scenarios + edge cases. ✓
  - Requirements: 110 functional requirements grouped by topic + 9 non-functional. ✓
  - Success Criteria: 20 measurable outcomes. ✓
  - Optional sections (Key Entities, Assumptions, Dependencies on prior phases, Out of Scope) all present and populated.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
  - Notes: Zero markers in the spec; informed defaults applied per the skill's guidance (and per the session-wide "work without stopping for clarifying questions" instruction). All ambiguity surfaced by the Rust-lens reviewer was resolved by explicit FRs (FR-327, FR-349, FR-385, FR-410, FR-411, FR-449, FR-450, FR-602) rather than left as markers.
- [x] Requirements are testable and unambiguous
  - Notes: Each FR pins a verifiable behaviour or invariant. The closed-error-set extension (FR-601, FR-602) names every Phase 4 failure mode and its corresponding exit code; reused variants are explicitly enumerated.
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
  - Notes: SC-119 references the binary-size cap, which is a constitution-defined metric, not a technology choice. SC-115 ("All Phase 1, 2, 3 SCs continue to hold") is a regression criterion measured by re-running the prior phases' test suites against Phase 4 code — same shape as Phase 3's SC-110.
- [x] All acceptance scenarios are defined
  - Notes: Five user stories carry 8, 11, 10, 8, 10 acceptance scenarios respectively (47 total). Each scenario is Given/When/Then with a single observable outcome.
- [x] Edge cases are identified
  - Notes: ~25 edge cases covering project-marker corruption, parallel command races, workspace renames against running MCP servers, composition cycles, multi-harness shared rules files, hand-edited Tome blocks, summariser output failures, schema-migration races, WSL filesystem boundary.
- [x] Scope is clearly bounded
  - Notes: "Out of Scope (Phase 4)" section explicitly excludes hooks/commands/agents translation, additional harnesses, HTTP/SSE for MCP, summariser config knobs, cross-machine sync tooling, native Windows / WSL1 / WSL2-on-Windows-FS, garbage collection, plugin authoring tools.
- [x] Dependencies and assumptions identified
  - Notes: "Dependencies on Phases 1, 2, and 3" section enumerates 8 carry-forward contracts. "Assumptions" section enumerates 9 items including the constitution-amendment requirement, summariser model identity (Qwen2.5-0.5B-Instruct + llama-cpp-2), `@`-include support per harness as implementation-time verification, and WSL filesystem boundary.

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
  - Notes: Each user story's acceptance scenarios cover its FR group(s). Specific FR-to-scenario mappings are implicit from the topic groupings (workspace lifecycle FRs map to US2 scenarios; layered settings FRs map to US3 scenarios; summarisation FRs map to US4 scenarios; doctor FRs map to US5 scenarios; the cross-cutting refactor FRs — paths, central DB, plugin enable/disable rewrite — are validated indirectly through US1's "agent can search workspace skills via MCP" and SC-101 through SC-105).
- [x] User scenarios cover primary flows
  - Notes: P1 (US1) is the headline command flow — bind project, get cross-harness integration. P2 (US2, US3, US4) cover lifecycle, layered settings, summarisation — all required for US1 to be usable. P3 (US5) is doctor.
- [x] Feature meets measurable outcomes defined in Success Criteria
  - Notes: Every SC has at least one FR backing it. SC-101 through SC-105 are the "headline outcome" SCs (paths, central DB, workspace creation, project binding, RULES.md propagation). SC-106 through SC-118 cover specific subsystem behaviours.
- [x] No implementation details leak into specification
  - Notes: Two library names (`serde_json` with `preserve_order`, `toml_edit`) appear in FR-349 because the third-party preservation contract requires a category of library; this is a *correctness* constraint, not an implementation choice. The summariser model identity (Qwen2.5-0.5B-Instruct, GGUF INT4, llama-cpp-2) appears in the Assumptions section per the same pattern as Phase 3's embedder / reranker identities — these are model contracts that affect the binary-size budget and the licensing review, both of which the constitution gates on.

## Validation Outcome

**Status**: PASS (all sections checked, zero `[NEEDS CLARIFICATION]` markers, all blockers from the Rust-lens review resolved in-spec).

## Rust-Lens Review Disposition

The Rust-lens reviewer (devs:rust-dev) surfaced **8 blockers, 11 majors, 12 minors**. Disposition:

| Severity | Count | Resolved in spec | Deferred to plan | Notes |
|----------|-------|-------------------|-------------------|-------|
| Blocker  | 8     | 8                 | 0                 | B1 (closed error set: 7 unmapped modes) → FR-601 expansion + FR-602 reused-variant list. B2 (`llama-cpp-2` singleton + MCP fallback) → FR-421 + FR-425. B3 (atomic multi-file directory landing) → FR-410 + cross-references in FR-400/403/404. B4 (v1→v2 migration semantics) → FR-327 + SC-116 update. B5 (concurrent `workspace use`) → FR-322 PK clarification + FR-342 + FR-403 advisory-lock contract. B6 (refcount TOCTOU) → FR-366 + FR-367. B7 (composition resolves to as-written) → FR-449. B8 (third-party config preservation) → FR-349 + FR-500. |
| Major    | 11    | 9                 | 2                 | M1 (block marker syntax) → FR-480. M2 (variant reuse justification) → partially addressed via FR-602 enumeration; code-16-vs-53 parallel is plan-time. M3 (override priority chain) → FR-344. M4 (define "project" for `workspace use`) → FR-403. M5 (read-during-migration) → FR-325. M6 (`std::env::home_dir` vs `home` crate) → plan-time research item. M7 (cascade ordering) → FR-405 with explicit numbered ordering. M8 (byte-for-byte sync idempotence) → FR-525. M9 (`block_body_style` on harness contract) → FR-461. M10 (summariser failure during enable) → FR-385. M11 (project marker strict + developer ergonomics) → FR-348. |
| Minor    | 12    | 4                 | 8                 | m4 (SC-105 phrasing tightening), m5 (`--inherit-global` no-op), m7 (reserved-name policy), m11 (RULES.md scaffold) addressed inline. m1 (binary-size projection), m2 (harness sync failure handling), m3 (last_used_at timing) addressed by FR-403/FR-411. m6, m8, m9, m10, m12 deferred to plan. |

The reviewer's spec-level observations (length, prioritisation, edge-case coverage, "resolved decisions" table value, dense acceptance scenarios, wire-format stability subsection) are noted; none rise to a blocker. The plan should consider adding a brief "Wire format stability across the refactor" subsection for catalog list / plugin list / status / version output structured forms — Phase 3 SCs (SC-110) demand the structured forms continue to behave; Phase 4 mainly changes the underlying queries, not the wire shape, so this is a mechanical regression-test exercise.

## Constitution Gate Notes

The spec author flagged the paths-principle conflict (CONSTITUTION.md §Operational Constraints §Paths, which mandates "XDG-aware via `directories`. Never hardcode `~/.tome`.") in the Assumptions section. Phase 4's FR-300 / FR-302 / FR-303 require dropping the `directories` crate and consolidating under `~/.tome/`. Per the constitution's governance rule, the PRD wins on *what* the layout should be. The plan's first deliverable MUST include a constitution v1.3.0 amendment rewriting the Paths operational constraint. The amendment requires a PR with brief rationale, green CI, and `Last Amended` bumped — non-NEGOTIABLE principles need a 24-hour cooling-off period, but Paths is an Operational Constraint, not a NON-NEGOTIABLE principle, so no cooling-off is required.

Additional gate notes from the Rust-lens reviewer:

- **Principle XII ("Inherit, Don't Reimplement")**: bundling Qwen2.5-0.5B + the LLM inference C-library is the third bundled inference runtime. The complexity-budget rule (Governance §Complexity budget) requires a one-paragraph justification in the plan: "we bundle a small summariser rather than calling an external API because (a) offline-first, (b) no API-key burden on contributors and users, (c) latency budget, (d) deterministic-enough output across machines." This justification belongs in the plan, not the spec.

- **Binary size 50 MB cap**: spec carries safety clause (NFR-101: "if the cap would be breached, the plan MUST revise rather than waive"). Plan must include a measurement of the linked summariser library against the cap. Projected ~30 MiB on macOS arm64 (Phase 3 baseline 22 MiB + ~8 MiB for the LLM C-library statically linked CPU-only).

- **Async constraint**: structural sync-boundary test (tests/sync_boundary.rs) currently exempts `src/mcp/`. Phase 4's summariser is sync; the new module (likely `src/summarise/` or `src/llm/`) MUST remain on the sync side. The plan's first task in the refactor user story should extend the sync-boundary test's allow-list if the directory choice changes; otherwise no change.

- **Strict-on-Tome-owned principle**: `manifest_strictness.rs` enforces this. Phase 4 adds new strict types (workspace settings.toml, project marker config.toml, global settings.toml). The structural test MUST be extended; this is a plan-time test-discipline note, not a spec change.

## Notes

- Items marked complete in this checklist are spec-level validation; they do not warrant implementation tasks themselves. Implementation tasks are produced by `/sdd:tasks` after `/sdd:plan` runs.
- The Rust-lens reviewer's deferred items (M2 code-reuse justification, M6 std vs home crate, minors m6/m8/m9/m10/m12) are appropriate plan-time decisions; the spec is feature-complete and can move to planning.
