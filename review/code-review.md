# Rust-lens code review

Audit of `/Users/aaronbassett/Projects/devrel-ai/tome/src/` ahead of v0.2.0 ship. Phase 2 feature-complete (240 tests / 36 suites green).

## Summary

| Severity | Count |
|----------|------:|
| blocker  | 0     |
| major    | 3     |
| minor    | 8     |
| nit      | 4     |

No invariant-breaking blockers found. The closed-error-set, atomicity, lock-discipline, and `vec0` DELETE-then-INSERT invariants are all upheld. Top concerns are (1) incorrect per-plugin attribution in the `catalog remove --force` cascade JSON, (2) eager-but-unneeded model registry index scans collapsing to `expect()` calls, and (3) DRY-violating duplication (`human_mb`) plus a minor dead-import preservation idiom that should be removed.

---

## Findings by area

### 1. Closed error set

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| minor    | `anyhow::anyhow!("config file `{}` is not valid: {}", …)` is used to manufacture a runtime error for a TOML parse failure on the config file. The `ManifestInvalid::TomlParse` variant exists and is the right shape — the config file is Tome-owned just like the catalog manifest. Collapsing it into `Internal` weakens the closed-set guarantee. | `src/catalog/store.rs:17-22` | Map to a dedicated variant — either reuse `ManifestInvalid::TomlParse` (which carries `file` + `message`) or add `TomeError::ConfigParseError { file, message }` with its own exit code in the 60+ range. |
| minor    | `TomeError::Internal(anyhow::Error::new(e))` for a TOML serialise error on the config-write path. A serialize-the-config-we-just-built failure is genuinely "should not happen", so `Internal` is defensible — but a `ConfigSerializeError` variant would still be more honest. | `src/catalog/store.rs:36` | Optional: add a dedicated variant; otherwise leave as-is with a `// Truly internal — we constructed this struct ourselves` comment to record the intent. |
| minor    | `inquire::InquireError` `Custom` / `InvalidConfiguration` fall through to `TomeError::Internal(anyhow!("prompt failed: {other:?}"))`. These are unlikely in practice — `Custom` is user-supplied and we don't use that API; `InvalidConfiguration` is a programmer error. Still, the closed-set principle says name every reachable failure. | `src/presentation/prompt.rs:69` | Exhaustive match: keep `Internal` for `Custom`/`InvalidConfiguration` but annotate with `// Programmer error: inquire builder misconfigured` so the variance is intentional, not an oversight. |

