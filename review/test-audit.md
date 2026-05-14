# Test-quality audit

Reviewed: 34 test files under `tests/` plus `tests/common/mod.rs` against Phase 2 contracts in `specs/002-phase-2-plugins-index/contracts/*.md` and retros P2–P9.

## Summary

| Severity | Count |
|---|---|
| blocker | 4 |
| major | 18 |
| minor | 14 |
| nit | 5 |

**Top blockers:**
1. **`tests/exit_codes.rs` is unit-level only.** No end-to-end exit-code coverage for the new Phase 2 codes 22, 23, 32, 33, 34, 35, 36, 37, 40, 41, 42, 50, 51, 52. The exhaustive enum→int compile check is in place, but no *integration* test verifies the binary actually exits with these codes for the shipped commands. T195 needs to enumerate these.
2. **No `tests/concurrency.rs` exists.** `tests/index_lock.rs` only exercises intra-process two-fd contention. The CLAUDE.md / TESTING.md design calls for two-process index contention (FR-040 / NFR-001 cross-cut). T193 is real, not a documentation drift.
3. **No `tests/schema_migrations.rs` exists.** Forward-only migration logic in `src/index/migrations.rs` is exercised by the bootstrap path only. T194 is real.
4. **Model-download interrupt (FR-020a / FR-053) is not tested.** `tests/atomicity.rs` is Phase 1; `tests/atomicity_enable.rs` covers the enable rollback only. `tests/model_download.rs` explicitly disclaims interrupt coverage. T196 is real.

## Pattern observations (apply broadly)

- **Loose matches on parameterised error variants.** `assert!(matches!(err, TomeError::ModelMissing { .. }))` in `plugin_enable.rs:332` passes regardless of which model is missing — should bind the `model` field. Same pattern in several places.
- **Phase 2 error-message format coverage is missing.** `tests/error_messages.rs` covers only `ManifestInvalid` variants. Every new `TomeError` variant with structured fields (`PluginManifestParseError`, `SkillFrontmatterParseError`, `ModelChecksumMismatch`, `EmbedderNameDrift`, `EmbedderVersionDrift`, `CatalogHasEnabledPlugins`) ships without Display-format assertions.
- **The grep guard in `tests/manifest_strictness.rs` checks only `src/catalog/manifest.rs` and `src/config.rs`.** It does NOT cover `src/embedding/registry.rs` where `ModelManifest` lives — the P2 retro called out "model `manifest.json` is strict" but the test guard is silent on it. There is no negative test that an unknown field in a model `manifest.json` is rejected (only the strictness attribute exists in source).
- **`catalog_update.rs` is Phase-1-only.** Phase 2 envelope additions (`plugin_changes`, `auto_disabled` per `contracts/catalog-extensions.md`) are only tested via the library API in `catalog_update_reindex.rs` — no integration test asserts the CLI binary emits the documented JSON shape.

---

## Per-file audit

### tests/atomicity.rs

**Covered:** Phase 1 `write_atomic`/`save` atomicity — successful writes, no temp file leaks, concurrent writers don't tear, missing parent dirs are created.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| blocker | Phase 2 atomicity scenarios entirely absent. No model-download interrupt test (FR-020a / FR-053), no enable-mid-pipeline SIGINT test (FR-053). | `models-commands.md` "Atomicity" + research §atomicity | Add `models_download_cleans_partial_dir_on_interrupt` that fires SIGINT mid-stream OR drives `CANCELLED` flip with controlled timing; add `enable_rollback_on_signal` complementing `atomicity_enable.rs`. T196. |
| nit | File name overlaps semantically with `atomicity_enable.rs` — could become `phase1_atomicity.rs` or merge with extensions. | — | Rename or fold into one. |

---

### tests/atomicity_enable.rs

