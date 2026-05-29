# US1 (Native agents) — Disposition

Decisions on the [findings](./us1-findings.md). Committed before fixes are applied.

## Fix now (blocker + selected majors/minors)

| ID | Sev | Decision | Action |
|---|---|---|---|
| S-1 | BLOCKER | **Fix** | Validate agent `name` as a single safe path segment at index time (`collect_agent_entries`) reusing the `identity::validate_segment` discipline; reject → exit 45 (`AgentTranslationFailed`). Defence-in-depth: assert the resolved write `target.parent() == Some(agent_dir)` (no `ParentDir` components) in `reconcile_agents` before write. Add a traversal-attempt test. |
| R-1 | MAJOR | **Fix** | Propagate `open_read_only` failure for an *existing* DB (`if index_db.exists() { open_read_only()? } else { None }`) so a SchemaTooNew/busy DB never collapses to an empty enabled set and triggers the mass-deletion cleanup. |
| R-2 | MAJOR | **Fix** | Consolidate the `<plugin>__*` ownership rule onto the `agents.rs` SSOT helpers; promote a `plugin_of_owned_file` accessor; delete `sync.rs`'s duplicated split logic; remove the module-wide `#![allow(dead_code)]` from `agents.rs`. |
| T-1 | MAJOR | **Fix** | Add a symlink-refusal test on the agent write path (exit 7, target not overwritten). |
| T-2 | MAJOR | **Fix** | Add per-harness read-only drop tests: indeterminate posture and explicit not-read-only → no harness read-only key emitted + recorded drop. |
| C-2 | MINOR→fix | **Fix** | Record the canonical *source* field(s) (`tools`/`disallowedTools`) in `dropped_fields` for the read-only-indeterminate case, not the harness target name — keeps the US5 doctor `DroppedFieldEntry` honest. |
| T-5/C-3 | MINOR→fix | **Fix** | Add `agents_absent_from_search` (real `search_skills` query returns no agent — load-bearing FR-070) and `same_name_skill_and_agent_produce_two_rows`. |
| T-4 | MINOR→fix | **Fix** | Add the agent forward-progress test (one malformed + one good agent → good emits, sync returns exit 45). |
| T-3 | MINOR→fix | **Fix** | Add a multi-harness single-sync fan-out test (two native harnesses, one agent emits into both dirs). |
| R-3 | MINOR | **Fix** | Correct the `write_agent_file` "0700 parent dirs" comment to "umask-governed". |
| R-4 | MINOR | **Fix** | `enable_plugin_atomic` calls `embed_unless_agent` (single-source the embed-skip predicate). |

## Defer (documented)

| ID | Sev | Decision | Rationale |
|---|---|---|---|
| C-1 | MINOR | **Defer + comment** | Full `tools` allowlist→per-tool-permission translation is an enhancement; the contract's core is read-only *intent* reconstruction, which is implemented. Add a code comment that only read-only intent is reconstructed (allowlist scoping beyond that is dropped). |
| C-4 | MINOR | **Defer + comment** | `prepare_agent`-failure-on-enabled-agent leaving a stale file is a narrow post-enable-corruption edge (malformed agents can't enable). Add a code comment; US5 `doctor --fix` removes orphans. |
| S-2 | MINOR | **Defer** | Intermediate-dir symlink check matches the pre-existing Phase 4 final-node-only discipline; out of scope for US1, project-wide concern (Polish P8 security backlog). |
| S-3 | MINOR | **Defer to US5** | Surfacing privileged passthrough is exactly the US5 `PrivilegeEscalationReport` (FR-051); no need to duplicate in `SyncOutcome`. |
| Per-agent shrink removal | — | **Defer + retro note** | Defensible under literal FR-043 (removal scoped to disabled plugins / non-live harnesses); US5 `doctor --fix` removes orphaned `<plugin>__*`. Recorded in P3 retro. |
| `TranslatedAgent.dir` dead field | INFO | **Leave** | Informational-only; harmless; consistent across harnesses. |
| AGENTS.md-first candidate | INFO | **No action** | Correct for US1; the rules-file correction is US3 (FR-020/022). |
