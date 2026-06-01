# Contract: Cleanup Bundle (US3)

**FRs**: FR-013, FR-014, FR-015, FR-016 · **Research**: §R-19, §R-20

A bundle of small correctness/hygiene defects so the public repo does not read as half-finished. No schema change, no new exit code. Split into ~2 small themed PRs (correctness cleanups; doc-comment + `--help` sweep).

---

## FR-013 — Intra-plugin duplicate `(kind, name)` detected, warned, truthfully counted (F-PLUGIN-NAME-COLLISION)

**Sites**: `src/plugin/lifecycle.rs` (count), `src/index/skills.rs` (insert).

**Invariant**: an intra-plugin duplicate `(kind, name)` MUST be detected and warned, and the "N indexed" message MUST count rows **actually written** (no silent `ON CONFLICT` overwrite, no over-count).

**Mechanism** (§R-20): detect the duplicate `(kind, name)` before insert, `tracing`-warn it, and tally written rows. **Respect that `sqlite-vec` virtual tables reject `ON CONFLICT`/`INSERT OR REPLACE`** — use `DELETE`-then-`INSERT` where the embedding row is touched.

**Test**: index a plugin with a duplicate `(kind, name)`; assert a warning is emitted, one entry is kept deterministically, and the reported count equals rows written.

---

## FR-014 — Malformed `config.toml` → `ManifestInvalid::TomlParse` (exit 5) (F-CONFIG-GENERIC-ERR)

**Site**: `src/catalog/store.rs:20–35`.

**Invariant**: a malformed `~/.tome/config.toml` MUST surface the specific TOML/manifest-parse error and exit code (**reuse `ManifestInvalid::TomlParse`, exit 5**), not a generic `Internal`/exit 1.

**Mechanism** (§R-19): map the parse failure to the existing variant naming the file. No new code (the one Tome-owned input currently violating specific-over-generic).

**Test**: a hand-corrupted `config.toml` → exit 5 + a message naming the file (not exit 1).

---

## FR-015 — Off-spec inputs fail closed (F-HOOKS-COERCE, F-BOOT-META-DIAG)

**Sites**: `src/harness/hooks.rs` (or `reconcile/hooks.rs` post-decomp), `src/index/migrations.rs`.

**Invariant**:
- A non-array `hooks` event value MUST be **rejected** with the existing settings-write error (**exit 44, `HookSettingsWriteFailed`**), not coerced to `[]`. (`append_if_absent` currently silently replaces a user's non-array value — asymmetric with the module's otherwise fail-closed discipline.)
- A **meta row indicating corruption** MUST be **distinguished** from a fresh database — a diagnostic distinction reusing existing variants, **no new exit code**. (`current_schema_version`'s blanket `.ok()` collapses "meta present but `schema_version` row missing" → "fresh DB" → a misleading "table meta already exists" on re-bootstrap.)

**Mechanism** (§R-19): fail closed in `append_if_absent` (mirror `load_settings`/`ensure_hooks_object`); replace the blanket `.ok()` with an explicit match distinguishing the corruption case (mirror `meta.rs`). The tx rolls back on `?` — no data loss.

**Test**: (a) a non-array `hooks` event value → exit 44; (b) a meta row with a missing `schema_version` → the corruption diagnostic, not "fresh DB".

---

## FR-016 — Dead code removed; stale doc-comments swept; `--help` citations stripped (F-STORE-REFCOUNT-DEAD, DOC-06)

**Invariant**: (a) the unused `store::reference_count` accessor MUST be **removed** and its misdirecting doc pointer (`paths.rs`) corrected; (b) stale doc-comments describing **shipped features as stubs** MUST be swept; (c) internal spec/contract citations (`FR-`/`NFR-`/`contracts/*.md`) MUST be **stripped from user-facing `--help`/clap `///` doc-comments** (DOC-06).

**Sweep list** (§R-20, the named stale-comment sites):
- `store::reference_count` + its `paths.rs` doc pointer — delete + fix.
- `mcp/prompts.rs` — the comment claiming `prompts/get` resolves to `METHOD_NOT_FOUND` (the real handler is wired).
- `commands/workspace/use_.rs` — the "US1.a stub" comment (sync is wired).
- `mcp/tools/search_skills.rs` — the stale "F2a single global config" comment.
- `substitution/mod.rs` — the self-contradicting Stage-3 no-args comment.
- `index/meta.rs` — the dead `MetaKey::LastWriterPid` enum variant.
- the embedding registry — the doc claiming downloads verify size **and** hash (only SHA-256 is verified).
- `src/cli.rs` — strip `FR-`/`NFR-`/`contracts/*.md` from clap `///` doc-comments.

**Test/verification**: removal compiles clean (`-D warnings`, incl. clippy doc-lints); a check (or review) confirms no `FR-`/`NFR-`/`contracts/*.md` token appears in `--help` output; the doc-comment sweep is verified by review.

---

## Cross-cutting

- All four FRs: **no schema change, no new exit code** (NFR-002).
- `-D warnings` promotes clippy doc-lints (`doc_lazy_continuation`, etc.) that plain `cargo test` ignores — run `cargo clippy --all-targets --all-features -- -D warnings` on touched files before committing (Phase 6 P8 lesson).
