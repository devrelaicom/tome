# Contract-vs-implementation audit

Scope: every Phase 2 command in `specs/002-phase-2-plugins-index/contracts/*.md`, the `TomeError` exit-code surface, and the integration tests under `tests/`.

## Summary

- **23 findings:** 4 blocker, 7 major, 9 minor, 3 nit
- Heaviest concentration: `tome catalog remove`/`tome catalog update` JSON envelopes; `tome query` model-download contract divergence; `tome status` missing `writer_pid`.

## Findings

### `tome catalog remove`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| blocker | gap (contract→code) | JSON envelope shape diverges from contract. Contract specifies a flat `{ "catalog": "...", "removed": true, "cascade": [...] }`. Implementation emits `{ "removed": { "name": "...", "url": "...", "cache_path": "...", "cascade": [...] } }` — the boolean `removed: true` is replaced by a nested object, `catalog` becomes `name` inside it, and adds `url` / `cache_path`. | `src/commands/catalog/remove.rs:160-203` | Either adjust the contract to document the actual Phase 1-inherited envelope, or rework `RemovedEnvelope` to match the documented Phase 2 shape. Recommend updating contract — the inherited shape carries more useful detail. |
| blocker | gap (contract→code) | Cascade `skills_dropped` per-plugin counts are not actually per-plugin. The implementation receives only the total from `cascade_disable_for_catalog` and attributes the full `dropped_total` to the first plugin and `0` to every other plugin. The contract example shows distinct per-plugin counts (`12` and `8`). | `src/commands/catalog/remove.rs:92-99` | Change `cascade_disable_for_catalog` to return `Vec<(String, u32)>` (per-plugin breakdown). Update the JSON `CascadeRecord` to carry the real count. Add a test assertion on per-plugin counts > 0. |
| major | hole (contract→test) | The cascade per-plugin `skills_dropped` field is not asserted on; the existing test only checks `cascade[0]["plugin"]`, never `cascade[*]["skills_dropped"]`. The blocker above would have been caught by a single line. | `tests/catalog_remove_cascade.rs:135-148` | Add `assert_eq!(cascade[0]["skills_dropped"], 4)` (the alpha fixture count) once the fix above lands. |
| minor | drift (test→contract) | Contract output example uses `✓` glyph but implementation emits `✓` only on TTY. Pre-existing TTY-aware behaviour; just a contract example clarity issue. | `src/commands/catalog/remove.rs:108-111` | Add a one-line note in `catalog-extensions.md` clarifying the glyph is TTY-only. |

### `tome catalog update`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| blocker | gap (contract→code) | JSON envelope shape diverges. Contract specifies one aggregate JSON object `{ "catalogs_refreshed": [...], "plugin_changes": [...], "auto_disabled": [...] }`. Implementation streams NDJSON envelopes — one `{"refreshed": {...}}` or `{"pinned": {...}}` per catalog and one `{"plugin_change": {...}}` per plugin. Consumers expecting the documented shape will not parse the output. | `src/commands/catalog/update.rs:309-358, 384-453` | Either adjust the contract to document NDJSON-with-typed-envelopes (consistent with other commands), or aggregate into one object before emitting. Recommend updating the contract to NDJSON. |
| major | gap (contract→code) | `auto_disabled` is documented as a separate top-level array; implementation folds it into the same `plugin_change` envelope via `auto_disabled_reason`. Same root cause as the envelope mismatch above but worth a separate fix-or-doc decision. | `src/commands/catalog/update.rs:340-350` | Document the consolidated `plugin_change` shape, OR split into two NDJSON envelope types. |
| major | hole (contract→test) | No test asserts the JSON output shape of `tome catalog update` end-to-end. `tests/catalog_update_reindex.rs` exercises the library API (`reindex_catalog_plugins`) and inspects the `CatalogReindexOutcome` struct, not the JSON envelope. The above blocker is invisible to CI. | `tests/catalog_update_reindex.rs` | Add a CLI-binary test that runs `tome catalog update --json` against a fixture and parses the output. The cascade path is achievable without `FastembedEmbedder` once the lazy-load skip-when-no-enabled path is exercised. |
| minor | gap (contract→code) | Contract output table header is "`Plugin … Added Modified Removed`"; human emission uses `added X · modified Y · removed Z · unchanged W` (no header, single line per plugin). Functionally equivalent; documented example mis-sets expectations. | `src/commands/catalog/update.rs:278-289` | Update contract example to match shipped output, or render a comfy-table to match the contract. |

