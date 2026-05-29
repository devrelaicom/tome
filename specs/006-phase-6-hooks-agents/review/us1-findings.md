# US1 (Native agents) — Reviewer findings

4-reviewer parallel pass (contract / Rust-lens / test / security) against the US1 surface (`fa9e5eb..HEAD`). Recorded verbatim-in-substance before any fix is applied.

## Security

- **[BLOCKER] S-1 Path traversal via attacker-controlled agent `name`.** Agent frontmatter `name` is resolved first (`CanonicalAgent::parse`, `agents.rs:190`) and stored verbatim into `skills.name` (`collect_agent_entries`, `lifecycle.rs:757`) with no path-segment validation. `agent_filename` (`agents.rs:277`) builds `"{plugin}__{name}.{ext}"` and `reconcile_agents` does `dir.join(filename)` + `write_standalone` (`sync.rs:829`). A plugin shipping `name: ../../../../tmp/evil` writes attacker-controlled content outside the harness `agent_dir` (verified: `dir.join("p__../../../../tmp/evil.md")` normalises outside). Fix: validate `name` as a single safe path segment (reuse `identity::validate_segment` discipline — reject empty/`/`/`\`/`..`/`.`/leading-`.`/NUL) at index time AND assert `target.parent() == Some(dir)` (no `ParentDir` components) before write.
- **[MINOR] S-2** Symlink refusal is final-node only (`refuse_symlink`, `rules_file.rs:236`) — a symlinked *intermediate* dir is not checked. Pre-existing Phase 4 discipline; defence-in-depth note.
- **[MINOR] S-3** Privileged passthrough (hooks/mcpServers/permissionMode → `.claude/agents/`) is the intended FR-050 default and is auditable in-file, but is not surfaced in `SyncOutcome`, so a sync gives no signal an enabled plugin just installed hooks/MCP/elevated permission.
- Non-issues confirmed: bounded reads, O(n) SQL, render via `toml_edit`/`serde_yaml` (no injection), `validate_db_stored_path` on source path, model never cross-vendor.

## Rust-lens

- **[MAJOR] R-1 Silent DB-open error → mass agent-file deletion.** `reconcile_agents` uses `open_read_only(&index_db).ok()` (`sync.rs:694`), collapsing SchemaTooNew (52) / vec-ext / busy errors on an *existing* DB to `None`. `None` → empty `enabled_plugins` → the cleanup pass matches and deletes EVERY `<plugin>__*` file for live native harnesses. Fix: `if index_db.exists() { let conn = open_read_only(...)?; ... } else { None }` so an existing-but-unopenable DB fails loudly instead of wiping emitted agents.
- **[MAJOR] R-2 SSOT removal helpers dead; sync re-rolled the logic.** `agents.rs` `is_owned_agent_file`/`owned_agent_files` (documented FR-043 removal SSOT) have 0 external refs; `sync.rs` reimplemented the `<plugin>__` split rule 3× (`owned_plugin_of`, `removed_disabled_owned`, `all_owned_in_dir`). Drift risk. Fix: consume the agents.rs helpers (promote a `plugin_of_owned_file` there), delete duplicated logic, remove the now-unjustified module-wide `#![allow(dead_code)]`.
- **[MINOR] R-3** `write_agent_file` doc claims "0700 parent dirs"; `atomic_write` uses umask-governed `create_dir_all`. Correct the comment.
- **[MINOR] R-4** `enable_plugin_atomic` inlines the agent-embed-skip; should call `embed_unless_agent` (single-source the predicate).
- Deferrals assessed: `TranslatedAgent.dir` informational (OK); per-agent-shrink removal within an enabled plugin (defensible under literal FR-043; US5 `doctor --fix` removes orphans) — record in retro.

## Test

- **[MAJOR] T-1 Symlink refusal on agent writes (exit 7) entirely untested** — `write_standalone`'s `refuse_symlink` has no test anywhere. Add: pre-plant a symlink at the agent target, sync, assert exit 7 + target not overwritten.
- **[MAJOR] T-2 Read-only intent indeterminate→dropped + not-read-only→dropped untested at the translate level** per harness (only the `infer_read_only` helper unit-tests the None case). Add per-harness: indeterminate posture → no `sandbox_mode`/`readonly`/`permission` key + recorded drop.
- **[MINOR] T-3** No multi-harness single-sync fan-out test (every sync test installs one harness).
- **[MINOR] T-4** No agent-specific forward-progress test (one malformed + one good agent → good emits, sync returns exit 45).
- **[MINOR] T-5** Missing `agents_absent_from_search` (actual `search_skills` query returns no agent — load-bearing FR-070) and `same_name_skill_and_agent_produce_two_rows`.
- **[MINOR] T-6** OpenCode `model: None`/no-model drop path unverified at translate level.
- Confirmed well-covered: 4 happy paths, Codex triple-quote round-trip, SC-002, OpenCode mode/description fallback, clash prefixing, removal glob, mtime-verified idempotence, T053 placeholder clearly bounded.

## Contract

- **[MINOR] C-1** `tools` allowlist is not translated (only read-only *intent* is reconstructed); a restrictive non-read-only allowlist is silently dropped. Document that only read-only intent is reconstructed (or implement allowlist→permission later).
- **[MINOR] C-2** `dropped_fields` records the *harness-native target* name (`sandbox_mode`/`readonly`/`permission`) when read-only is indeterminate, instead of the *canonical source* field (`tools`/`disallowedTools`). Will confuse the US5 doctor `DroppedFieldEntry`. Record the source field name.
- **[MINOR] C-3** Missing contract tests `agents_absent_from_search` + `same_name_skill_and_agent_produce_two_rows` (overlaps T-5).
- **[MINOR] C-4** `prepare_agent` failure for an enabled agent (post-enable source corruption) leaves a stale prior-sync file while surfacing exit 45. Narrow (malformed can't enable). Add a code comment.
- **[INFORMATIONAL]** `claude_code` rules-file candidate array still lists `AGENTS.md` first — CORRECT for US1; the Phase 4 rules-file correction is assigned to US3 (FR-020/021/022). Not a US1 regression.

**Overall**: 1 BLOCKER (S-1 path traversal), 4 MAJOR (R-1 data-loss, R-2 SSOT, T-1 symlink test, T-2 read-only-drop tests), plus MINORs. No catch-all regressions; SC-002 verified.