**Covered:** FR-004 rollback for `lifecycle::enable` via `StubEmbedder::with_force_fail_after(n)`. Tight binding on `EmbeddingGenerationFailure { input_desc, detail }`. Tests rollback invariant *and* call-count exceeds threshold (guards against early-bail regression). Second test verifies a clean enable after a failed one.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No SIGINT-equivalent failure injection; only embedder failure is exercised. Frontmatter mid-stream failures, lock-loss mid-stream not covered. | `plugin-commands.md` "Atomicity" | Add a `failing_skill_walk_rolls_back` variant. |
| minor | No assertion that the index lock was *released* after the failure. | FR-040 | Reopen-and-acquire after the failed enable. |

---

### tests/catalog_add.rs

**Covered:** Happy path, name override, duplicate exit 4, duplicate display name exit 4, missing manifest exit 5, fatal-git exit 6. Tight `Some(N)` exit-code asserts throughout.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No coverage of model-URL scrubbing in catalog-add output (Phase 2 retro says scrubbing extended to model URLs and reqwest errors). | P2 retro | A test that an upstream URL containing userinfo is scrubbed in `tome catalog add` failure output. |

---

### tests/catalog_list.rs / tests/catalog_show.rs

**Covered:** Phase 1 enumeration + show. Exit codes appear sound.

**Gaps:** None vs Phase 1. (Not in Phase 2 scope.)

---

### tests/catalog_remove.rs

**Covered:** Happy path human + JSON, non-TTY without `--force` exits 2 (Phase 1 behaviour), unregistered exit 3, missing cache succeeds.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | `non_tty_without_force_exits_2` asserts exit **2** (`Usage`). Phase 2 added exit **54** (`NotATerminal`) which is what new commands use (`plugin disable`, `models remove`). This inconsistency is intentional Phase 1 carry-over but isn't documented anywhere. | `exit-codes.md` 54 vs Phase 1 doctrine | Either: (a) migrate this to exit 54 for consistency and update test; or (b) add an explanatory comment + spec note that catalog remove is grandfathered. Recommend (a) for the v0.2.0 cut. |

---

### tests/catalog_remove_cascade.rs

**Covered:** Exit 53 refuse + cascade JSON `removed.cascade[]` envelope + Phase 1 behaviour preserved when no enabled plugins. Tight `Some(53)`. JSON schema-checks `cascade[0].plugin` and `removed.name`.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | `cascade[0]["skills_dropped"]` from `contracts/catalog-extensions.md` not asserted. | catalog-extensions.md sample JSON | Add assertion on `skills_dropped` count matching pre-enable row count (e.g. `4`). |
| minor | Refuse path doesn't assert the error message format (FR-023 actionability). Stderr substring `"sample-plugin-catalog/plugin-alpha"` is asserted; the directive "Disable them first… or pass --force to cascade" isn't. | catalog-extensions.md sample stderr | Add stderr-contains for "--force to cascade". |

---

### tests/catalog_update.rs

**Covered:** Phase 1 update — up-to-date, unregistered exit 3, SHA-pinned no-op, alphabetical refresh.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | No Phase 2 envelope test. The CLI binary's `--json` extended envelope (`plugin_changes`, `auto_disabled`) is documented in `catalog-extensions.md` but never asserted end-to-end via the binary. Library-level coverage is in `catalog_update_reindex.rs`. | catalog-extensions.md | Add at least one test that runs `catalog update --json` after enabling a plugin via library API, then mutates upstream and asserts the JSON envelope shape (key presence). |

---

### tests/catalog_update_reindex.rs

**Covered:** Library API `reindex_catalog_plugins` — cheap-skip via `embedder.call_count() == baseline` invariant; auto-disable on `PluginNotFound`; unchanged-skill zero-embed. Three solid tests.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | Only "modified" diff class tested. Added/Removed skill paths are not exercised separately (added-only fixture, removed-only fixture). | catalog-extensions.md step 3 | Add `reindex_inserts_added_skill_and_drops_removed_skill`. |
| minor | `summary.added` and `summary.removed` assertion always `== 0` in current tests; no positive case. | data-model §plugin-changes | Use a fixture that adds/removes a SKILL.md between baseline and reindex. |
| minor | Auto-disable reason matched as `contains("missing") || contains("malformed")`; should bind one variant or use a `match`. | FR-033 | Tighten to test both variants in separate tests. |

