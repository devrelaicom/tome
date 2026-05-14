# Phase 2 review — triage

Source reports: `contract-audit.md`, `code-review.md`, `test-audit.md`, `security-audit.md`.

## Aggregate counts

| Reviewer | blocker | major | minor | nit |
|---|---:|---:|---:|---:|
| contract | 4 | 7 | 9 | 3 |
| code | 0 | 3 | 8 | 4 |
| test | 4 | 18 | 14 | 5 |
| security | 0 | 4 | 4 | 2 |
| **total raw** | **8** | **32** | **35** | **14** |

After deduplication and rule-mapping (test-audit's "blockers" = Phase 10 tasks T193–T196, already enumerated), the real pre-Phase-10 disposition shrinks substantially.

## Disposition rule

Per handover:
- **blocker** (real) → dedicated PR before Phase 10. Wrong contract, broken invariant, or credential issue.
- **major** → dedicated PR before Phase 10. Missing test for shipped behaviour, public-surface mis-scoped, perf regression.
- **minor** → fold into a Phase 10 task.
- **nit** → wontfix unless trivial-while-nearby.

A test gap that already has a Phase 10 task (T193 concurrency, T194 schema migrations, T195 exit codes, T196 atomicity) is **not** a pre-Phase-10 blocker even if a reviewer flagged it as such — the task captures it.

## PR plan (pre-Phase 10)

| PR | Theme | Findings folded in |
|---|---|---|
| #034 | `feat(catalog): cascade reports real per-plugin skills_dropped` | code-review §"Other findings" #1 (major), contract-audit §catalog remove blocker #2, test-audit §catalog_remove_cascade minor #1 |
| #035 | `docs(spec): reconcile contracts with shipped behaviour` | contract-audit blockers #1, #3, #4 (catalog-remove envelope, catalog-update envelope, query no-prompt); contract-audit majors (catalog-update auto_disabled / output sample, models-download spinner, reindex progress bar, status writer_pid removal, status MiB); plus 7 minor wording drifts |
| #036 | `fix(security): scrub catalog-add URL, tighten config perms, gitignore harden` | security-audit majors #1, #2, #4; test-audit catalog_add scrubbing minor; test-audit scrubbing.rs CLI-binary minor (folded as test inside this PR) |
| #037 | `refactor: open index read-only on read paths + tidy unused-import idioms` | security-audit major #3; code-review §7 minor (--version pre-parse defensive scan), §9 nits ×3 (`_force_*` idioms), §"Other findings" minor parens for `status::classify`, code-review §"Other findings" minor `status::check_index` swallowed errors. Combines tightly into one ~250-line refactor. |

Phase 10 still owns:
- T193 concurrency (test-audit blocker #2)
- T194 schema migrations (test-audit blocker #3)
- T195 exit-code coverage E2E (test-audit blocker #1)
- T196 atomicity interrupt (test-audit blocker #4)
- Phase 2 error-message Display tests (test-audit major)
- Model-manifest strictness grep guard + negative test (test-audit major)
- CLI-binary JSON-envelope schema for `catalog update`, `reindex`, `models list` (test-audit major)
- `tome query --strict`, drift, `--no-rerank` coverage (test-audit major; recommends new library entry point)
- Remaining minors and nits captured in the docs/README/CHANGELOG slices of Phase 10

## Wontfix / defer (with rationale)

- **`tome query` library entry point** — proposed by test-audit to enable `--strict` / drift testing. The query command already has reasonable library coverage (`knn`, `StubReranker`) and a stale-skin CLI handler wrapping them. Refactor to `query::run_with_deps` is reasonable but isn't on the Phase 10 task list; the missing CLI coverage is real but not a blocker. **Fold into a Phase 10 task instead of pre-Phase-10 PR.**
- **`tome show` last-updated git-log line** — contract-audit minor. Documented as a follow-up; defer to a post-v0.2.0 task. Remove the line until then OR update the contract to mark it "(not yet available)" — folding into PR-035.
- **`tome status writer_pid` field** — both contract-audit and security-audit flagged it. The cheapest path is to drop the claim from the contract (PR-035) and revisit when concurrency.rs (T193) lands. If T193 surfaces a strong need, add the field as a Phase 10 extension.
- **`tests/concurrency.rs` two-process behaviour for `writer_pid`** — depends on the decision above; revisit during Phase 10 T193.
- **`commands::catalog::update::count_commits_between` shell-out bypassing `Git::run`** — code-review minor §5. Currently safe (stdout is an integer, stderr discarded). Fold the documentation comment into PR-035 or skip.
- **All "nit" findings.** Apply opportunistically while nearby; do not create dedicated work.

## Disposition summary

- 1 PR for the real code blocker (cascade per-plugin counts).
- 1 PR for contract-doc reconciliation (replaces the bulk of "blocker"-style contract drift findings — most divergences resolve by updating the spec to match shipped behaviour, per the reviewers' own recommendations).
- 1 PR for security hardening (3 majors at once — small, themed).
- 1 PR for the read-only DB open + small code-review cleanups.
- All other findings fold into Phase 10 tasks (T192–T212).

Then Phase 10 closes the project.