### `tome plugin enable`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| minor | drift (test→contract) | Contract step 4 documents "TTY (stdout AND stderr): prompt to download". Implementation gates on `stdin_is_tty() && stdout_is_tty()` only — stderr TTY-ness is not consulted. Practically identical, but the contract specifies a different predicate. | `src/commands/plugin/enable.rs:113` | Update contract to "stdin AND stdout TTY", which is the load-bearing condition for `inquire`. |
| minor | undocumented behaviour | The decline path returns `TomeError::Interrupted` (exit 8). Contract says "clean abort" but does not specify the exit code. The `Interrupted` choice is defensible; just document it. | `src/commands/plugin/enable.rs:139` | Add a one-line note to `plugin-commands.md` §"plugin enable" that a declined model download exits 8 (Interrupted). |
| nit | drift (test→contract) | Contract example output includes "Enabling …" banner + check-mark line; implementation matches but uses 1-decimal seconds (`{:.1}s`). Contract example uses `8.4s`. Fine, just nit. | `src/commands/plugin/enable.rs:166-170` | None. |

### `tome plugin disable`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| minor | undocumented behaviour | Decline path returns `Ok(())` (exit 0), unlike enable's decline which returns `Interrupted` (exit 8). Inconsistent across the two prompts. | `src/commands/plugin/disable.rs:53-62` | Pick one convention and apply it to both. Recommend `Ok(0)` for both — no state change is not an error. |
| nit | drift (test→contract) | Contract output uses past tense `Disabled <id>`; implementation emits `Disabling <id>…` banner + `✓ disabled <id> (N skill records retained)` follow-up. Functionally equivalent, slightly chattier. | `src/commands/plugin/disable.rs:68-90` | None. |

### `tome plugin list`

No findings — output shape, sort order, `--catalog`/`--enabled-only` filters, exit codes (3 on unknown catalog) all match the contract and are exercised by `tests/plugin_list.rs`.

### `tome plugin show`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| minor | gap (contract→code) | Contract output includes a "Last updated: 3 days ago — Alice <alice@example.com>" line driven by `last_upstream_change`. Implementation always emits `Last updated: — — <author>` because `last_upstream_change` is hard-coded to `None` (the `git log` integration is a documented follow-up). | `src/commands/plugin/show.rs:58-60, 99-104` | Either remove the line until the git-log integration lands, or surface "—" with a clearer "(not yet available)" hint. Update contract if the line will remain empty for v0.2.0. |

### `tome plugin` (bare interactive)

No findings — TTY refusal with exit 54 plus the documented pointer message is wired and tested via `tests/plugin_interactive.rs` (pty + `rexpect`). Quit / Back / Esc all collapse to clean `Ok(())` per contract.

### `tome models download`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| major | gap (contract→code) | Contract output specifies a determinate byte-progress bar (`[#####…] 100% downloaded · 8.2s`). Implementation uses an indeterminate `indicatif` spinner because `download_model` does not surface a byte-progress callback. Documented as TD-010 in `CONCERNS.md`. | `src/commands/models/download.rs:78-90` | Either land the byte-progress refactor (planned past rule-of-three) or update the contract to specify an indeterminate spinner. |
| nit | hole (contract→test) | The `redownloaded` action value is never asserted on (would require a real network fetch + pre-existing install). Documented as a CI boundary (`tome models download` is not exercised end-to-end against `MODEL_REGISTRY` URLs). Acceptable for now. | `tests/models_download.rs` | None. |

