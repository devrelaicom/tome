# Tome — Comprehensive Code Review for Beta Readiness

**Date:** 2026-05-30 · **Scope:** all of `src/` (~40.7K LOC, 14 modules) reviewed line-level for correctness, security, completeness, placeholders/smells · **Audited version:** 0.6.0

**Method:** a 31-agent workflow — **11 module reviewers** (every module assigned, balanced by LOC, load-bearing files read in full) + **2 cross-cutting specialists** (whole-repo invariant tracing; exhaustive placeholder/smell sweep) → **triage/dedup** (calibrate severity, drop false alarms) → **adversarial verification** of every candidate bug against the real code (refute-by-default; 17 findings verified). Findings were judged against the project's *documented invariants* (closed error set, sync-boundary, atomic writes, credential scrubbing, the reconcile contract, path-segment validation), not generic style. `clippy -D warnings`, `fmt`, and the full test suite are already green in CI, so this hunted what those cannot catch.

**Verification outcome:** 17 findings verified → **12 confirmed, 4 partial, 1 refuted** → **13 real bugs (8 major, 5 minor/info)**. No blockers.

---

## Verdict: **architecturally beta-ready; not ship-ready as-is on the retrieval path**

This is a high-discipline, mature codebase. The security-critical trust boundaries the reviewers independently stress-tested all came back **clean with strong evidence** (details in *Verified clean* below). There are **no blockers, no data loss on normal use, no crashes on normal input, no security exposure, and no unimplemented features or production placeholders.** That is a genuinely solid core for a beta.

But there are **8 confirmed real bugs at major severity** in otherwise-working features. None is a hard blocker, yet several would make a beta user's *first experience* look broken (empty search results, a crashing `doctor` after upgrade, broken `catalog show` for SSH sources). I recommend a focused **beta gate** of 5 fixes before shipping, fix-or-document for the next 3, and a cheap cleanup pass for the minors + stale doc comments.

### Direct answers to your questions

| Question | Answer |
|---|---|
| **Is all functionality correctly implemented?** | **Functionally complete** — all 12 advertised commands exist and execute; no stubs, no "not implemented" runtime paths. But **8 real bugs** mean some working features misbehave on specific inputs (most importantly semantic search under multi-workspace/filtered queries). |
| **Free of placeholders / dummies / smells?** | **Production code is free of TODO/FIXME/FIX/DEFER/HACK placeholders and of dummy/mock/fake functions** (verified exhaustively; stubs are confirmed `#[doc(hidden)]` test-only seams). Residue is **stale doc *comments*** describing shipped features as stubs (cosmetic) + two real smells (dead `reference_count`, an over-broad `.ok()`). |
| **Critical security issues or bugs?** | **No critical/exploitable security issues.** Path-traversal, injection, symlink, and credential-scrubbing defenses all held under adversarial probing. One DoS-class bug (unbounded `plugin.json` → OOM from a hostile catalog) and one credential-*handling* correctness bug (cache-key divergence orphans/leaks clones for token/SSH URLs — it does not *expose* secrets). |
| **Ready for the initial beta?** | **Yes, after a small beta gate.** Fix the 5 first-impression bugs (KNN, doctor-RW, cache-key, prompt-collision, ws-toml-newline); fix-or-document 3 more; bundle the cheap minors. Then ship. |

---

## 🔴 Confirmed MAJOR bugs (8) — all verified real against the code

> None is a blocker (no data loss on normal use, no crash on normal input, no security exposure). All are real, reachable, and contradict a documented invariant or advertised behavior.