---

### tests/embedding_stub.rs

**Covered:** R10 properties of `StubEmbedder` — length 384, determinism, distinguishability, L2 normalisation; `StubReranker` identity order + `ReverseStubReranker` flip.

**Gaps:** None.

---

### tests/error_messages.rs

**Covered:** `ManifestInvalid` variants only — each Display includes file + key/value.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | Zero coverage of Phase 2 `TomeError` Display messages. Variants with structured fields (`PluginManifestParseError { file, message }`, `SkillFrontmatterParseError { file, message }`, `ModelChecksumMismatch { model, expected, got }`, `EmbedderNameDrift { stored, configured }`, `EmbedderVersionDrift { stored, configured }`, `CatalogHasEnabledPlugins { catalog, plugins }`, `ModelCorrupt`, `ModelRegistrationParseError`) ship without Display-format tests. | FR-023 (actionability) for Phase 2 | Extend with one assertion per variant: Display contains file path, key field, and the "what to do next" verb. |

---

### tests/exit_codes.rs

**Covered:** Unit-level: every `TomeError` variant gets `.exit_code()` and `.category()` checked; exhaustive `match` enforces compile-time closure of the enum; pairwise-unique-code guard.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| blocker | NO end-to-end coverage of the new Phase 2 exit codes via the binary. The codes verified end-to-end by existing tests are: 2 (`catalog_remove`, `plugin_show`, `models_remove`, `reindex`), 3 (`catalog_*`, `plugin_*`, `reindex`), 20 (`plugin_show`), 21 (`plugin_disable`, `plugin_repeated`), 30 (`models_remove`), 53 (`catalog_remove_cascade`), 54 (`plugin_disable`, `models_remove`, `plugin_interactive`), 1 (`status`), 0. **NOT verified end-to-end** via the binary: **22, 23, 32, 33, 34, 35, 36, 37, 40, 41, 42, 50, 51, 52**. T195 needs to enumerate these. | exit-codes.md, every contract's Errors table | One CLI-binary test per missing code, ideally consolidated as `tests/exit_codes_e2e.rs`: e.g. malformed plugin.json → 22; malformed SKILL.md delimiters → 23; checksum-mismatched model → 32; index busy (two-process) → 50; integrity-check failure (corrupt DB) → 51; embedder-drift query → 41; `--strict --min-score 999` → 40. |
| minor | `Internal(_)` mapping to exit 1 isn't tested at the binary boundary. | exit-codes.md | Trigger via a contrived bug-injection or accept the gap. |

---

### tests/frontmatter.rs

**Covered:** FR-013c delimiter-vs-yaml-body distinction; FR-011 name fallback; FR-012 description fallback (incl. 500-char cap, char-not-byte counting); CRLF, BOM, empty yaml block, extra fields tolerance. Table-driven, exhaustive.

**Gaps:** None worth flagging.

---

### tests/index_lock.rs

**Covered:** Intra-process two-fd `acquire` contention returns `IndexBusy`; drop releases; explicit release; pre-existing lockfile is idempotent. Tight `TomeError::IndexBusy` match.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| blocker | **No two-process contention test.** This is what `concurrency.rs` was supposed to be per CLAUDE.md. FR-040 mandates cross-process lock semantics, and the same-fd vs different-fd OFD/flock semantics differ across platforms. | FR-040, research §R2 | T193 — spawn two `tome reindex --force` or `plugin enable` subprocesses sharing the same data dir, assert one exits 50. Use `std::process::Command::spawn` + busy-wait. |

---

### tests/index_schema_bootstrap.rs