### `tome models list`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| nit | drift (test→contract) | Contract uses `ChecksumMismatched` (PascalCase) in prose, but the JSON `state` value uses snake_case `checksum_mismatched`. The JSON test asserts the snake_case value. | `src/commands/models/mod.rs:50-57`, `tests/models_list.rs:104` | Update contract prose to spell out the JSON spelling (`state = "checksum_mismatched"`). |

### `tome models remove`

No findings — every documented error path (exit 2 unknown, exit 30 not installed, exit 54 non-TTY-without-force, manifest-first deletion) is implemented and tested in `tests/models_remove.rs`.

### `tome query`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| blocker | gap (contract→code) | Contract step 2 specifies: "Load the embedder; if missing → exit 30 (`ModelMissing`) **or download prompt (TTY only)**". Implementation never prompts — `commands::query::run` exits 30 with no TTY branch. The doc comment at the top of `query.rs` explicitly states "No model download is offered here … surfacing a multi-MB download behind a `tome query` is hostile UX." This is a deliberate divergence, but the contract is the spec of record. | `src/commands/query.rs:62-75` | Pick one: (1) implement the TTY-prompt path; (2) update `query.md` step 2 to spell out "non-interactive only — `tome query` never prompts" and point the user at `tome plugin enable` / `tome models download`. Recommend (2). |
| major | hole (contract→test) | No integration test exercises `commands::query::run` directly — `tests/query.rs` only drives the library API (`knn`, `StubReranker`). The following contract behaviours have no test coverage: `--strict` exit 40, embedder-name drift exit 41, embedder-version drift exit 42, `--no-rerank` banner, `threshold_passed` JSON field, `reranker_drift` JSON field, `--catalog`/`--plugin` filter validation (exits 3 / 20). | `tests/query.rs` | Add CLI-binary tests for the drift paths (use the meta-mutation pattern from `tests/status.rs`) and library-API tests for `--strict` / threshold semantics. The query handler itself can be tested by promoting the inner KNN+rerank loop to a `pub fn run_with_deps`-style entry point per the precedent in `commands::reindex`. |
| major | hole (contract→test) | No test for `--top-k N` cap, the 4× candidate-pool expansion when reranking is on, or the score sort order. | `tests/query.rs` | Add a library-API test asserting that `args.top_k.saturating_mul(4)` candidates are pulled from `knn` when a reranker is present. |
| minor | drift (test→contract) | Contract: error message on embedder drift is "Stored vectors were produced by a different embedder. Run `tome reindex --force` to rebuild." Implementation `Display`: "stored vectors were produced by embedder `{stored}`; currently configured embedder is `{configured}`. Run `tome reindex --force` to rebuild the index." Slightly more detailed but not identical. | `src/error.rs:93-105` | Update contract to match the shipped messages (they're more informative). |
| minor | drift (test→contract) | Contract `--no-rerank` banner is documented as "human output" but implementation emits it on stderr. Reasonable choice (keeps stdout byte-stable), just not documented. | `src/commands/query.rs:250-253` | Update contract to specify stderr explicitly. |

### `tome reindex`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| major | gap (contract→code) | Contract output example shows a progress bar: `[#####…] 156/156 skills · 41.2s`. Implementation emits a final summary only (no progress bar). | `src/commands/reindex.rs:239-258` | Either add an `indicatif::ProgressBar` wired to per-skill increments inside `lifecycle::reindex_plugin`, or remove the progress-bar example from the contract. The library boundary makes the progress-bar route non-trivial — recommend updating the contract for v0.2.0. |
| minor | hole (contract→test) | No test asserts the `Reindex unknown plugin` → exit 20 path; only the unknown-catalog (exit 3) and bad-id-format (exit 2) paths are covered in `tests/reindex.rs`. | `tests/reindex.rs:206-223` | Add a CLI-binary test: `reindex sample-plugin-catalog/ghost` after registering the catalog → assert exit 20. |

### `tome status`

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| major | gap (contract→code) | Contract: "Status NEVER takes the advisory lock. If a writer holds it, the report includes a `writer_pid` field." Implementation never inspects the lockfile and never emits a `writer_pid` field in `StatusReport`. | `src/commands/status.rs:73-80, 134-187` | Either implement a non-blocking read of `index.lock` (the lockfile is Tome-owned; read PID from contents) and add the field to `StatusReport`, or remove the `writer_pid` clause from the contract. |
| minor | drift (test→contract) | Contract example uses "MB" suffix ("4.2 MB"). Implementation uses binary suffixes `B`/`KiB`/`MiB` (`4.2 MiB`). | `src/commands/status.rs:336-346` | Pick one — recommend updating contract to "MiB" (closer to what `cargo` and `du` use for binary sizes). |
| minor | hole (contract→test) | No test asserts the `index database: not yet bootstrapped` branch. `status_healthy_with_no_index_yet` checks `report.index.present == false` but does not check the human output line. | `tests/status.rs:73-85` | Capture CLI human output and assert the "not yet bootstrapped" string. |

### `tome --version`

No findings — three-line plain text, JSON envelope (`tome`, `embedder.{name,version}`, `reranker.{name,version}`), `-V` short flag, and `--json --version` order-irrelevance are all implemented and tested in `tests/version_output.rs`.

### Exit-code surface

| Severity | Category | Finding | File:line | Suggested action |
|---|---|---|---|---|
| minor | drift (test→contract) | Contract `exit-codes.md` describes the variant table as starting at code 20 for Phase 2 and references codes 0–8 for Phase 1, but does not document `Internal` (exit code 1). The variant exists in `src/error.rs` and is asserted in `tests/exit_codes.rs`. | `specs/002-phase-2-plugins-index/contracts/exit-codes.md:5-16`, `src/error.rs:139-140` | Add a row to the Phase 1 table for code 1 (`Internal`). |
| minor | drift (test→contract) | Contract claims "The `TomeError` enum is `#[non_exhaustive]` on the consumer side". The enum is **not** marked `#[non_exhaustive]` in `src/error.rs`. The compile-time exhaustiveness guarantee comes from the `impl TomeError::exit_code` arms, not from the attribute. | `specs/002-phase-2-plugins-index/contracts/exit-codes.md:52`, `src/error.rs:9-10` | Remove the `#[non_exhaustive]` claim from the contract — the closed-set guarantee is real but stems from the exhaustive `match` in `exit_code`, not the attribute. |
| nit | gap (contract→code) | `Cargo.toml` claims `rust-version = "1.93"` but `MSRV` policy is not documented in `exit-codes.md`. Not a contract violation, just adjacent context. | n/a | None. |

## Cross-cutting observations

1. **JSON envelope inconsistency.** Tome alternates between three patterns: NDJSON (one envelope per line for `plugin list`, `catalog update`), single-object JSON (`status`, `enable`, `disable`, `query`), and pre-existing Phase 1 envelopes wrapped in a discriminant key (`catalog remove`, `catalog update`). The Phase 2 contracts assume aggregate single-object JSON for the new commands but the implementation often emits NDJSON or inherits Phase 1's nested envelope. A short "JSON output conventions" addendum to the spec would resolve five of the findings above in one move.

2. **CLI-binary test coverage for the query path is thin.** `tome query` is one of the two end-user surfaces (the other being `tome plugin enable`); both are heavily library-tested but lightly CLI-tested. The library boundary is the right choice for the heavy-state paths, but a handful of light-state CLI tests would close real holes (especially around drift, `--strict`, and filter validation).

3. **Documented intentional divergences live in code comments, not the spec.** Two examples: `commands/query.rs` deliberately refuses to prompt; `commands/models/download.rs` deliberately uses a spinner instead of a byte-progress bar. The contracts still claim the original behaviour. Reconciling either by updating the spec is cheaper than retrofitting the implementation; recommend a "divergence appendix" in each contract that calls these out explicitly.
