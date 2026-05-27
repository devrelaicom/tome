# Phase 5 Polish — Disposition

Decision record for `findings.md`. Mirrors the per-US disposition pattern. Applied items are split across PR-B (Rust fixes), PR-C (test fills + contract doc), PR-D (docs + release), PR-E (closeout).

## Applied — PR-B (Rust fixes)

1. **M-1** — Port the US4.d bounded `char_indices` walk into `prompts::truncate_description`. Update docstring to truthfully claim "mirrors search_skills' bounded-walk approach". (Smaller blast radius than extracting a shared `util::text` helper; can revisit at v0.6+ if a third truncate site emerges.)
2. **M-2** — Extract `build_context_for_entry` helper into `src/substitution/context.rs` (or a sibling module). Both call sites (`prompts.rs::build_get_context` + `get_skill.rs::build_substitution_context`) reduce to one-line calls. Drops the stale "Real production callers in US2 will replace this" comment from `prompts.rs`.
3. **M-3** — Replace `match kind.as_str()` in `doctor/checks.rs:483` + `commands/plugin/mod.rs:266` with `kind.parse::<EntryKind>()` + match-on-variants. Surface unknowns as `IndexIntegrityCheckFailure`.
4. **M-4** — Extract `pub(crate) fn validate_db_stored_path(stored: &Path) -> Result<(), TomeError>` (or `is_safe_relative`) helper to `src/index/skills.rs`. Consume from both `resolve_entry_body_path` and `commands/plugin/show.rs::list_entries`.
5. **m-3** — One-line comment on `apply_arguments_match`'s `unwrap_or(usize::MAX)` overflow path.
6. **m-4** — Rename `PromptEntry::descriptor`'s `name: String` parameter to `prompt_name: String` to disambiguate from `self.name`.

## Applied — PR-C (test fills + contract doc)

1. **GAP-1** — Add e2e exit-code tests for Phase 5 codes 9 + 25 in `tests/exit_codes_e2e.rs`. Codes 26 (PromptArgumentMismatch), 27 (EntryNotFound), 28 (SubstitutionFailed), 29 (InvalidArgumentFrontmatter) are MCP-server-only error paths that require driving the rmcp server in-process; defer those to v0.6+ when test infrastructure for the MCP wire surfaces in `tests/exit_codes_e2e.rs` (they ARE covered in `tests/mcp_*.rs` at the library API level).
2. **GAP-2** — Add `pending_re_embedding_zero_when_no_files_touched` to `tests/doctor_p5.rs`.
3. **CA-M1** — Doc-only amendment to `contracts/substitution-engine.md` § Stage 4 clarifying single-string vs Object input shapes for the append-fallback.

## Applied — PR-D (docs + release)

1. `Cargo.toml` version bump: `0.5.0-dev` → `0.5.0`.
2. `CHANGELOG.md` new `[0.5.0]` entry naming all five Phase 5 user stories, the substitution engine, the MCP prompts capability + the `get_skill_info` middle-tier tool, the doctor Phase 5 surfaces, schema migration v2→v3, the 6 new exit codes (9, 25-29), and the test count delta (954 → 1193).
3. `README.md` update: "Phase 4 shipped (v0.4.0)" → "Phase 5 shipped (v0.5.0)" with a short paragraph naming the commands-as-prompts + substitution + doctor extensions.
4. Constitution check: no amendments needed; Phase 5 added zero new top-level dependencies (`regex` promotion from transitive → direct happened at Phase 5 start before US1.a; documented in `retro/P2.md`).

## Applied — PR-E (closeout)

1. `/sdd:map incremental` — refresh all 8 `.sdd/codebase/*.md` docs via 4 parallel mappers against the post-Polish-fix tree.
2. `retro/P8.md` — fill the Polish phase retro with Polish-specific learnings.
3. `CLAUDE.md` — update current-phase line to "v0.5.0 shipped" + add a Recent Changes entry for Polish.

## Deferred to v0.6+ / Polish backlog

- All items from `us{1..5}-disposition.md` carried forward.
- **WEAK #2-1** (`SubsystemHealth` enum variants pin): Phase 4 type; existing JSON tests structurally cover.
- **WEAK #3-1** (`MAX_DESCRIPTION_MAX_CHARS` parse-time cap): cap is intentionally soft per trust model (US5.c R-M4).
- **GAP #2-1** (`ProjectBindingState`, `RulesCopyState`, `HarnessSubsystemReport` pins): Phase 4 types; no regression risk.
- **GAP-1 partial defer**: Phase 5 exit codes 26-29 require MCP server in-process driving in `exit_codes_e2e.rs`; they ARE covered at the library API level via `tests/mcp_prompts*.rs` and `tests/mcp_get_skill*.rs`. v0.6+ test infrastructure work.
- **m-1** (`Value::String(s) => s.clone()` cosmetic).
- **m-2** (`body_references_arguments` one-line delegate inline opportunity).
- 3 cosmetic Rust minors + 3 nits from per-US dispositions.
- **MEDIUM symlink path disclosure** (deferred since US5.c, resolved by cap-std hardening).
- **LOW terminal escape sequences in plugin/entry names** (design surface).
- TOCTOU residual in `walk_resources` (US4.d disposition).
- Read-only DB open refactor (Phase 3 backlog).

## Application order

1. **PR-A** (this commit set) — `docs(review):` two files: `findings.md` + `disposition.md`.
2. **PR-B** — `fix(phase-5-polish):` Rust fixes M-1/M-2/M-3/M-4 + m-3/m-4.
3. **PR-C** — `test(phase-5-polish):` + `docs(contracts):` test fills + Stage 4 doc clarification.
4. **PR-D** — `chore(release):` + `docs:` version bump + CHANGELOG + README.
5. **PR-E** — `docs:` mapper refresh + retro fill + CLAUDE.md.

Tests must remain green at every commit boundary.