### F-KNN — semantic search silently under-fetches (or returns ZERO) — **highest priority**
- **`src/index/query.rs:77-96`** (+ `commands/query.rs:215-219`, `mcp/tools/search_skills.rs`). The sqlite-vec `k = ?` limit is bound to `top_k` and applied to the **global** vector neighborhood; the workspace / `searchable=1` / `--catalog` / `--plugin` filters are a regular SQL `JOIN`/`WHERE` applied **after** vec0 already chose the global-nearest *k*. So when nearer vectors belong to other workspaces, non-searchable commands, or filtered-out catalogs, results fall below `top_k` — **empirically reproduced returning 0 rows** when a matching row existed.
- **Bites:** multi-workspace users, anyone using `--catalog`/`--plugin`, anyone with `user-invocable: false` commands. The common single-workspace, no-filter path is unaffected (which is why tests didn't catch it — the one filter test uses `k=20` on a tiny corpus).
- **Fix:** over-fetch then post-trim with a bounded widen loop (no schema change), or add workspace/searchable/catalog/plugin as vec0 `PARTITION KEY`/metadata or a `rowid IN (...)` constraint (schema migration). Add a regression test placing ≥`top_k` nearer non-matching rows ahead of the match.

### F-DOCTOR-RW — read-only `tome doctor` migrates the DB unlocked and can crash on upgrade
- **`src/doctor/checks.rs:58-68`** uses the **read-write** `index::open` (which runs `apply_pending` migrations + WAL pragmas) instead of `open_read_only`. On a stale-schema DB, plain `tome doctor` runs an **unlocked schema migration** (FR-124 violation + a race with a concurrent `reindex`/`enable` holding `index.lock`); on a *future*-schema DB it propagates `SchemaVersionTooNew` → **exit 73 aborts the whole `doctor` run**, contradicting the documented "doctor never crashes."
- **Bites:** a user who upgrades Tome and runs `doctor` first (a near-certain early beta action).
- **Fix:** `index::open_read_only` + swallow the error to a degraded report (mirrors `check_index` at `doctor/mod.rs:101-119`). `--fix` still runs the real lock-held migration via `repair_schema`.

### F-CACHE-KEY-DIVERGE — SSH/credentialed catalogs orphan their clone; `show`/`update` break
- **`src/commands/catalog/add.rs:33,67,117,128`**. The cache dir and reuse-refcount are keyed by the **raw** URL, but the DB persists the **scrubbed** URL, and *every reader* (`show`, `update`, `remove`, `list`, doctor, sync) resolves the cache by the scrubbed URL. For any URL where scrubbing changes the string — **`git@github.com:owner/repo` (plain SSH!)**, `ssh://git@…`, `https://user:token@…` — the clone lands at one path and is looked up at another: `catalog show` → ENOENT, `update` → wrong dir, `remove` → orphaned clone leaked on disk, reuse never triggers.
- **Bites:** anyone adding a private repo over SSH (common) or a token URL (the privacy-conscious user).
- **Fix:** key the cache dir + refcount by `scrubbed_url`; keep `git.clone_shallow` on the raw URL for auth. Add an SSH-source round-trip test (current fixtures only use plain https where raw==scrubbed).

### F-PLUGIN-MANIFEST-DOS — unbounded `plugin.json` read → OOM from a hostile catalog
- **`src/plugin/manifest.rs:61-68`** uses raw `std::fs::read` (allocates the whole file), bypassing the 256 KiB `PLUGIN_MANIFEST_MAX` cap that the *sibling* `SKILL.md` path already enforces (`frontmatter.rs:292`). A multi-GiB `plugin.json` from a cloned catalog OOM-crashes Tome at `enable`/`show`/`list`.
- **Fix (one line):** route through `crate::util::bounded_read(path, PLUGIN_MANIFEST_MAX)`. **Fix the class, not just the instance:** the same unbounded-read gap exists at `catalog/manifest.rs:46`, `plugin/lifecycle.rs:958`, `plugin/components.rs:170`, and notably **`doctor/checks.rs:174`** (reads `tome-catalog.toml` unbounded on the read-only/CI surface).

### F-MCP-PROMPT-COLLISION — a prompt silently vanishes on *ordinary* names
- **`src/mcp/prompt_collision.rs:71-126`** + `prompts.rs:529`. Collision losers get a `{base}{idx+1}` suffix that is **never checked against other buckets' names**; the terminal `HashMap::insert` then silently discards a duplicate. Reachable with **non-hostile input**: a plugin with Command `foo` + user-invocable Skill `foo` + Command `foo2` → one entry disappears from `prompts/list` and returns `prompt_not_found` on `prompts/get`. Worse, `tome doctor` *misreports* the resolution.
- **Fix:** assign final names against a single global taken-set, suffixing until free (preserves both entries). Add the command+skill+`foo2` regression test.

### F-WS-TOML-NEWLINE — a newline in a catalog name bricks a new workspace
- **`src/workspace/init.rs:264-266`** `escape_toml_basic` escapes only `\` and `"`, not newlines/control chars. Catalog names are free-form third-party text (the manifest validator only rejects empty). A newline-bearing name → unparsable `settings.toml` → `WorkspaceMalformed` (exit 70) on *every* harness op for that workspace. Triggered via `workspace init --inherit-global` re-emitting a poisoned global catalog name.
- **Fix:** emit `settings.toml` via `toml_edit` (like the sibling `rename`/`regen_summary` paths — deletes `escape_toml_basic`) **and** reject control chars in catalog names at the manifest boundary.

### F-REMOVE-TOCTOU — `catalog remove --force` cascades off a stale pre-lock list
- **`src/commands/catalog/remove.rs:54,96,114`**. The enabled-plugins list is read **before** acquiring `index.lock`; the locked closure reuses that stale `Vec` instead of re-reading. A concurrent `plugin enable` (which serializes on the same lock) → lost update → a **ghost-enabled plugin** whose catalog enrolment is deleted (and cache dir possibly removed). Contradicts the contract's "acquire lock, *then* check enabled plugins."
- **Bites:** scripted/parallel usage (CI enrolling while another job removes). **Fix:** re-derive the cascade input inside the locked closure.

### F-RULES-OPENCODE — OpenCode silently misses Tome's rules when paired with Codex/Gemini
- **`src/harness/sync.rs:269-302`**. The shared-`AGENTS.md` dedup writes only the **first** sharer's body style. Registry order makes Codex (`AtInclude`) win, so OpenCode (`Inline`, because it can't resolve `@`-includes) receives `@.tome/RULES.md` as literal prose → Tome's rules/skills index is **never delivered to OpenCode** in that combo. No error surfaces. (The parallel *guardrails* reconciler already does this correctly via a union.)
- **Fix:** pick the lowest-common-denominator body style — if any live sharer is `Inline`, write `Inline` (valid for `AtInclude` harnesses too).

