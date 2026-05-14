# Review disposition — pre-Phase-10

## Shipped

Four PRs landed before starting Phase 10:

- **PR #34** — `fix(catalog): report real per-plugin skills_dropped in cascade`
  - Closes contract-audit blocker (cascade `skills_dropped` was attributing the catalog total to the first plugin and zero to the rest).
  - `lifecycle::cascade_disable_for_catalog` now returns `Vec<(String, u32)>`.
  - Tightened `tests/catalog_remove_cascade.rs` to assert the real per-plugin count.
- **PR #35** — `docs(spec): reconcile Phase 2 contracts with shipped behaviour`
  - Closes the bulk of the contract-audit findings (~15 spec/code divergences). Every change moves the spec toward the implementation; zero code changes.
  - Touched: `catalog-extensions.md`, `query.md`, `plugin-commands.md`, `status.md`, `exit-codes.md`, `models-commands.md`, `reindex.md`.
- **PR #36** — `fix(security): scrub catalog URL on add, chmod config 0600, ignore harness state`
  - Closes the three security-audit majors. `tome catalog add` now scrubs the resolved URL before persisting; `config.toml` is chmod 0600 on Unix; `.gitignore` extended for harness regen state.
  - Widened the `URL_LOGIN` scrub regex to cover any RFC-3986 scheme (was `https?://` only).
- **PR #37** — `refactor: tidy small code-review findings`
  - Five small refactors: parens in `status::classify`, surfaced `query_row` errors as `integrity_ok = false`, removed three `_force_*` unused-import idioms (plus an orphan `Rfc3339` import), and promoted `human_mb` from two duplicated definitions to `presentation::format`.

## Deferred to Phase 10

- **T193 / T194 / T195 / T196** — concurrency tests, schema-migration tests, exit-code E2E coverage, model-download interrupt. All four were on the existing Phase 10 plan.
- **Phase 2 `TomeError` Display tests** — `tests/error_messages.rs` covers only `ManifestInvalid` today; ~10 new Phase 2 variants need Display coverage. Folds into Phase 10 test-polish.
- **ModelManifest strictness grep guard** — `tests/manifest_strictness.rs` checks `catalog/manifest.rs` and `config.rs` but not `embedding/registry.rs`. Folds into Phase 10 test-polish.
- **CLI-binary JSON-envelope schemas** — `catalog update`, `reindex`, `models list` are tested at the library boundary but their CLI-binary `--json` envelopes are not schema-checked end-to-end. Folds into Phase 10 test-polish.
- **`tome query --strict` + drift + `--no-rerank` coverage** — current `tests/query.rs` exercises only the library API; needs CLI-binary or library-API coverage of the flag-driven exit codes (40/41/42). Likely needs a `query::run_with_deps`-style library entry point (precedent: `commands::reindex::run_with_deps`).
- **Read-only DB open refactor** — security-audit major #3. The current code opens read-write everywhere and never actually writes outside the lifecycle paths. PR-035 dropped the `writer_pid` claim from the status contract, lowering the urgency. Plumb `OpenFlags::SQLITE_OPEN_READ_ONLY` through `commands::plugin::open_index_for_read` and `commands::status::*` in Phase 10.
- **`tome show` `Last updated:` git-log integration** — currently surfaces only the author. PR-035 updated the spec to document the "not yet available" state; the integration is a post-v0.2.0 candidate.

## Wontfix / followups

- **`commands::catalog::update::count_commits_between` shell-out bypassing `Git::run`** — stdout is a parsed integer, stderr discarded. Currently safe; no scrub bypass risk in practice.
- **All "nit" findings** — applied opportunistically where adjacent to other work (e.g. PR-037 captured the `_force_*` nits). The rest stay unactioned.

## Aggregate

- Review surfaced 8 blocker-label findings; 4 were genuine pre-Phase-10 work (shipped in PRs #34, #36; ½ each), 4 were Phase 10 task-list items that the reviewer flagged as blockers but the plan already owned.
- 32 major findings; ~10 shipped in PRs #34–#37 (split across the four PRs as the natural theming allowed), ~22 deferred to Phase 10 polish slices.
- 35 minor + 14 nit; folded opportunistically into the same PRs (PR-035 doc reconciliation absorbed a lot of these) or deferred to Phase 10.

Phase 2 review pass complete. Phase 10 begins.