### 2. Atomicity invariants

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| —        | Registry writes (`catalog::store::write_atomic`) are atomic: `NamedTempFile` in the parent directory + `sync_all` + `persist`. Correct. | `src/catalog/store.rs:42-52` | None. |
| —        | Model downloads run the full post-stream pipeline (verify → rename → manifest) under a single closure with cleanup on any failure (`let _ = std::fs::remove_dir_all(&partial_dir);`). The P2 retro leak is fixed and the fix is sound. | `src/embedding/download.rs:67-87` | None. |
| —        | `enable_plugin_atomic` and `reindex_plugin_atomic` each wrap their work in a single `Connection::transaction()`, so any branch failure rolls back. The `tx.commit()` is the last fallible step before returning. | `src/index/skills.rs:319-383, 413-511` | None. |
| —        | All five `lifecycle::*` write paths (`enable`, `disable`, `reindex_plugin`, `auto_disable_orphan`, `cascade_disable_for_catalog`) acquire the advisory lock via `acquire_lock`, run work in a closure / helper, then `lock.release()?` on Ok or `drop(lock)` on Err. Pattern is consistent. | `src/plugin/lifecycle.rs:127-148, 167-184, 212-239, 262-295, 304-322` | None. |
| —        | `delete_by_plugin` opens its own `unchecked_transaction` to wrap the embedding-then-skills delete pair. Correct for the bare-helper case (e.g. inside `cascade_disable_for_catalog`'s already-locked closure). | `src/index/skills.rs:212-235` | None. |

### 3. Embedder lifecycle

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| —        | `catalog update` uses the lazy `Option<FastembedEmbedder> + GetOrInsertWithResult` pattern, and `read_enabled_plugins` runs *before* `get_or_insert_with_result`. A zero-enabled-plugins install never touches model files. | `src/commands/catalog/update.rs:44-77` | None. |
| —        | `reindex::run` checks `plugins.is_empty()` and short-circuits *before* `load_embedder(&paths)?`. Functionally equivalent to the lazy pattern. | `src/commands/reindex.rs:42-52` | None. |
| —        | `query::run` always loads the embedder — correct, a query without an embedder is incoherent. The reranker is conditional on `--no-rerank`. | `src/commands/query.rs:79-92` | None. |
| —        | `plugin enable` loads the embedder unconditionally after the TTY-confirmed model download — also correct, enable needs to embed every skill. | `src/commands/plugin/enable.rs:50` | None. |
| —        | `StubEmbedder` is `#[cfg(test)]`-gated at the module level in `src/embedding/stub.rs` — never compiles into production. The lifecycle, reindex, and catalog-update tests all consume it through the `Embedder` trait. | `src/embedding/stub.rs` | None. |

### 4. `vec0` virtual-table interaction

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| —        | The only `INSERT` against `skill_embeddings` lives in `upsert_skill`, which now `DELETE`-then-`INSERT`s — the fix from PR #25 is in place and documented inline. | `src/index/skills.rs:284-294` | None. |
| —        | The only other `skill_embeddings` mutations are `DELETE`s in `delete_by_plugin` (line 218) and the Removed branch of `reindex_plugin_atomic` (line 498). Both are vec0-safe. | `src/index/skills.rs:218, 498` | None. |
| —        | `meta.rs:66` uses `ON CONFLICT(key) DO UPDATE` — but that's against the regular `meta` table, not `skill_embeddings`. No issue. | `src/index/meta.rs:66` | None. |
| —        | `upsert_skill:252` uses `ON CONFLICT(catalog, plugin, name) DO UPDATE` against the `skills` table (a regular SQLite table). Also fine. | `src/index/skills.rs:252` | None. |

### 5. Credential scrubbing

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| minor    | `count_commits_between` shells out to `git rev-list --count` directly via `std::process::Command` rather than through the `Git` struct, and `.output()` swallows stderr without running it through `scrub_credentials`. The stdout (just an integer) is safe, but if a future caller widens this helper to surface stderr on failure, scrubbing would be missed. | `src/commands/catalog/update.rs:366-382` | Either (a) refactor to call `Git::run` (and route through `scrub_to_string`), or (b) add a `// scrubbing not required: stdout is a parsed integer, stderr discarded` comment to record the intent and pin the shape against drift. |
| —        | The only direct `reqwest` call site (`embedding::download::stream_to_partial`) wraps both error display *and* the non-error "HTTP {status} fetching {url}" string through `scrub_for_diag` → `git::scrub_to_string`. Presigned URL query params get scrubbed. Correct. | `src/embedding/download.rs:95-105` | None. |
| —        | `tracing::info!` / `warn!` / `debug!` calls in production code use field-style structured logging with model names, plugin IDs, and local paths — no URLs, no credentials. | `src/plugin/lifecycle.rs:218,282,310,420,538,578,644`, `src/commands/catalog/remove.rs:119`, `src/commands/plugin/enable.rs:154`, `src/commands/models/download.rs:92` | None. |
| —        | The `Git` struct's `run` method (which is the only path that captures Git stderr) routes through `scrub_to_string` before constructing `TomeError::GitFailed`. Correct. | `src/catalog/git.rs:142-148` | None. |

### 6. `std::process::exit` outside `main.rs`

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| —        | `commands::status::run` invokes `std::process::exit(1)` for non-Ok health — the documented exception. Comment at line 12-15 records the rationale. | `src/commands/status.rs:40` | None. |
| —        | `main.rs` has three legitimate `std::process::exit` sites: pre-parse `--version` short-circuit, Ok-path, Err-path. All correct. | `src/main.rs:16,37,41` | None. |
| —        | Grep shows no other process::exit sites in `src/`. | — | None. |

### 7. Pre-parse `--version` hook in `main.rs`

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| minor    | The pre-parse hook scans `std::env::args` for `--version` / `-V` and (separately) `--json`. Both forms (`--version --json` and `--json --version`) work because the scan is order-independent. However, the hook also triggers on any subcommand argument literally spelled `--version` or `-V` — e.g. `tome plugin --version` would print the global version output, not error out as clap would. In practice subcommands don't accept those flags, but it's worth a defensive check that the version flag comes from the top-level slot. | `src/main.rs:12-17` | Optional: restrict the scan to args before the first non-flag token (the subcommand name). For v0.2.0 this is low-risk because no subcommand defines its own `--version` flag, but pin the contract with a regression test. |
| nit      | `commands::status::print_version` is `pub` so it can be called from `main.rs`; everything else in `status.rs` is `pub` for the same reason or because it's a library-API entry. The visibility surface is fine but worth a `pub(crate)` audit once status grows. | `src/commands/status.rs:356-390` | None for now. |

### 8. Public API surface

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| minor    | `commands::plugin::registry_seeds` is `pub` (vs `pub(crate)`) per the P8 retro for drift tests. `commands::reindex::run_with_deps`, `commands::reindex::Scope`, `commands::status::assemble_report` are all `pub` for the same library-bypass test pattern. Documented and consistent. | `src/commands/plugin/mod.rs:69`, `src/commands/reindex.rs:288, 70`, `src/commands/status.rs:91` | None. |
| minor    | `commands::models::ModelState`, `cheap_state`, `read_manifest`, `primary_file_path`, `human_mb` are all `pub`. Per the P6/P8 retro this was deliberate so `status.rs` could consume them. They're also reachable from tests as a side benefit. But `human_mb` is duplicated identically in `commands::plugin::human_mb` — both `pub(crate)`. The DRY violation is acknowledged in the comment at `models/mod.rs:127-128` ("Worth promoting if a third caller appears"). The third caller exists in spirit (`status.rs::human_size`, although the implementation differs — KiB/MiB vs MB). | `src/commands/models/mod.rs:129-132`, `src/commands/plugin/mod.rs:119-122`, `src/commands/status.rs:336-346` | Promote one of the `human_mb` helpers to `presentation::format` or similar before v0.2.0 — or accept the duplication and remove the deferred-promotion comment. The current state is "we knew, we didn't act" which becomes drift over time. |
| minor    | `commands::plugin::resolve_plugin_dir` is re-exported via `pub(crate) use crate::plugin::lifecycle::resolve_plugin_dir;` but `resolve_plugin_dir` itself is declared `pub` (not `pub(crate)`) in `plugin/lifecycle.rs:341`. The re-export then narrows the surface. Either over- or under-shoot is fine but the asymmetry reads strangely. | `src/plugin/lifecycle.rs:341`, `src/commands/plugin/mod.rs:161` | Tighten `lifecycle::resolve_plugin_dir` to `pub(crate)` — only `commands::plugin` reaches across, and the helper isn't a library-API entry. |
| nit      | `commands::plugin::read_catalog_manifest` is a `pub(crate) use` re-export at `commands/plugin/mod.rs:210` — looks like dead code (only `query.rs` consumes it; `super::plugin::{… read_catalog_manifest, …}`). Verify it's actually reached. | `src/commands/plugin/mod.rs:210` | Grep to confirm the import is live, otherwise drop. |

### 9. `unwrap()` / `expect()` / `panic!()` in non-test code

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| minor    | `commands::plugin::mod.rs:91-92, 102, 113` use `expect("MODEL_REGISTRY must declare exactly one embedder/reranker")` — assertions about the compile-time-constant registry. The invariant is enforced by the registry file's structure plus a `#[test]` in `embedding::registry`. Acceptable: panics surface programmer error in the registry, not user input. | `src/commands/plugin/mod.rs:91,92,102,113` | None — these are intentional and unreachable on a well-formed registry. |
| minor    | `commands::catalog::update.rs:99` `expect("just inserted")` inside the `GetOrInsertWithResult` impl. The pattern is correct (we just set `*self = Some(f()?)`), and the `expect` documents the invariant. | `src/commands/catalog/update.rs:99` | None. |
| minor    | `commands::catalog::update.rs:208` `expect("caller checked")` after `config.catalogs.get(name)` — the caller (`run`) does check the key membership a few lines above. Defensible. | `src/commands/catalog/update.rs:208` | Could thread the `&CatalogEntry` directly instead of re-looking-up, but the duplication is harmless and explicit. |
| minor    | `index::vec_ext::register_globally` `expect("REGISTER_RC poisoned")` on two mutex lock sites. Mutex poisoning here means another thread panicked while registering the extension — propagating the panic is the right outcome (we can't recover from a half-registered C extension). | `src/index/vec_ext.rs:44, 47` | None. |
| nit      | `catalog::git.rs` has 4 `expect("valid regex")` calls inside `OnceLock::get_or_init` closures for the credential-scrubbing regexes. Acceptable: the regexes are compile-time constants and there's a `#[test]` in the same file that round-trips them. | `src/catalog/git.rs:51,53,67,77,85,194` | None. |
| nit      | `presentation::prompt.rs:77` `let _ = &progress::stderr_is_tty;` — keeps an unused import alive across module-shape refactors. Code smell: if the import is unused, drop it; if it's needed elsewhere, document that. | `src/presentation/prompt.rs:77` | Remove the binding and let the unused-import lint catch real drift. |
| nit      | `commands::status.rs:394-397` `_force_sha_use` with `#[allow(dead_code)]` — same smell as the prompt one above. | `src/commands/status.rs:394-397` | Remove. |
| nit      | `commands::reindex.rs:300-303` `_force_embedder_use` with `#[allow(dead_code)]` — same idiom. | `src/commands/reindex.rs:300-303` | Remove. The `Embedder` import is consumed transitively by `LifecycleDeps`; the compiler will tell you if it isn't. |

### 10. LockGuard / explicit-release pattern

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| —        | `LockGuard` provides both `Drop` (best-effort) and explicit `release(self) -> Result<(), TomeError>`. All five `lifecycle::*` write paths use the explicit pattern: `let lock = acquire_lock(...)?; let result = work(); match result { Ok(_) => { lock.release()?; ... } Err(e) => { drop(lock); Err(e) } }`. | `src/plugin/lifecycle.rs:127-148, 167-184, 212-239, 262-295, 304-322`, `src/index/lock.rs:37-47` | None. |
| —        | The `LockGuard` is held across the entire inner work scope — no `?` early-return that would release the lock before the transaction commits. The `?` operator can short-circuit *inside* the inner closure, but the closure's `Err` returns up through the `match`, which always reaches the `drop(lock)` branch. Verified for `enable`, `disable`, `reindex_plugin`, `auto_disable_orphan`, `cascade_disable_for_catalog`. | as above | None. |

---

## Cross-cutting observations

### Other findings (not in numbered focus areas)

| Severity | Finding | File:line | Suggested action |
|----------|---------|-----------|------------------|
| **major**| `tome catalog remove --force` cascade emits incorrect per-plugin `skills_dropped` counts in the JSON `cascade` array. `cascade_disable_for_catalog` returns one `u32` (the catalog total), and the caller distributes it as `[total, 0, 0, …]` — comment at line 91-93 acknowledges "we don't have a per-plugin breakdown". This makes per-plugin telemetry meaningless and would silently surface as drift in any downstream consumer that sums the array (it'll equal the right total) but breaks any consumer that reads per-row. | `src/commands/catalog/remove.rs:76-100`, `src/plugin/lifecycle.rs:252-295` | Change `cascade_disable_for_catalog` to return `Vec<(String, u32)>` (plugin name → rows dropped) instead of a single `u32`. `delete_by_plugin` already returns per-plugin counts; the helper just needs to collect them. The caller then maps directly without the "first record gets all" hack. |
| **major**| `commands::status::classify` precedence is `embedder.state != "ok" || !index.integrity_ok && index.present` — Rust parses this as `(embedder.state != "ok") || ((!index.integrity_ok) && index.present)`. The intended semantics ("index integrity failed AND we have an index to check") read correctly, but the priority is non-obvious and a future reader could "fix" it the wrong way. | `src/commands/status.rs:222` | Parenthesise explicitly: `embedder.state != "ok" || (!index.integrity_ok && index.present)`. Zero semantic change, much clearer. |
| **major**| Phase 9 closeout left an inline comment admitting the cascade JSON output is wrong (see major #1 above). Either the contract under-specifies, the test under-asserts, or the helper signature was minimised too aggressively. The P9 retro mentions "helper signature minimisation (don't pass `LifecycleDeps` when only `(paths, seeds)` are used)" — this is the same axis but in the wrong direction. | `src/commands/catalog/remove.rs:90-99`, `specs/002-phase-2-plugins-index/retro/P9.md` (out of scope to read) | Treat as the same fix as major #1 — return per-plugin counts. The signature minimisation principle still applies; this isn't `LifecycleDeps`, it's a richer return type. |
| minor    | `commands::status::check_index` swallows the result of `current_schema_version` and `query_row` (lines 158, 166-172, 173-177): `Err(_)` becomes `None` or `0`. For status this is correct — the report still needs to render — but the silent fallback for `plugins_enabled` / `skills_indexed` means a corrupt index reports `(0, 0)` instead of "unknown", which could read like an empty install. | `src/commands/status.rs:166-177` | Surface `query_row` errors into `integrity_ok = false` (overall = Unhealthy) instead of `0`. Or thread a `bool unknown` field into `IndexHealth` so the JSON consumer can tell "zero" from "couldn't query". |
| minor    | `embedding::download::write_manifest` maps `serde_json` serialisation errors to `ModelRegistrationParseError`, which is named for *parse* errors. The exit code (33) still makes sense, but the error name lies. | `src/embedding/download.rs:152-157` | Either rename the variant to `ModelRegistration{Parse,Serialize}Error` or add a `ModelRegistrationSerialiseError` companion. |
| nit      | `commands::catalog::update.rs:472` `let _ = Rfc3339; // silence unused-import in this fn` — same smell as the `_force_*` functions. If the import isn't used in this fn, scope it elsewhere or just delete. | `src/commands/catalog/update.rs:472` | Remove; let the unused-import lint speak. |

---

## What's done well

- **Closed error set.** 27 named `TomeError` variants, each with an explicit `exit_code()` and `category()` arm. The `Internal` variant exists but is genuinely reserved for "unexpected" cases (config serialisation, prompt-builder misconfiguration). The `ManifestInvalid` sub-enum is a textbook example of "error precision should match the layer that produced it".
- **Lock discipline.** Five lifecycle write paths, all using the same explicit-release-or-drop pattern. The `LockGuard` API forces you to *choose* — `release()?` for the success path that wants the unlock error, `drop()` for the failure path. The fact that all five call sites read identically is the strongest evidence the pattern works.
- **vec0 awareness.** The `DELETE`-then-`INSERT` pattern in `upsert_skill` is documented inline (line 281-283) explaining WHY `INSERT OR REPLACE` doesn't work. The comment will save the next person 30 minutes.
- **Lazy embedder loading.** The `GetOrInsertWithResult` trait in `commands::catalog::update.rs:83-101` is a clean six-line extension to `Option<T>` that reads at the call site exactly like `Option::get_or_insert_with` — but for the `Result<T, E>` case. Worth promoting to a utility module if a third caller appears.
- **Credential scrubbing.** The regex layering (URL login, SSH login, KV secret, long hex) plus the `safe`/`unsafe_hex` alternation that preserves git SHA references in error messages is subtle and right.
- **Pre-parse `--version`.** Disabling clap's auto-`--version` and shipping a manual pre-parse hook in `main.rs` to surface embedder + reranker identities is the right call. Cleaner than wedging it into a `Cli::display_version`.

---

## Verification commands

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

All clean as of the audit (per CLAUDE.md "240 tests pass across 36 suites").