---

## 🟡 Confirmed MINOR bugs (4) — real, narrower trigger, cheap fixes

| ID | Issue | File | Fix | Reachable on normal input? |
|---|---|---|---|---|
| **F-PLUGIN-NAME-COLLISION** | Intra-plugin duplicate `(kind,name)` silently overwrites one entry via `ON CONFLICT` and over-counts the "N skills indexed" message. | `plugin/lifecycle.rs:601-612`, `index/skills.rs:803-908` | Detect duplicate `(kind,name)`, warn, and count rows actually written. | Yes (plugin-author footgun) |
| **F-CONFIG-GENERIC-ERR** | Malformed `~/.tome/config.toml` → generic `Internal`/exit 1 (the one Tome-owned input that violates specific-over-generic; `Internal`'s own doc forbids this collapse). | `catalog/store.rs:20-35` | Reuse the existing `ManifestInvalid::TomlParse` variant (exit 5) — no new code. | No (hand-edit/corruption) |
| **F-HOOKS-COERCE** | `append_if_absent` silently replaces a user's non-array `hooks` event value with `[]` before appending (asymmetric with the module's otherwise fail-closed discipline). | `harness/hooks.rs:272-292` | Fail closed (exit 44) like `load_settings`/`ensure_hooks_object`. | No (off-spec input only) |
| **F-BOOT-META-DIAG** | `current_schema_version`'s blanket `.ok()` collapses "meta present but `schema_version` row missing" to "fresh DB" → re-bootstrap fails with a misleading "table meta already exists." | `index/migrations.rs:333-348` | Explicit match distinguishing the corruption case (mirror `meta.rs:59-66`). No data loss (tx rolls back). | No (corruption/tampering) |

---

## ⚪ Partial / refuted (5) — verified *not* to need action for beta