**Covered:** Schema version meta row, all tables + indexes present, meta seeded with embedder/reranker, vec extension reachable, `skill_embeddings` accepts 384-dim insert, reopen no-op, `SchemaTooNew` refused (exit 52 path, library level).

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| blocker | No `tests/schema_migrations.rs`. Forward-only migration logic in `src/index/migrations.rs` has no migration-step tests — there's currently only one schema version, but the framework needs at least one no-op + one synthetic-bump test that documents the forward-only contract. | research §schema-migration, T194 | Add bootstrap-at-N, mutate-to-(N-1)-with-deletes-elided-or-skipped, reopen, assert N restored. |
| minor | `schema_too_new_is_refused` is library-level only. End-to-end CLI exit 52 untested. | exit-codes.md 52 | One CLI test (synthetic bump via tempdir + sqlite-cli or rusqlite probe) → `tome status --json` exits 1 and report identifies. |

---

### tests/manifest_strictness.rs

**Covered:** Grep guard on `src/catalog/manifest.rs` + `src/config.rs`; full bad-manifest corpus (unknown top-level/owner/plugin field, missing fields, invalid semver, invalid email, duplicate plugin, malformed TOML); config corpus.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | The grep guard does NOT cover `src/embedding/registry.rs` where `ModelManifest` lives. P2 retro flagged that model `manifest.json` is strict (`#[serde(deny_unknown_fields)]`) but there is no test. The strictness attribute could silently regress. | P2 retro "model manifest is strict"; FR-013a strictness boundary | Add `model_manifest_module_structs_all_carry_deny_unknown_fields("src/embedding/registry.rs")`, plus a negative round-trip test: serialise a `ModelManifest`, inject an unknown field, assert parse fails with the expected `ModelRegistrationParseError` (exit 33). |
| minor | No positive test for `serde(deny_unknown_fields)` on `Paths`-related serde structs in `src/paths.rs` (if any). | — | Confirm coverage. |

---

### tests/model_download.rs

**Covered:** Library `download_model` happy path + manifest write, checksum mismatch (tight `ModelChecksumMismatch { model, .. }` match), HTTP 404 (looser `matches!(err, Io(_))`), placeholder-checksum refused with tight `ModelCorrupt { model, detail }`. Hand-rolled `TcpListener` HTTP fixture.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| blocker | **No interrupt safety test.** File header explicitly disclaims: "Interrupt safety (FR-053) is not exercised here". P2 retro called out the cleanup bug T057 uncovered. FR-020a / FR-053 explicitly require partial-dir cleanup on interrupt. | FR-020a, FR-053 | Add interrupt test using a slow-trickle TCP server: send 50% of `Content-Length`, sleep, then return. Use a thread to flip `catalog::git::CANCELLED` (guarded by a global mutex shared across the test binary). Assert partial dir is cleaned and final dir absent. |
| minor | `http_error_status_aborts_and_cleans_partial_dir` matches `Io(_)` loosely. The wrapped reqwest error chain isn't asserted (substring check for "404" or HTTP status code). | — | Tighten with a substring match on the error message + assert scrubbing didn't munge the status code line. |
| minor | No size-mismatch test (`size != size_bytes` but checksum matches — impossible in practice but explicit per contract). | models-commands.md step 2 | Add or document as unreachable. |

---

### tests/models_download.rs

**Covered:** CLI binary skip-path with `fabricate_all_installed_models`; JSON envelope `models[].action == "skipped"` + `duration_ms == 0`; fresh layout reports `missing` via `list` cross-check.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | No CLI test for `--force` re-download path or actual download (CI cost rationale acknowledged). | models-commands.md step 1 | Use the same one-shot `TcpListener` trick from `model_download.rs` with an injected `MODEL_REGISTRY` substitute. Currently the registry is a const, so this requires either a test-only `MODEL_REGISTRY` override or library-level test of the `models download` CLI handler (not `download_model`). |
| minor | JSON envelope schema only spot-checks `models[].action` and `duration_ms`. Doesn't verify `sha256_verified`, `size_bytes`, `kind`. | models-commands.md JSON sample | Add per-record schema check: `kind ∈ {embedder, reranker}`, `size_bytes > 0`, `sha256_verified: true`. |
| minor | No test for partial-failure: one model downloads, the other 404s. Should the JSON envelope still record both? | models-commands.md | Spec clarification + test or document. |

