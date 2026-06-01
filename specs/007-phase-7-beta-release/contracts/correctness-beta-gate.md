# Contract: Correctness — Beta Gate (US1)

**FRs**: FR-001, FR-002, FR-003, FR-004, FR-005 · **SCs**: SC-001, SC-002, SC-003, SC-004 · **Research**: §R-2/3/4/5/6

The five first-impression bugs. Each is a contained fix to existing working code; none changes the schema or adds an exit code.

---

## FR-001 — Semantic search returns `min(top_k, total matches)` (F-KNN)

**Site**: `src/index/query.rs::knn` (+ callers `commands/query.rs`, `mcp/tools/search_skills.rs`).

**Invariant**: `query` (CLI, with/without `--catalog`/`--plugin`/`--strict`) and the MCP `search_skills` tool MUST return exactly `min(top_k, total matching entries)` regardless of how many nearer vectors are excluded by workspace/`searchable`/`--catalog`/`--plugin` filtering.

**Mechanism** (§R-2): bounded **over-fetch + widen**, no schema change. Bind vec0 `k` to a multiple of `top_k`; apply the existing post-`JOIN`/`WHERE` filters; if fewer than `min(top_k, total matches)` survive, re-query with a geometrically larger `k` until `top_k` post-filter matches are collected **or** the candidate set is exhausted (`k` reaches the row count). The widen ceiling returns the true smaller match set — never an error, never global-neighbourhood leakage.

**Test obligations**:
- `tests/search_knn_recall.rs` (stub embedder): place **≥`top_k` nearer non-matching rows** ahead of the match, **on a corpus large enough that a naive fixed-multiplier over-fetch would still miss it**; assert the match is present (0 → present) and the count does not shrink as the corpus grows.
- `tests/search_knn_recall_realmodel.rs` (SC-001): the **one-time** real-embedding-model recall check — the stub cannot prove recall. Run once, not in the fast CI suite.

---

## FR-002 — Read-only `doctor` never migrates unlocked, never aborts (F-DOCTOR-RW)

**Site**: `src/doctor/checks.rs` (the schema/index check, ~l.58–68).

**Invariant**: `tome doctor` (no `--fix`) MUST open the index **read-only**, MUST NOT run a migration, MUST NOT take the advisory lock, and MUST degrade to a partial report (never abort) on a stale **or** future schema. `doctor --fix` MUST still perform the lock-held migration.

**Mechanism** (§R-3): switch to `index::open_read_only` + swallow the schema error into a degraded report, mirroring the sibling `check_index` pattern in `doctor/mod.rs`. `--fix` keeps `repair_schema`.

**Test obligations**: `tests/doctor_readonly_schema.rs` — (a) stale-schema DB → read-only doctor completes, no migration ran, no lock taken; (b) future-schema DB → degraded report, **no exit-73 abort**; (c) `--fix` on a stale DB → migration performed as before. SC-002: 100% completion across an upgrade.

---

## FR-003 — Cache dir + refcount keyed by the scrubbed URL (F-CACHE-KEY-DIVERGE)

**Site**: `src/commands/catalog/add.rs` (`cache_dir_for`, `refcount_by_url`).

**Invariant**: the cache directory and reuse refcount MUST be keyed by the same **scrubbed** URL every reader resolves by; cloning MUST still use the **raw** URL for auth. Sources whose URL changes under scrubbing (plain SSH `git@host:owner/repo`, `ssh://`, `https://user:token@…`) MUST round-trip through `show`/`update`/`remove`/reuse without orphaning a clone.

**Mechanism** (§R-4): compute `scrubbed_url` first; key `cache_dir_for(&scrubbed_url)` + `refcount_by_url(&conn, &scrubbed_url)`; keep `git.clone_shallow(&raw_url)`.

**Test obligations**: `tests/catalog_ssh_roundtrip.rs` — add a catalog by a plain-SSH source; assert `show`/`update`/`remove` all resolve the cached clone and **zero clones are orphaned on disk** (SC-003). The plain-`https` raw==scrubbed case stays green (regression guard).

---

## FR-004 — MCP prompt names against a single global taken-set (F-MCP-PROMPT-COLLISION)

**Site**: `src/mcp/prompt_collision.rs` (+ consumer `mcp/prompts.rs`).

**Invariant**: prompt names MUST be assigned against **one global taken-set** so **no** user-invocable entry is dropped on a cross-kind collision; `doctor` MUST report the resolution truthfully.

**Mechanism** (§R-5): suffix `{base}{n}` until free, **re-checking each candidate suffix against the same taken-set** before the terminal insert (no silent `HashMap::insert` overwrite). Personas already join this single namespace (Phase 6 FR-066); the fix makes the implementation match.

**Test obligations**: `tests/prompt_collision_global.rs` — the Command `foo` + user-invocable Skill `foo` + Command `foo2` case: all three present in `prompts/list`, all resolvable on `prompts/get`, `doctor` reports the true resolution (SC-004). End-to-end verification is FR-012's job (the in-process harness); the fix may land before the harness exists.

---

## FR-005 — Workspace settings can't be poisoned by a control-char catalog name (F-WS-TOML-NEWLINE)

**Sites**: `src/workspace/init.rs` (emission) + `src/catalog/manifest.rs` (boundary reject).

**Invariant**: workspace `settings.toml` MUST be emitted such that a third-party catalog name containing a newline/control char cannot produce unparsable settings; control chars MUST be **rejected** in catalog names at the manifest boundary.

**Mechanism** (§R-6): emit `settings.toml` via `toml_edit` (deleting the bespoke `escape_toml_basic`, matching the sibling `rename`/`regen_summary` paths) **and** reject control chars in the recognised catalog `name` field at parse time (a value reject — third-party manifests stay lenient on *unknown* fields, §IV).

**Test obligations**: `tests/workspace_toml_control_chars.rs` — a newline-bearing catalog name: (a) rejected at the manifest boundary; (b) if already present (e.g. via `workspace init --inherit-global` re-emitting a poisoned global name), the emitted `settings.toml` remains parseable and every harness op on that workspace succeeds (no exit-70 brick).

---

## Cross-cutting

- No schema change, no exit-code change across all five (NFR-002).
- Each fix is a small independent PR (K1–K5, §R-22), landing after the decomposition (D) but otherwise parallelisable.
- `--json` output shape on `query`/`doctor`/`catalog` paths unchanged (byte-stable JSON pins stay green).