These were flagged by a reviewer but the adversarial verifier **downgraded or refuted** them — included to show the rigor and to pre-empt re-discovery:

- **F-MCP-STORE-LOAD-RUNTIME** *(partial, not a bug)* — `store::load` runs sync on the single-thread MCP runtime without `spawn_blocking` (unlike its siblings), but it's a bounded ≤1 MiB sub-ms read on a single-client server. Cosmetic discipline nit; optional.
- **F-DOCTOR-STALE-PHASE5** *(partial, info)* — Phase 5 info surfaces (`entry_counts`/`orphan_data_dirs`/`prompts`) aren't refreshed after `--fix`; the only real trigger is a cross-major DB migration during `--fix`, and the effect is a briefly-omitted block (not wrong numbers). A catalog re-clone is **not** a trigger (original finding overstated). Defer past beta.
- **F-SUB-DATADIR-DOT** *(partial, not reachable)* — `sanitise_path_component` allows a bare `.`/`..`, but the upstream `PluginId::validate_segment` gate closes every reachable path. Defense-in-depth hardening only.
- **F-STORE-REFCOUNT-DEAD** *(partial, not a bug)* — `store::reference_count` is dead code (zero callers) with docs (`paths.rs:130-133`) that misdirect a future maintainer at the wrong refcount path. The `Vec<Scope>` vs `usize` type mismatch blocks silent misuse. **Cleanup:** delete it + fix the pointer.
- **F-CACHE-REMOVE-SYMLINK** *(refuted)* — `remove_dir_all` without symlink-refusal at cache cleanup is **not** a vector: empirically verified on the MSRV that `remove_dir_all` *unlinks* a symlinked root without following it, and the path is a sha256 name under the user's own home. Optional consistency hardening only.

---

## 🔍 Placeholders, dummies & completeness (your explicit ask) — **clean, with one caveat**

- **No production placeholders.** Exhaustive sweep (comments + strings, `src/` and `tests/`): **zero** real `TODO`/`FIXME`/`FIX`/`XXX`/`HACK`/`DEFER`/`WIP`/"for now"/"placeholder"/"not implemented" in production code. The handful of hits are docstring prose describing intended design.
- **No `unimplemented!`/`todo!`** anywhere. All 12 `panic!` are `#[cfg(test)]`; the one `unreachable!` is a dispatch guard. No reachable-on-input panic (incl. hostile frontmatter; the agent-name defense held against 19 bypass attempts).
- **No dummy/mock/fake reaching production.** `StubEmbedder`/`StubReranker`/`StubSummariser` are confirmed `#[doc(hidden)]`, LTO-eliminated, and referenced only from test/test-injection seams.
- **No `dbg!`/stray debug prints** bypassing the output/tracing layer (the only `println!` is the intentional `--version`/status path).
- **The one real residue — stale doc *comments* (all `info`, code is correct):** several `///` comments describe shipped features as stubs and will mislead an auditor/contributor:
  - `mcp/prompts.rs` — comment claims `prompts/get` resolves to `METHOD_NOT_FOUND`, but the real substitution-driven handler is wired.
  - `commands/workspace/use_.rs` — comment calls harness-sync a "US1.a stub"; the real sync is wired.
  - `mcp/tools/search_skills.rs` — stale "F2a single global config" comment implying non-workspace-scoped results.
  - `substitution/mod.rs` — a self-contradicting "thinking-out-loud" comment in the Stage-3 no-args branch.
  - `index/meta.rs` — `MetaKey::LastWriterPid` is a defined-but-never-read dead enum variant.
  - `embedding` registry — doc claims downloads verify size *and* hash; only SHA-256 is verified (`size_bytes` unchecked).
  - **Recommendation:** a 30-minute comment sweep before beta so the public repo doesn't read as half-finished when it isn't.

---

## ✅ Verified clean (high-confidence positives)

The reviewers and verifiers specifically tried to break these and could not:

