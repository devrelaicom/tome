# US2 — Pre-Closeout Review Findings

Four reviewers dispatched in parallel against the merged US2 surface (PRs #82–#85).

Per-reviewer source files in `/tmp/tome-review-us2-{contract,rust,test,security}.md`.

Counts: **4 blockers, 23 majors, ~30 minors+nits.**

Triage in `us2-disposition.md`.

---

## Blockers (4)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **C-B1** | contract | `workspace remove` cascade iterates the global `SUPPORTED_HARNESSES` instead of per-project effective list (contract step 1 line 145). | `src/workspace/remove.rs:377-388` vs `contracts/workspace-commands.md:145` |
| **C-B2** | contract | `workspace rename` marker rewrites happen OUTSIDE the DB transaction; a SQL UPDATE failure leaves markers pointing at `<new>` with DB row still at `<old>`. | `src/workspace/rename.rs:224-247` vs `contracts/workspace-commands.md:128-130` |
| **C-B3** | contract | `workspace list --json` wraps the array in `{"workspaces": [...]}` envelope; contract specifies bare array. | `src/commands/workspace/list.rs:42-45,141-143` vs `contracts/workspace-commands.md:53` |
| **T-B1** | test | `rename` destroys forward-compat fields in bound-project marker `config.toml` (wholesale `format!` rewrite drops the optional `harnesses` field per data-model §7). | `src/workspace/rename.rs:225-228` (same root as C-M3) |

## Majors (23)

### Contract audit (5)

| # | One-line | File ref |
|---|---|---|
| C-M1 | `init` never emits `[summaries]` section in settings.toml. | `src/workspace/init.rs:234-250` |
| C-M2 | `init` writes empty RULES.md (`b""`) instead of "no summary yet" comment. | `src/workspace/init.rs:214` |
| C-M3 | `rename` clobbers optional `harnesses` field in project markers (same root as T-B1). | `src/workspace/rename.rs:225-228` |
| C-M4 | `regen-summary <name>` accepts optional positional (contract synopsis: required). | `src/cli.rs:236-240`, `src/commands/workspace/regen_summary.rs:27-30` |
| C-M5 | `generated_at` written as TOML basic-string (contract example: unquoted datetime literal). | `src/workspace/regen_summary.rs:291` |

### Rust-lens (8)

| # | One-line | File ref |
|---|---|---|
| R-M1 | `remove` Step 5 HashSet dedup after SELECT; cleaner as `SELECT DISTINCT url`. | `src/workspace/remove.rs:228-255, 308-316` |
| R-M2 | `rename` module doc claims marker rewrites are in tx; code does them BEFORE tx. (Same root as C-B2.) | `src/workspace/rename.rs:36, 217-244` |
| R-M3 | `init` deep-clones Vec<CatalogEnrolment> + WorkspaceName into FnOnce closure. | `src/workspace/init.rs:204-208` |
| R-M4 | Hand-rolled TOML escape in `init::render_settings_toml` misses `\n`/`\r`/`\t`. | `src/workspace/init.rs:234-258` |
| R-M5 | `regen_summary` holds advisory lock during summariser invocation (many seconds with LlamaSummariser). | `src/workspace/regen_summary.rs:86, 116-160` |
| R-M6 | TOCTOU comment in `remove` Step 5 binds correctness to `conn` handle lifetime; future refactor could reintroduce gap. | `src/workspace/remove.rs:304-317, 350` |
| R-M7 | `info::compute_info` early-return for global-not-found drops `embedder`/`schema_version` already populated. | `src/commands/workspace/info.rs:166-188` |
| R-M8 | `rename` pre-checks dir+marker existence but doesn't re-verify before per-file writes (TOCTOU). | `src/workspace/rename.rs:194-228` |

### Test audit (6)

| # | One-line | File ref |
|---|---|---|
| T-M1 | No JSON wire-shape pin for `InitOutcome`, `RenameOutcome`, `RegenSummaryOutcome`, `RemoveOutcome`. | `tests/workspace_{init,rename,remove,regen_summary}.rs` |
| T-M2 | No concurrent-init/rename/regen tests via `Barrier::new(2)`. | same |
| T-M3 | Cascade Step-1 only covers Claude Code MCP entry; no rules-file teardown or user-owned MCP-entry preservation coverage. | `tests/workspace_remove_cascade.rs:67-111` |
| T-M4 | `FailingSummariser` uses `matches!(err, SummariserFailure { .. })` — `kind: ModelMissing` payload unverified. | `tests/workspace_regen_summary.rs:240-243` |
| T-M5 | No `regen_summary` test verifying `[[catalogs]]`/`harnesses` survive the toml_edit patch. | `tests/workspace_regen_summary.rs:118-140` |
| T-M6 | `workspace_list` never exercises non-zero `bound_projects` or `enabled_plugins` counts. | `tests/workspace_list.rs:62-89` |

### Security audit (4)

| # | One-line | File ref |
|---|---|---|
| S2-M1 | Mode preservation (S-M3 from US1.d-2a) NOT carried over to `catalog::store::write_atomic`; US2 writers (init/rename/regen/sync) silently stomp 0644 → 0600. | `src/catalog/store.rs:81-97` + 4 US2 callers |
| S2-M2 | Symlink refusal NOT extended to `catalog::store::write_atomic`; US2 writers can have their target replaced via TOCTOU symlink race (mitigated by `rename(2)` no-follow). | same as S2-M1 |
| S2-M3 | Unbounded reads on workspace-owned + project-marker files (settings.toml, RULES.md, config.toml). Inherits US1.d-2a S-M1 deferral. | `src/workspace/{regen_summary,sync}.rs` |
| S2-M4 | `rename` recovery branch (`old_dir.exists() == false`) creates central workspace dir at 0o755 instead of 0o700. | `src/workspace/rename.rs:269-274` |

---

## Verdict

US2 surface is solid in shape; the four blockers are real divergences across the contract + test spectrum. Highest-priority fixes:
1. **C-B1** — per-project effective list in remove cascade (~15-30 LOC).
2. **C-B2 + R-M2** — open transaction before marker rewrite loop in rename (~3 LOC).
3. **C-B3** — emit bare array in workspace list --json (~3 LOC).
4. **T-B1 + C-M3 + R-M4** — toml_edit for marker rewrite (preserves harnesses field + side-steps escape bugs).
5. **S2-M1 + S2-M2** — lift mode preservation + symlink refusal into `catalog::store::write_atomic` (single source).
6. **S2-M4** — chmod 0o700 in rename recovery branch (3 LOC).
7. **C-M1, C-M2, C-M4, C-M5, C-Mn4** — small contract alignments.
8. **T-M1, T-M2, T-M3, T-M4, T-M5, T-M6** — test gap coverage.

Plan: ship as PR US2.d-1 (fixes); closeout in US2.d-2 (sdd:map + retro + CLAUDE.md).
