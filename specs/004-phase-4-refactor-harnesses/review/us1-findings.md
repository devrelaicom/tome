# US1 — Pre-Closeout Review Findings

Four reviewers dispatched in parallel against the merged US1 surface (PRs #74–#79):

- **Contract audit** — compare shipped behaviour against `contracts/*.md`
- **Rust-lens code review** — correctness, ergonomics, idiomaticity
- **Test audit** — coverage gaps + test quality
- **Security audit** — new attack surfaces from rules-file + MCP config writes

Per-reviewer source files in `/tmp/tome-review-us1-{contract,rust,test,security}.md`.

Counts: **3 blockers, 25 majors, ~30 minors+nits.**

Triage is in `us1-disposition.md`.

---

## Blockers (3)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **R-B1** | rust | `to_string_lossy()` lossy-encodes non-UTF8 project paths into `workspace_projects` PK; subsequent lookups silently miss the row → silent data corruption. | `src/workspace/binding.rs:181` |
| **T-B1** | test | `tests/harness_sync_stub.rs::force_overrides_clash_and_preserves_env` pins the OPPOSITE of contract `mcp-config-integration.md` §"env field preservation" — contract reads as blanket `env` preservation on `--force`; test asserts env dropped for user-owned. One side is wrong; contract has internal ambiguity (the §Ownership-marker bullet says preserve; §"env field preservation" restricts to Tome-owned rewrites). Test stance is the safer one; resolve by clarifying the contract. | `tests/harness_sync_stub.rs:233-270` vs `contracts/mcp-config-integration.md:45-53` |
| **T-B2** | test | `tests/sync_idempotence.rs` referenced by 3 contracts (`rules-file-integration.md:103`, `sync-algorithm.md:71`, `plan.md`) does NOT exist. Cross-harness FR-525 idempotence is scattered across single-harness files; the named holistic proof is missing. | "no file at `tests/sync_idempotence.rs`" |

## Majors (25)

### Contract audit (3)

| # | One-line | File ref |
|---|---|---|
| C-M1 | When claude-code AND opencode are both in the effective list and target the same `AGENTS.md`, the orchestrator picks the FIRST-iteration body style (AtInclude wins lex-order), so opencode silently gets a directive it doesn't support. | `src/harness/sync.rs:209-248` |
| C-M2 | (Pair with T-B1) `--force` overriding a user-owned entry drops env; contract §"Ownership marker" reads as blanket-preserve. Resolve by amending the contract carve-out. | `src/harness/mcp_config.rs:412-457` |
| C-M3 | Temp file mode 0600 (mkstemp default) ≠ contract-pinned 0644. Fresh `.claude/settings.json` lands at 0600. | `src/harness/rules_file.rs:252`, `src/harness/mcp_config.rs:123` |

### Rust-lens (7)

| # | One-line | File ref |
|---|---|---|
| R-M1 | DB UPSERT + `last_used_at` bump are two separate `conn.execute()` calls — not transactional. Contract `workspace-commands.md` line 88 says single transaction. | `src/workspace/binding.rs:205-224` |
| R-M2 | `read_to_string(...).unwrap_or_default()` in `compute_rules_body` Inline path silently swallows EACCES → empty rules block. | `src/harness/sync.rs:426` |
| R-M3 | JSON MCP config writes via `to_vec_pretty` discard original indentation; docstring overclaims full formatting preservation. | `src/harness/mcp_config.rs:423, 536` |
| R-M4 | `is_project_root_acceptable` swallows canonicalise errors → fallback to literal path equality, can both false-negative AND false-positive. | `src/workspace/binding.rs:81-82` |
| R-M5 | `bind_project` 130-line body mixes lockfile + 2-step DB writes + FS landing; no DB transaction means partial commit possible on FS-landing failure. | `src/workspace/binding.rs:124-259` |
| R-M6 | `WorkspaceCommand::Use` and `McpArgs` doc comments still mention `--global` (F10 deleted). Surfaces in `--help`. | `src/cli.rs:124, 170` |
| R-M7 | `commands::workspace::run` accepts `scope: &ResolvedScope` but `Use` arm ignores it — user running `tome --workspace foo workspace use bar` silently gets `bar`. | `src/commands/workspace/mod.rs:20` |

### Test audit (11)

| # | One-line | File ref |
|---|---|---|
| T-M1 | No e2e coverage of nested AtInclude (`@../.tome/RULES.md`) — only top-level `<project>/AGENTS.md` case is exercised. | `tests/workspace_use_claude_code_e2e.rs` |
| T-M2 | No coverage of TOML inline-table branch in `mcp_config.rs:465-489`. | `tests/mcp_config_*.rs` |
| T-M3 | `tests/workspace_use_forward_progress.rs:273-277` hard-asserts code 7; surrounding comment tolerates 18/19 on permissive FS. Inconsistent. | `tests/workspace_use_forward_progress.rs` |
| T-M4 | No `tome workspace use <invalid-name>` → exit 15 coverage. | "no test for code 15 e2e" |
| T-M5 | No `--force` bypass for cwd-is-home refusal positive test. | `tests/workspace_use_binding.rs` |
| T-M6 | Happy-path tests never assert `outcome.sync == Some(_)` on the wrapper flow. | `tests/workspace_use_*` |
| T-M7 | No byte-stable JSON wire-shape test for `BindOutcome` or `SyncOutcome`. | "no JSON shape pins" |
| T-M8 | No positive `subdirectory-of-$HOME-is-acceptable` test. | `tests/workspace_use_binding.rs` |
| T-M9 | `OtherStubHarness.detect()` returns true unconditionally; no test exercises `detect() == false` filtering. | `tests/sync_algorithm.rs:36,70` |
| T-M10 | Symlink-refusal tests are Unix-only without explicit "Unix-only" reasoning comment. | `tests/rules_file_*` |
| T-M11 | No integration test for empty-args `is_tome_owned` clash path. | `tests/mcp_config_clash.rs` |

### Security audit (4)

| # | One-line | File ref |
|---|---|---|
| S-M1 | Unbounded `read_to_string` on third-party files (`AGENTS.md`, `.claude/settings.json`, etc.). Phase 3 PR-F established 1 MiB cap; same discipline needed here. | multi-site (see audit) |
| S-M2 | Symlink refusal has TOCTOU window across multiple stat+read calls; `rename(2)` no-follow semantic mostly closes but should be documented or replaced with `O_NOFOLLOW`. | `mcp_config.rs:312,368`, `rules_file.rs:269,297,356,384` |
| S-M3 | Original 0644 mode of pre-existing harness files silently changed to 0600 on rewrite (NamedTempFile default). | `rules_file.rs:252`, `mcp_config.rs:123` |
| S-M4 | Harness-owned parent dirs (`<project>/.cursor/`, `<home>/.codex/`) get chmod 0700 on Tome-create — should respect harness convention. | `rules_file.rs:360-367`, `mcp_config.rs:112-121` |

---

## Minors + nits

See per-reviewer files in `/tmp/tome-review-us1-{contract,rust,test,security}.md`. ~30 items spanning idiom, redundant assertions, docstring drift, formatting cleanup. Dispositioned in `us1-disposition.md` (mostly "defer; not load-bearing").

---

## Verdict

The shipped US1 surface is solid on every load-bearing dimension. Three blockers (one data-correctness, two test-contract integrity) and a manageable list of majors. No security blocker. The blockers + ~9 highest-impact majors land in **PR US1.d-2a**; the remainder defer to a tracked follow-up issue or to US5's doctor `--fix` slice (which already owns the related orphan-cleanup work).