- **Closed error set:** 52 distinct exit codes, **zero duplicates, no `Other` arm**, exhaustively tested; every failure funnels through one scrubbed-stderr path.
- **The documented mass-delete hazard is CLOSED** — all three reconcile sinks propagate the existing-DB-open error *before* any destructive pass (the single biggest worry from the project history; verified safe).
- **Credential scrubbing** is applied at every reqwest/git/log boundary.
- **Path-traversal defense** (`is_safe_agent_name` + write-site `parent==dir` re-check) held against 19 unicode/encoding bypasses.
- **Guardrails marker scanning fails closed** across all spellings + CRLF; scan and parse share the same compiled regexes.
- **Hooks merge** writes only `settings.local.json` (never the committed `settings.json`); structural-match merge/remove is correct and atomic.
- **Substitution single-sweep no-rescan (NFR-007)** is structurally enforced and holds under hostile input; no ReDoS found.
- **Migration framework:** forward-only, per-step transaction, FK-toggled, id-preserving rebuilds, rollback + MVCC test-covered; `v3→v4` marker-only no-op correct.
- **SHA-256 model download integrity** is un-bypassable and well-tested; placeholder-checksum refusal correct; `LlamaBackend` `OnceLock` init/race/poison handling correct.
- **SQLite:** no SQL injection (all parameterized; only compile-time constants interpolated); advisory lock released on every path; transactions roll back on `?`; `orphan_cleanup` staging sweep is data-loss-safe.

---

## ⚠️ What this review did *not* exercise (honest coverage gaps)

Static reading + targeted probes, not full live reproduction. Before/after fixing, these warrant an empirical check:
- **F-KNN recall against the real binary** on a realistically-populated multi-workspace index (quantify how far below `top_k`).
- **The two concurrency races** (F-REMOVE-TOCTOU, F-CACHE-KEY-DIVERGE) via an actual two-process race.
- **Hostile/oversized inputs** end-to-end: a multi-GiB `plugin.json` through `enable`; a newline-bearing `tome-catalog.toml` through `workspace init --inherit-global`.
- **F-MCP-PROMPT-COLLISION** and **F-RULES-OPENCODE** against a live MCP server / real Codex+OpenCode harness combo.
- **Windows** path/agent-name behavior (reasoned via a Unix port only).

---

## 📋 Recommended beta gate & sequencing

**Beta gate (fix before shipping — first-impression + cheap):**
1. **F-KNN** — the headline feature returning nothing is the #1 "this is broken" risk. (Largest effort; do the no-migration over-fetch+widen.)
2. **F-DOCTOR-RW** — beta users will upgrade + run `doctor`. (~one-line.)
3. **F-CACHE-KEY-DIVERGE** — SSH catalog sources are common. (Small.)
4. **F-MCP-PROMPT-COLLISION** — silent prompt loss on ordinary names. (Small.)
5. **F-WS-TOML-NEWLINE** — bricked workspace; cheap `toml_edit` + boundary reject. (Small.)

**Fix-or-document (acceptable to ship with a known-issue note if time-boxed):**
6. **F-PLUGIN-MANIFEST-DOS** — one-line fix; do it, and sweep the sibling unbounded reads (esp. `doctor/checks.rs:174`).
7. **F-RULES-OPENCODE** — fix (the guardrails reconciler shows the pattern) or document the OpenCode+Codex/Gemini limitation.
8. **F-REMOVE-TOCTOU** — needs concurrency; document + fix opportunistically.

**Cheap cleanup bundle (low risk, high polish):**
- F-PLUGIN-NAME-COLLISION (warn + truthful count), F-CONFIG-GENERIC-ERR (reuse existing variant), F-HOOKS-COERCE (fail closed), F-BOOT-META-DIAG (explicit match), delete dead `store::reference_count` + fix its doc pointer, and the **stale doc-comment sweep**.

After the gate (1–5) + bundle, Tome is ready for an initial beta. Items 6–8 and the partials can follow in the beta window.

---

*Generated with [Claude Code](https://claude.com/claude-code). Every finding ID (e.g. `F-KNN`) traces to adversarially-verified evidence with the actual code and `file:line`; ask to expand any one for the full verifier transcript and suggested patch.*