---

### tests/models_list.rs

**Covered:** Three states — `missing`, `ok` (cheap probe), `checksum_mismatched` (`--verify`). Sparse-file fixture pattern.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | The `corrupt` state (files referenced by manifest are missing or have wrong sizes) is not tested. | models-commands.md state enum | Fabricate manifest then delete a referenced file or truncate to wrong size; assert `state == "corrupt"`. |
| minor | JSON shape only asserts `name` + `state`. Other documented fields (`version`, `kind`, `size`, `path`, `licence`) not validated. | models-commands.md JSON | Add schema check on at least one record. |
| nit | "Only stage the embedder" comment in `list_with_verify_flips_tampered_artefact_to_checksum_mismatched` — good rationale, keep. | — | — |

---

### tests/models_remove.rs

**Covered:** Unknown model exit 2, uninstalled exit 30, non-TTY without `--force` exit 54, force happy path. Tight `Some(N)` exits, stderr-contains for `--force` pointer.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No coverage of the `embedder removed → tome status reports unhealthy` cross-cut documented in `contracts/models-commands.md` "If the removed model is the embedder…" | models-commands.md tail paragraph | Compound test: remove embedder, assert `tome status` exits 1 and report identifies. |
| minor | "unknown model" stderr substring check (`stderr.contains("unknown model")`) is brittle. | — | Use a more stable phrase or test via `--json` error envelope if available. |

---

### tests/path_validation.rs

**Covered:** URL/file://, SSH, absolute Unix, Windows drive, `..` syntactic, embedded traversal, symlink escape, unresolvable, happy. Tight `matches!(err, ManifestInvalid::*)` per case + Display content checks.

**Gaps:** None.

---

### tests/paths_phase2.rs

**Covered:** `Paths::resolve` with XDG + default fallback, `model_path` join + traversal rejection. Uses a `Mutex<()>` `ENV_LOCK` for serialised env mutation — good.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| nit | The `ENV_LOCK` is local to this file. If any other file in the test binary also mutates env vars, the lock won't help. Currently no other file does — but a comment to that effect or promotion to `tests/common/mod.rs` would harden the invariant. | — | Move the env-guard helpers + `ENV_LOCK` to `tests/common/mod.rs` and reference from any future env-touching test. |

---

### tests/plugin_disable.rs

**Covered:** FR-005 row retention via JSON `skills_retained`, FR-007 / FR-051 non-TTY refuse → exit 54 + pointer message, FR-008 second-disable exit 21. Tight `Some(54)` / `Some(21)`. JSON record schema-checks `plugin`/`status`/`skills_retained`.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No test for unknown-plugin exit 20 on disable (covered by `plugin_show.rs` but not on disable specifically). | plugin-commands.md disable step 1 | Add. |
| minor | No test for unknown-catalog exit 3 on disable. | plugin-commands.md | Add. |
| minor | JSON status field is strict-equal `"disabled"` — but the human banner says "Disabled <id> (12 skill records retained)"; no test for human output. Acceptable. | — | — |

---

### tests/plugin_enable.rs

