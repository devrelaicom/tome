# US2 — Disposition

Maps findings in `us2-findings.md` to actions. Three buckets:

- **In PR US2.d-1** — applied as part of the reviewer-pass commit
- **Follow-up issue** — non-blocking, tracked separately
- **Defer to US5 polish** — natural home in the doctor work

## Blockers — all applied in US2.d-1

| # | Disposition |
|---|---|
| C-B1 | Apply: in `remove::teardown_integration_for_project`, instead of iterating `SUPPORTED_HARNESSES` via `with_effective_modules`, read the per-project marker, compute the effective list via `settings::resolver::resolve_effective_list` (using `StubScope::new()` for the workspace registry — same as `harness::sync::sync_project`), then dispatch per-harness teardown only for harnesses in the effective list. Add coverage in `tests/workspace_remove_cascade.rs` for the per-project narrowing. |
| C-B2 + R-M2 | Apply: in `rename::rename`, open `conn.transaction()` BEFORE the per-marker rewrite loop. Markers written successfully but SQL UPDATE failing → no rollback of marker writes possible, but at least the DB stays consistent with `old`. Document the partial-failure mode honestly in the module docstring. |
| C-B3 | Apply: in `commands::workspace::list::emit_json`, emit the bare `Vec<WorkspaceListEntry>` directly instead of the `ListEnvelope` wrapper. Update the byte-stable JSON wire-shape test to match. |
| T-B1 + C-M3 + R-M4 | Apply: use `toml_edit::DocumentMut` for the marker rewrite in `rename`. Read existing marker → parse via `DocumentMut::from_str` → update `workspace = "<new>"` → serialise → atomic-write. Preserves the `harnesses` field + side-steps the hand-rolled escape bugs in `init`. Add a test verifying a marker with `harnesses = ["[workspace]", "!cursor"]` survives the rename intact. |

## Majors — applied in US2.d-1 (11)

| # | Disposition |
|---|---|
| C-M1 | Apply: `init::render_settings_toml` emits `[summaries]` as an empty table. Cheap. |
| C-M2 | Apply: `init` writes `"# No summary yet — run `tome workspace regen-summary <name>` to populate.\n"` to RULES.md (or a similar one-line comment). Update existing test expectation. |
| C-M4 | Apply: make `<name>` REQUIRED on `tome workspace regen-summary`. Drop `Option<String>` → `String`. Update tests. |
| C-M5 | Apply: write `generated_at` as `toml_edit::Datetime` value (unquoted datetime literal), not basic string. |
| C-Mn4 | Apply: `regen_summary` bumps `last_used_at` on the workspaces row per FR-411. |
| C-Mn5 | Apply: move `list_for_workspace` inherited-rows read INSIDE the transaction in `init`. Same advisory lock, more contract-faithful. |
| S2-M1 + S2-M2 | Apply: lift mode preservation + symlink refusal into `catalog::store::write_atomic` (the single atomic-write surface outside `src/harness/`). Add `tests/security_hardening.rs::preserve_file_mode_on_workspace_settings_rewrite` + `…_on_project_marker_config_rewrite` + `…_on_project_marker_rules_rewrite` + `…_refuses_symlink_*` analogues. |
| S2-M4 | Apply: rename recovery branch chmod 0o700 (3 lines). |
| T-M1 | Apply: add JSON wire-shape pins for `InitOutcome`, `RenameOutcome`, `RegenSummaryOutcome`, `RemoveOutcome`. Pattern from `tests/workspace_sync.rs::report_serialises_to_byte_stable_json_for_*`. |
| T-M3 | Apply: extend `tests/workspace_remove_cascade.rs` with: (a) rules-file teardown via `BlockInExistingFile` strategy on a pre-populated `AGENTS.md`; (b) user-owned MCP entry left alone during cascade (per-project effective list narrowing already needed for C-B1). |
| T-M4 | Apply: tighten `FailingSummariser` assertion to `assert!(matches!(err, TomeError::SummariserFailure { kind: SummariserFailureKind::ModelMissing, .. }))`. |
| T-M5 | Apply: extend `regen_summary_writes_settings_and_rules` (or new test) to seed `[[catalogs]]` + `harnesses` in settings.toml before regen, then assert both survive. |
| T-M6 | Apply: extend `tests/workspace_list.rs` with a test that seeds bound projects + enabled skills under one workspace and asserts non-zero counts. |

## Majors — deferred to follow-up issue

| # | Reason for deferral |
|---|---|
| R-M1 | Cleaner SQL DISTINCT — cosmetic. Defer. |
| R-M3 | Init clones too much in FnOnce closure — cosmetic ~1% perf. Defer. |
| R-M5 | Summariser called under advisory lock — performance trade-off; correctness over performance for now. Defer (revisit when LlamaSummariser ships in US4.a — `tome catalog update` may need to coexist). |
| R-M6 | TOCTOU comment in remove — purely defensive, future-refactor concern. Defer. |
| R-M7 | `compute_info` early-return for global-not-found drops fields — cosmetic for the `--json` shape; pinned outcome doesn't include them today. Defer. |
| R-M8 | rename pre-check vs write TOCTOU — small attack surface; doctor surfaces drift. Defer. |
| S2-M3 | Unbounded reads — multi-site change; mirrors US1.d-2a S-M1 deferral. Defer to a dedicated PR before v0.4 cut. |
| T-M2 | Concurrent init/rename/regen tests — pattern established elsewhere; defer to "test hardening" follow-up. |

## Minors + nits

Bulk-deferred per individual reviewer files. Documented in `us2-disposition.md`'s tail (this file).

## Net effect of US2.d-1

- ~9 production-source touches (`cli.rs`, `init.rs`, `rename.rs`, `regen_summary.rs`, `remove.rs`, `commands/workspace/list.rs`, `catalog/store.rs`, `commands/workspace/regen_summary.rs`, `tests/security_hardening.rs`)
- ~10 new tests, 1 modified test (existing `init` empty-RULES.md assertion → new comment)
- 1 contract amendment? — actually no, the disposition above keeps contracts intact; we just align the impl. Mid-flight: C-M1/C-M2/C-M4 changes are impl-side only (the contracts as written are the source of truth).
- 2 review artefacts (`us2-findings.md`, `us2-disposition.md`)

Then US2.d-2 runs `/sdd:map incremental` + retro/CLAUDE.md updates.
