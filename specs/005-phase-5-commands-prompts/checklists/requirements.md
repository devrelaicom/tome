# Specification Quality Checklist: Phase 5 — Commands as Prompts, Unified Entries, and Variable Substitution

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-05-26
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

## Validation Notes

Initial pass surfaced 3 blockers + 8 majors + 12 minors + 6 suggestions from a Rust-lens spec review. The spec was edited inline before this checklist landed; the items below capture the resolution.

### Blockers — resolved

- **B1 (substitution pass count was ambiguous)**: FR-051 rewritten — "Each substitution stage MUST scan the body at most once" replaces "MUST run exactly once over the entry body"; composite-pass invariant made explicit.
- **B2 (schema migration was silent on backfill defaults)**: New **FR-111a** pins backfill values for `kind`, searchability, user-invocability, `when_to_use` on existing rows.
- **B3 (data-directory failure mode under-specified)**: Edge Case for data-dir creation failure clarified to allow a single error code covering both directory classes; new **NFR-012** requires creation to be idempotent and safe under concurrent retrieval; FR-021 strengthened.

### Majors — resolved

- **M1**: FR-062 collision tie-breaker pinned — lexicographic (catalog, plugin, kind, name) when timestamps equal; counter starts at 2.
- **M3**: New **FR-025** mandates relocation of `${TOME_WORKSPACE_DATA}` directory on workspace rename, with dedicated error code on failure.
- **M4**: FR-007 reaffirms third-party lenient parsing for the new frontmatter fields.
- **M5**: New **NFR-010** mandates substitution be invocable from blocking contexts.
- **M6**: NFR-001 reworded to include clock reading in the determinism input set; clock-derived built-ins evaluated once per pass.
- **M7**: FR-021 anchors persistent data directories under Tome's central state tree (NOT under bound project's marker directory).
- **M8**: FR-071/072 pin the catch-all argument name as `args`.
- **M2**: FR-082 pins the per-directory cap at 5.

### Minors — resolved

- **N1**: SC-009 reworded to be testable from input string, not output.
- **N2**: "Sitting between" dropped from FR-080.
- **N3**: Edge Case 9 references "entry-not-found error code" with contract pinning the numeric value.
- **N4**: FR-024 clarifies that `${TOME_CATALOG_NAME}`/`${TOME_PLUGIN_NAME}` return unsanitised values; sanitisation applies only to path-construction contexts.
- **N5**: FR-020 defines the Tome namespace as `${TOME_<NAME>}` with `<NAME>` matching uppercase ASCII + digits + underscores.
- **N6**: SC-007 reworded — entries whose `when_to_use` newly contributes to embedding text are re-embedded but identity-preserved.
- **N7**: "Regex pattern matching" replaced with "simple pattern matching" in Assumptions.
- **N8**: "Tera-class dependency" replaced with "full templating-engine dependency" in §Overview.
- **N9**: One-line MCP gloss added to §Overview for non-technical readers.
- **N10**: FR-007 inlines the full list of recognised frontmatter fields rather than redirecting to PRD.
- **N11**: NFR-008 keeps the list-changed-not-supported decision as a behavioural commitment without "per the source PRD" redirect.
- **N12**: SC-002 reworded — "environments" instead of "machines".

### Suggestions — adopted

- **S1**: New **SC-013** requires byte-stable serialisation tests for every new MCP response shape and diagnostic record.
- **S2**: New **NFR-011** bounds substitution layer's working memory by a constant multiple of body size; no superlinear scans.
- **S3**: FR-063 clarifies that harness-side `mcp__<server>__` prefix is preserved; `prompt_name` override only replaces Tome's contributions.
- **S4**: FR-030 defines the user-controlled env namespace as `${TOME_ENV_<NAME>}` with `<NAME>` matching uppercase ASCII + digits + underscores; FR-031 pins the `:-default` syntax.
- **S5**: New **FR-124** mandates doctor's Phase 5 surface is read-only by default.
- **S6**: New **FR-046** lists exactly which surfaces invoke the substitution layer.

### Notes

- The spec inherits Phase 4's centralisation invariants (single `<home>/.tome/` tree, central SQLite DB, named workspaces). Phase 5's new persistent data directories are anchored to that tree per FR-021/M7 resolution above.
- The spec is technology-agnostic in phrasing; the eventual plan, research, and contracts under `specs/005-phase-5-commands-prompts/` will pin the Rust-side implementation choices.
- One assumption ("`directories not need active cleanup in Phase 5`") explicitly defers orphan cleanup tooling to Phase 6+; this is consistent with the PRD's resolved decisions.