**Covered:** Skill row insertion + content_hash + enabled flag + plugin_version propagation; FR-011 name fallback; FR-012 description fallback warnings; FR-013c skip warning; idempotency (`PluginAlreadyInState`, tight match); nested-source resolver regression; cheap re-enable via `call_count()`; missing-models error.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | `enable_returns_model_missing_when_no_models_on_disk` uses loose `matches!(err, TomeError::ModelMissing { .. })`. Should bind `model` and assert it's one of the registry names. | plugin-commands.md step 4 | Tighten: `match err { ModelMissing { model } if model == "bge-small-en-v1.5" || model == "bge-reranker-base" => ... }`. |
| minor | No test of "embedder model present, reranker missing" → does enable proceed? (Contract is silent; verify intent.) | plugin-commands.md step 4 + FR-006 | Add or document. |
| nit | `outcome.warnings` substring checks are brittle. If the message wording changes, tests break. Acceptable as a smoke check. | — | — |

---

### tests/plugin_interactive.rs

**Covered:** Full scripted pty session — catalog selector → plugin browser → plugin view → action → confirm → redraw → Back → Back → Quit; exit 0; DB state flipped. Non-TTY refuse exit 54 with pointer-message content checks + bounded-time assertion.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No EOF/SIGINT mid-flow test (contract: "Exit cleanly on Quit / EOF / SIGINT"). | plugin-commands.md interactive §3 | Add: send `\x04` (Ctrl-D) at the catalog selector, assert exit 0 with clean shutdown. |
| minor | No coverage of "Enable" verb inside the interactive loop because the CLI's enable path constructs `FastembedEmbedder`. Doc-comment acknowledges this. | plugin-commands.md interactive §"On Enable" | Document as deliberate gap or factor enable verb to accept an injected embedder for tests. |
| nit | Brittle prompt strings — comment in the test acknowledges. | — | Acceptable. |

---

### tests/plugin_list.rs

**Covered:** Both plugins + correct status, `--enabled-only`, `--catalog` filter narrows, unknown catalog filter exits 3, empty config no records. Tight `Some(3)` and JSON schema-checks on `status`, `version`, `id.plugin`, `id.catalog`.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | `unindexable` status (from the human-output sample in `plugin-commands.md` §plugin list) — i.e. a plugin whose `plugin.json` is malformed appearing in the list — isn't tested. | plugin-commands.md table sample row 3 | Add a fixture plugin with malformed plugin.json and assert it surfaces as `unindexable` rather than crashing. |
| minor | Sort-order asserted only via "alpha + beta" — not via three plugins to confirm strict catalog-asc-then-plugin-asc ordering. | plugin-commands.md sort rules | Acceptable but worth a comment. |

---

### tests/plugin_repeated.rs

**Covered:** Enable-of-enabled (library API, tight `PluginAlreadyInState { state, plugin }` match), disable-of-disabled (CLI, `Some(21)`).

**Gaps:** None — consolidation is exactly what was wanted.

---

### tests/plugin_show.rs

**Covered:** Full metadata + component counts, FR-013a lenient tolerance of unknown fields, unknown plugin exit 20, unknown catalog exit 3, malformed id exit 2. Tight `Some(N)` throughout.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No exit-22 (`PluginManifestParseError`) test — a plugin with malformed `plugin.json`. | plugin-commands.md show errors | Add fixture + assert `Some(22)`. |
| minor | Component count assertion is `>= 4` rather than strict-equal. Loose. | plugin-commands.md component breakdown | Tighten to `== 5` (including the malformed-yaml-body skill which still counts as a directory). |

---

### tests/query.rs

