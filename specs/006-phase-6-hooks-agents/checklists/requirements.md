# Specification Quality Checklist: Phase 6 — Hooks and Agents

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-05-28
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Notes

- **Validation result**: PASS (all items). Validated 2026-05-28 after a Rust-lens review pass (`devs:rust-dev`) surfaced 2 blockers + 10 majors + 6 minors, all folded into the spec before this validation (see below).
- **On "No implementation details"**: This spec deliberately names a small number of internal artefacts where a requirement is load-bearing and would otherwise be ambiguous or hide a latent failure — specifically the `EntryKind` enum widening (FR-070a), the Tome-owned settings structs and their layering (FR-053), and the model-alias table as a named verification artefact (FR-037). This is consistent with the established house style of the Phase 1–5 specs (which name the schema-migration framework, the strictness boundary, the embedding-text composition, etc.), and the project is single-author where the spec author is the implementer. The references are minimal, justified per requirement, and the rest of the spec stays WHAT-focused. Marked PASS on that basis; flag for a reviewer who prefers a stricter WHAT/HOW separation.
- **Rust-lens review findings applied** (all blockers + all majors + applicable minors):
  - B1 → FR-070a + FR-071 reword + Assumptions reword (the `kind='agent'` row would crash existing exhaustive `EntryKind` matches; widening is now explicit and in-scope).
  - B2 → FR-022 reword (Claude Code rules-file candidate precedence is now pinned to `CLAUDE.md` > `.claude/CLAUDE.md`, with `AGENTS.md` removed — the substance of the Phase 4 correction).
  - M1 → FR-016 (hooks-presence determination computed before `CLAUDE.md` guardrails reconciliation; both suppression transitions specified symmetrically).
  - M2 → FR-053 (the two new bool settings as strict Tome-owned fields; first-declarer-wins priority walk, not the `harnesses` composition grammar).
  - M3 → FR-067 (persona toggle resolved against the MCP server's startup scope, reconciling "workspace-layered setting" with "global exposure").
  - M4 → FR-036 expanded (canonical read-only source fields + inference rule).
  - M5 → FR-072 tightened (clash set defined as workspace-enabled agent rows, computed once per sync, governing all three name consequences identically).
  - M6 → FR-066 (persona names share the single Phase 5 prompt-name collision namespace; clash prefix before counter-suffix backstop).
  - M7 → FR-063 amended (`drop-persona` is a reserved name).
  - M8 → FR-084 (cross-sink forward progress, mirroring Phase 4's binding-then-sync discipline).
  - M9 → FR-092 + Assumptions (exit-code reassignment precision; notes 34–37 are also taken).
  - M10 → FR-037 + SC-002 (model-alias table named as the single source of truth; SC-002 made verifiable against it).
  - m1 → FR-011a (guardrails marker literal pinned). m2 → FR-011 (deterministic placement order). m4/m5 → FR-031 (dir hedge + OpenCode singular). m6 → FR-035 (first non-empty line, placeholder fallback). m3 (numbering gaps) → no change (deliberate, matches house style).
- **Clean dimensions reported by the review** (no change needed): sync-only constraint honoured (only the existing `src/mcp/` async island); no new top-level dependencies; no-sidecar / filesystem-inferred state; atomic-write + symlink-refusal carried to all four new write sinks; persona substitution reuse is genuinely feasible against the Phase 5 machinery.
- Codebase documents under `.sdd/codebase/` were not re-generated for this spec (no code change yet), consistent with the Phase 5 spec's approach; they are current as of the Phase 5 Polish closeout.
