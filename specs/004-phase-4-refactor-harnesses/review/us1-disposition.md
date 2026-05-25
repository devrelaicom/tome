# US1 — Disposition

Maps findings in `us1-findings.md` to actions. Three buckets:

- **In PR US1.d-2a** — applied as part of the reviewer-pass commit
- **Follow-up issue** — non-blocking, tracked separately
- **Defer to US5 polish** — natural home in the doctor `--fix` work

## Blockers — all applied in US1.d-2a

| # | Disposition |
|---|---|
| R-B1 | Apply: refuse non-UTF8 project paths in `bind_project` via `Path::to_str().ok_or_else(...)`; surface `TomeError::Io` with a clear message naming the invariant. Add a unit test under `tests/workspace_use_binding.rs::refuses_non_utf8_project_path` (`#[cfg(unix)]` with `OsString::from_vec(b"\xff\xfe".to_vec())`). |
| T-B1 | Resolve by **amending the contract**: clarify in `contracts/mcp-config-integration.md` § "Ownership marker" that the `env` preservation guarantee applies ONLY when rewriting a Tome-owned entry (i.e. the existing entry passed `is_tome_owned`). User-owned `env` is dropped on `--force` overwrite — the safer reading aligned with the existing test stance. Rename the test to `force_overrides_user_owned_clash_drops_unowned_env` for clarity. |
| T-B2 | Create `tests/sync_idempotence.rs` covering the cross-harness FR-525 proof: bind to a workspace with `harnesses = ["claude-code"]`, sync, capture mtimes for AGENTS.md + `.claude/settings.json` + `STUB_RULES.md` + `stub.mcp.json` (with `HarnessModulesGuard` for the stub pair), sleep 1.5s, re-sync, assert ALL mtimes unchanged. |

## Majors — applied in US1.d-2a (9)

| # | Disposition |
|---|---|
| R-M1 + C-Mn1 | Apply: wrap the UPSERT + `last_used_at` update in a single `conn.transaction()`. Matches contract FR-411. |
| R-M2 | Apply: replace `unwrap_or_default()` with a `NotFound → empty string; other → Err` match, mirroring `read_workspace_settings`. |
| R-M4 | Apply: bubble canonicalise errors from `cwd`; document HOME canonicalise as best-effort. |
| R-M6 | Apply: scrub `--global` doc refs from `cli.rs` and any remaining surfaces. Grep first. |
| R-M7 | Apply: when `WorkspaceCommand::Use` is dispatched, ignore `--workspace` cleanly with a `tracing::warn!` noting the override (or document in `--help` that `<name>` always wins over global flag for `workspace use`). |
| C-N1 | Apply: extend `HarnessClash` Display to include the doctor pointer ("Re-run `tome doctor --fix` to repair after the clash is resolved"). |
| R-M3 | Apply (docstring): downgrade `mcp_config.rs` write_entry docstring claim from "preserves every other ... formatting" to "preserves key order and unknown keys; formatting normalised". |
| S-M3 | Apply: read existing target's mode before write; chmod the staged tempfile to that mode before persist. Add `tests/security_hardening.rs::preserve_file_mode_on_rewrite`. |
| T-M1, T-M4, T-M7 | Apply test additions: nested AtInclude e2e, exit 15 e2e, BindOutcome JSON wire-shape pin. |

## Majors — deferred to follow-up issue

| # | Reason for deferral |
|---|---|
| C-M1 | Multi-harness mixed-style edge case. Only fires when claude-code AND opencode both in effective list. Edge case; US3.c brings the full harness matrix tests. |
| C-M3 | Contract pins 0644 temp file mode but the libc default is 0600; aligns with S-M3 mode-preservation work. Cover in S-M3 fix or contract amendment. |
| R-M5 | bind_project length refactor — pure ergonomics; defer. |
| S-M1 | Unbounded `read_to_string` cap — multi-site change; natural home is a `util::bounded_read_to_string` helper + dedicated PR before v0.4 cut. |
| S-M2 | TOCTOU symlink window — closing it properly needs `O_NOFOLLOW`-aware open + dirfd-based rename. Phase 4 P5 polish or US5 doctor's hardening sweep. |
| S-M4 | chmod 0700 on harness-owned parent dirs — choice of "drop chmod" vs "respect harness convention" is a small design call; defer. |
| T-M2, T-M3, T-M5, T-M6, T-M8, T-M9, T-M10, T-M11 | Test gaps that are real but not regressions of shipped behaviour. Roll into a single follow-up "us1-test-gap-followups" issue tagged for US2/US3 polish phases. |

## Minors + nits

Bulk-deferred to a tracking issue. Each is documented in the per-reviewer files. Most are docstring drift, redundant assertions, or stylistic preferences that don't carry behavioural risk.

## Net effect of US1.d-2a

- ~12 production-source touches (file count: `cli.rs`, `binding.rs`, `sync.rs`, `mcp_config.rs`, `rules_file.rs`, `commands/workspace/mod.rs`, `error.rs` Display impl)
- 4 new tests, 1 amended test name, 1 new test file (`sync_idempotence.rs`)
- 1 contract amendment (`mcp-config-integration.md` env preservation clarification)
- 2 review artefacts (`us1-findings.md`, `us1-disposition.md`) — this file

Then US1.d-2b runs `/sdd:map incremental` + retro/CLAUDE.md updates as the final closeout PR.