**Covered:** Self-similarity top-1, catalog+plugin filter narrowing, unknown catalog filter → empty, stub reranker identity order + scoring, reverse stub reranker flips order, wrong-length vector → tight `IndexIntegrityCheckFailure`.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| major | **`--strict` + `--min-score` path not tested.** Exit 40 (`QueryNoResultsStrict`) has zero coverage. | query.md flag table + Errors | Add: enable, query with `--strict --min-score 999`, assert exit 40. Note this requires `commands::query` to expose a library entry point analogous to `reindex::run_with_deps` OR to load real models in CLI (probably needs a library bypass). |
| major | **Drift exits 41/42 not tested at any layer.** `status.rs` tests *report* drift; no test that `tome query` itself refuses with 41/42 when stored meta disagrees. | query.md "Embedder drift handling" | Library-level test (analogous to status's `write_meta` then re-open and assert). May need `commands::query` library entry point. |
| major | `--no-rerank` `scoring: "embedding-similarity"` JSON field not tested. | query.md output sample | Add. |
| major | `reranker_drift` warning field not tested. | query.md "Reranker drift handling" | Add: mutate `meta.reranker_name`, query, assert warning + non-error exit. |
| minor | `--top-k` flag behaviour not directly tested (default vs explicit). | query.md flag table | Add. |
| minor | `knn_top_one_matches_self_embedded_skill` self-similarity tolerance `1e-4` is fine but worth a comment about expected float drift. | — | Acceptable. |

---

### tests/reindex.rs

**Covered:** Library-API `run_with_deps` for `Scope::All`, `Scope::Plugin`, `Scope::Catalog`; `--force` re-embeds all; zero-change zero-embed invariant via `call_count()`. CLI binary: unknown catalog → 3, malformed id → 2, empty install → 0 with "Nothing to reindex". Tight asserts.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No CLI binary test for unknown plugin (`<catalog>/<plugin>` where catalog is known) → exit 20. | reindex.md errors table | Add. |
| minor | No test for `tome reindex --json` envelope shape (`scope`, `plugins_visited`, `skills_checked`, `skills_re_embedded`, `skills_unchanged`, `duration_ms`). Library tests assert the aggregate struct; CLI binary `--json` output schema isn't asserted. | reindex.md JSON sample | Add a CLI-binary test (library-API setup, then CLI run) that schema-checks the JSON envelope. |
| minor | No skill-frontmatter-malformed-in-scope test (exit 23). | reindex.md errors table | Add. |
| minor | No "FR-016 recovery path" test — reindex --force after embedder drift restores query. Would compound `query.rs` drift coverage. | reindex.md "Notes" final paragraph | Add a cross-cutting drift-then-recovery test. |

---

### tests/scrubbing.rs

**Covered:** Phase 1 URL userinfo, SSH host, kv tokens, long hex, SHA-1 preserved in colon/equals context, clean-pass-through, ordering. Phase 2: AWS presigned URL, HF presigned URL, reqwest-style URL credentials, colon-form signed-URL keys.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | No assertion that `models download` output / errors actually invoke scrubbing in practice. Library-level tests confirm the regex; no end-to-end test that a download error containing userinfo emerges scrubbed from the binary. | P2 retro "scrubbing extended to model URLs and reqwest errors" | Add CLI test: configure a synthetic model URL with userinfo, trigger HTTP 404, assert stderr does NOT contain the userinfo. Mirrors `model_download.rs::http_error_status_aborts` but at the binary boundary. |

---

### tests/status.rs

**Covered:** Library API (`assemble_report`) — healthy with index, healthy no-index, unhealthy embedder missing, degraded reranker-only-missing, degraded reranker drift, unhealthy embedder drift, verify-flag checksum mismatch. CLI binary: exit 0 healthy, exit 1 unhealthy, JSON record. Drift tests `write_meta` to bypass the StubEmbedder identity mismatch — clean pattern.

**Gaps:**

| Severity | Gap | Contract | Suggested fix |
|---|---|---|---|
| minor | The `writer_pid` field documented in `status.md` ("if a writer holds the lock, the report includes a `writer_pid` field") is not tested. | status.md "Notes" | Cross-process test: start a `tome reindex --force` in a subprocess (blocking on a slow embedder), then call `tome status`, assert `writer_pid` present. Couples with the missing `concurrency.rs`. |
| minor | `assert!(report.embedder.state == "ok")` uses a stringly-typed compare. Should be an enum if it isn't already. | — | If the `state` field is `String`, consider a typed enum on the `StatusReport`. |
| minor | No test for index-integrity failure → exit 1 (i.e. seed a corrupt SQLite file, run `tome status`). | status.md step 4 + exit-codes 51 | Add. |
| nit | Verify test rehashes both models (~66 MB) every CI run. Maybe acceptable; sparse-file zero-bytes hash is fast. | — | — |

---

### tests/version_output.rs

**Covered:** Three-line plain text (tome version + embedder + reranker), JSON envelope, flag-order-irrelevant (`--json --version` vs `--version --json`), `-V` short flag. Compile-time pin against `MODEL_REGISTRY`.

**Gaps:** None — this is the model for "compile-time-pinned content".

---

### tests/common/mod.rs

**Covered:** `Fixture`, `ToolEnv`, lifecycle helpers (`lifecycle_paths`, `fabricate_models`, `fabricate_installed_model`, `fabricate_all_installed_models`, `copy_sample_plugin_catalog`, `config_with_catalog`, `write_config_for_cli`), stub seeds, `paths_for`.

**Observations:**

| Severity | Issue | Suggested fix |
|---|---|---|
| nit | `fabricate_models` writes only manifest (used by lifecycle gate); `fabricate_installed_model` / `fabricate_all_installed_models` write manifest + sparse artefacts. P6 retro flagged the trap: using `fabricate_models` where `fabricate_all_installed_models` is needed (and vice versa). **Spot check confirms each test picks correctly.** `plugin_enable.rs` etc. use `fabricate_models` (only need manifest gate); `models_*.rs` and `status.rs` use `fabricate_all_installed_models` (need real files for size + verify probes). No misuse detected. |
| minor | `fabricate_installed_model` writes auxiliary files (`tokenizer.json`, etc.) as 1-byte sparse — fine for existence but `--verify` rehashes the *primary* file only. If verify ever extends to auxiliaries the helper needs updating. | Document or extend. |
| nit | The env-guard utilities live in `paths_phase2.rs` only. If a second env-touching test appears, promote at rule-of-three. | — |

---

## Cross-cutting observations

1. **Library/CLI test boundary is consistently respected.** No test attempts to construct `FastembedEmbedder` from a `ToolEnv.cmd()` subprocess for the heavy-state paths. Boundary is firm.
2. **`call_count()` instrumentation is used correctly** in the four places it matters (`cheap_reenable_after_disable_invokes_embedder_zero_times`, `reindex_pass_unchanged_skills_does_no_embed_work`, `reindex_all_visits_every_enabled_plugin_zero_changes`, `reindex_after_update_re_embeds_only_modified_skill`, `reindex_one_plugin_re_embeds_only_modified_skill`, `reindex_force_re_embeds_every_skill_in_scope`). No misuse.
3. **`CANCELLED` static is not touched by any test.** `atomicity_enable.rs` explicitly documents the avoidance. Good discipline.
4. **JSON schema checking is patchy.** Most tests spot-check 2–3 fields; only `version_output.rs` and `catalog_remove_cascade.rs` schema-check enough fields to call them complete. For v0.2.0, every `--json` output should have at least one test that schema-checks every documented field at least once.
5. **`tests/exit_codes.rs` is the only test enforcing closed-set discipline.** Adding a new variant breaks the compile — that's good. But adding an *unused* variant won't be caught; the exhaustive-match test passes if every variant maps somewhere. Acceptable.

## Recommended Phase 10 task additions (beyond existing T193–T196)

- **T197 (proposed):** Cover Phase 2 `TomeError` Display formats in `tests/error_messages.rs` (10+ variants).
- **T198 (proposed):** Extend `tests/manifest_strictness.rs` grep guard to `src/embedding/registry.rs` and add a negative test for `ModelManifest` strictness (closes the P2 retro item formally).
- **T199 (proposed):** Add CLI-binary JSON-envelope tests for `tome catalog update` Phase 2 extensions, `tome reindex`, `tome models list` (full schema, not spot checks).
- **T200 (proposed):** Add `--strict` + drift + `--no-rerank` coverage for `tome query` (requires either a library entry point or a model-bypass mechanism in the query command).
