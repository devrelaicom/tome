# Phase 0 Research — Phase 2

This document records the design decisions that the spec and the PRD do not fix but the plan depends on. Each section follows **Decision / Rationale / Alternatives considered / Open questions for review**. Where a decision has a non-trivial blast radius, the consequences are spelled out.

---

## R1. Binary size budget

### Decision

The release binary will use the following profile, with the targets below verified by CI on both `macos-latest` and `ubuntu-latest`:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
panic = "abort"
strip = "symbols"
opt-level = 3
```

`ort` is statically linked with **only the CPU execution provider** enabled:

```toml
[dependencies]
ort = { version = "2", default-features = false, features = ["load-dynamic", "fetch-models"] }
# NOTE: actual feature set tuned in implementation. The intent is: CPU EP only, no CUDA/CoreML/DirectML/TensorRT/OpenVINO.
```

Where `ort` exposes `default-features = false`, we disable every execution provider except the CPU provider. Where it exposes a `download-binaries` feature, we disable it and rely on the build script to compile against the statically-linkable upstream.

`fastembed-rs` is consumed with `default-features = false` plus only the features we use (embedder + reranker). Model files are downloaded at runtime — they are *not* bundled into the binary.

### Worst-case projection (per-component, stripped, x86_64-unknown-linux-gnu, release profile above)

| Component | Estimated stripped contribution |
|---|---|
| Phase 1 baseline (clap + serde + toml + anyhow + thiserror + tracing + directories + sha2 + tempfile + ctrlc + regex + semver + time + ~50 KB Tome code) | ~3.2 MB |
| `rusqlite` (`bundled`) | ~1.1 MB |
| `sqlite-vec` (vendored C) | ~0.25 MB |
| `indicatif` + `console` | ~0.20 MB |
| `comfy-table` | ~0.08 MB |
| `owo-colors` | ~0.03 MB |
| `inquire` | ~0.30 MB |
| `reqwest` (blocking + rustls only, no defaults) | ~0.70 MB |
| `fastembed-rs` shim | ~0.10 MB |
| `ort` CPU-only static | ~3.5 MB (conservative) |
| Tome Phase 2 code | ~0.5 MB |
| **Total projection** | **~9.95 MB** |

This is uncomfortably close to the 10 MB cap.

### Contingencies if the cap is breached in CI

Applied in order; we stop at the first that brings the binary back under the cap.

1. Cut features from `inquire` — we use Select, MultiSelect, Confirm only; some feature flags pull in editor / fuzzy / autocomplete machinery.
2. Replace `comfy-table` with a hand-rolled minimal table renderer (~5 KB). Tradeoff: maintenance burden vs ~75 KB saved.
3. Replace `reqwest` with `ureq` (rustls). `ureq` is ~300 KB; saves ~400 KB.
4. Replace `regex` (already in Phase 1) with `regex-lite` for the scrubber — saves ~200 KB but loses some perf. The scrubber is not hot.
5. As a last resort, switch `ort` to `load-dynamic` and ship a separate small loader. This is **not preferred** because it breaks "single binary, no system dependencies" (FR-038 spirit), and is only acceptable if every other contingency is exhausted. If we end up here, the spec must be revisited per NFR-001.

### Rationale

`ort`'s static linkage is the load-bearing variable. The published ONNX Runtime release for x86_64-linux-gnu is ~20 MB unstripped, ~6–8 MB stripped with `--gc-sections` and no execution providers other than CPU. The exact number depends on the upstream we link against; we will measure on a clean macOS arm64 and Ubuntu x86_64 in the first Phase-2 commit and bake the numbers into the CI assertion.

Stripping a static library at link time removes ~30–40% of symbols. `panic = "abort"` removes the unwinding tables (~5–8% saved). `lto = "thin"` is preferred over `lto = "fat"`: comparable size savings, far better compile times (which we care about for the MSRV+stable CI matrix).

### Alternatives considered

- **Bundle the models into the binary.** Rejected immediately — model files alone are ~325 MB.
- **Dynamic-load `ort` only on demand.** Saves binary size but breaks the "no system dep" guarantee. Reserved as a last resort.
- **Use `candle-rs` instead of `fastembed-rs`/`ort`.** `candle-rs` produces smaller binaries (~2–3 MB total inference stack) but the BGE family is not first-class supported there; tokenizer integration is also handlier. Tracked as a Phase-3 reconsideration if `ort` becomes painful.
- **Build a custom ONNX op subset.** Yak-shaving. Rejected.

### Open questions for review

None. Decision is committed; CI gate proves correctness on a per-PR basis.

---

## R2. SQLite concurrency model

### Decision

The single global `index.db` is opened with the following invariants:

- `journal_mode = WAL` set at first open per connection. WAL gives readers and a writer concurrent access without blocking.
- `synchronous = NORMAL` (the WAL default; balanced durability vs throughput).
- `busy_timeout = 5000` ms set on every connection. This makes SQLite itself wait up to 5 s when a competing writer holds the write lock before returning `SQLITE_BUSY`.
- A Tome-owned **advisory lockfile** at `${XDG_DATA_HOME}/tome/index.lock`, acquired with `fs2::FileExt::try_lock_exclusive` (or std equivalent if available by Tome's MSRV — resolved in Phase 1) **only for write-mutating commands**. Held for the duration of a single Tome process's write transaction. Read-only commands (`query`, `plugin list`, `plugin show`, `status`) do NOT take the lockfile.

The advisory lockfile is belt-and-braces on top of SQLite's own locks: it lets us return a clear, dedicated error (FR-040's "index database busy") within milliseconds in the common case of "another `tome plugin enable` is running," rather than blocking the user for the full 5 s timeout. It also lets us serialise schema migrations cleanly: the migration runner takes the lockfile, applies the migration in a transaction, releases.

### User-visible behaviour table

| Caller | Concurrent state | Behaviour |
|---|---|---|
| `tome query` (read) | Any | Always proceeds. Sees the most recent committed state. |
| `tome plugin list` / `show` | Any | Always proceeds. |
| `tome plugin enable` (write) | No other writer | Acquires lockfile, runs, releases. Common case. |
| `tome plugin enable` (write) | Another Tome writer running | `try_lock_exclusive` fails immediately. Exits with the dedicated `IndexBusy` exit code (FR-040, FR-048). Error message: "Another `tome` process is updating the index. Retry when it has finished." |
| `tome status` | Any | Always proceeds. Reports the lockfile state ("write in progress" if locked by another PID) as informational. |
| Migration | Anything | Takes the lockfile (waiting up to 5 s). On timeout: `IndexBusy`. |

### Crash safety

WAL means a power loss mid-transaction leaves the DB recoverable. The lockfile uses `fcntl`-style advisory locking which is released by the OS when the process dies (no orphan locks). The atomicity of FR-004 (plugin-granular enable) is achieved by wrapping the entire embed-and-insert sequence in a single SQLite transaction: SIGINT between skills aborts the transaction; SIGINT during a single skill's embedding completes that skill (we cannot cancel an in-flight ONNX call cleanly) but the surrounding transaction still rolls back at process exit because the connection drops without `COMMIT`.

### Rationale

The PRD requires WAL ("WAL mode enabled at open time"). The constitution requires predictable exit codes for every named failure class — a generic "lock contended" would violate that. The lockfile gives us deterministic, sub-millisecond detection of the contended case without a 5 s blocking wait; the `busy_timeout` is still set as a defensive backstop for the (rare) case where two writers race past `try_lock_exclusive` at the OS level.

### Alternatives considered

- **SQLite locks alone.** Adequate for correctness but the user-visible UX is a multi-second wait then a busy error, indistinguishable from a hang. Worse for scriptability.
- **Single connection pool with internal mutex.** Doesn't help — different Tome processes don't share an in-process mutex.
- **Async DB engine.** Violates the sync-only constraint.

### Open questions for review

- Whether to use `fs2` (small, well-known) or `std::fs::File` + `OS-specific advisory lock` (zero deps). Decided in Phase 1: prefer std-only if Rust 1.93 has stable `try_lock`; otherwise `fs2`.

---

## R3. Schema migration mechanics

### Decision

Schema versioning lives in a `meta` table:

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
INSERT INTO meta VALUES ('schema_version', '1');
```

Migrations are a fixed-order Rust array in `src/index/migrations.rs`:

```rust
static MIGRATIONS: &[Migration] = &[
    // Migration { from: 0, to: 1, sql: include_str!("../../migrations/v1.sql") },
    // Future Phase 2 patches append here.
];
```

On `db_open()`:

1. Read `meta.schema_version`. If the table doesn't exist, treat as `0` (fresh DB).
2. If `current == COMPILED_VERSION` — proceed.
3. If `current > COMPILED_VERSION` — return `SchemaTooNew` error. Tell the user to upgrade Tome.
4. If `current < COMPILED_VERSION` — acquire the index lockfile (R2), open a transaction, apply each pending migration in sequence, update `meta.schema_version`, commit. Log a one-line notification to the developer.

Migrations run inside a single transaction per step. WAL + `synchronous = NORMAL` means a crash mid-migration recovers the pre-step state. We do **not** support down-migrations — if Tome rolled back its schema, the user would lose data; the policy is "newer Tome opens older DBs; older Tome refuses newer DBs."

Phase 2 ships schema version 1. There is no v0→v1 migration (v0 is "no DB"). The migration framework exists from day one so adding a v2 in a future Phase-2 patch is a single-row append to the migrations array plus a SQL file under `migrations/v2.sql`.

### Rationale

Forward-only migrations inside a transaction inside the advisory lockfile boundary give the SC-015 atomicity guarantee. Refusing on backward-version-drift is conservative but constitutes the "fail clear" behaviour the constitution requires.

### Alternatives considered

- **`refinery`, `sqlx-migrate`, or `rusqlite_migration` crate.** All add 50–200 KB to the binary and a tiny amount of dependency surface. A hand-rolled 50-line migration runner is preferable for KISS. `rusqlite_migration` is the closest fit; reconsider if the migrations array grows past 5–10 entries.
- **No migrations at all — version bump invalidates the DB.** Would require users to re-embed everything on every Tome upgrade. Hostile UX.

### Open questions for review

None. Will revisit when we ship the first v2 migration.

---

## R4. Frontmatter strictness boundary

### Decision

The constitution's principle IV ("strict schemas") applies to **Tome-owned declarative input**, per the operational boundary set in spec FR-013a. Concretely:

| Input | Owner | Strictness | Parser attribute |
|---|---|---|---|
| `config.toml` | Tome | Strict | `#[serde(deny_unknown_fields)]` |
| `tome-catalog.toml` (Phase 1 manifest) | Tome | Strict | `#[serde(deny_unknown_fields)]` |
| `models/manifest.json` | Tome | Strict | `#[serde(deny_unknown_fields)]` |
| `index.db` `meta` rows | Tome | Strict — unknown keys ignored on read, unknown keys never written | enum-based key validation |
| `plugin.json` (Claude Code's plugin manifest) | Third-party | **Lenient** — unknown fields ignored without warning | `serde` default behaviour |
| `SKILL.md` YAML frontmatter | Third-party | **Lenient** — unknown fields ignored without warning; missing `name` / `description` covered by FR-011 / FR-012 fallbacks; malformed YAML covered by FR-013c | `serde_yaml` default + manual fallbacks |

### Rationale

The constitution's IV principle was written for Tome's own configuration surfaces, where strictness catches author typos before they ship. Forcing the same strictness on third-party plugin manifests would break Tome on any forward-compatible field addition to the upstream plugin format — i.e., a developer's plugin would stop working when its author added a new field that Tome didn't know about yet. That cost is paid by the user, not the contributor who introduced the strictness; the trade is wrong.

FR-013a is the spec's codification of this distinction. The constitution stays as written; the documentation in CHANGELOG and `quickstart.md` will note the operational boundary.

### Alternatives considered

- **Strict everywhere; warn-and-continue on unknown fields in third-party inputs.** Adds log noise for the developer for an upstream change they have no control over. Rejected.
- **Allow unknown fields in third-party inputs; emit a one-time warning per plugin per Tome version.** Some merit, but state management (where do we store "already warned about X"?) is friction not justified by the benefit. Rejected.

### Open questions for review

None.

---

## R5. Model artefacts — files, URLs, checksums

### Decision

**Per spec FR-019 / FR-020:** Tome downloads two models on first need. Each model is stored as a directory under `${XDG_DATA_HOME}/tome/models/<model-name>/` containing the ONNX file(s), tokenizer files, and a Tome-written `manifest.json` recording name, version, source URL, SHA-256, file list, size, and licence.

| Model | Use | Approximate disk | Licence |
|---|---|---|---|
| `bge-small-en-v1.5` (INT8 ONNX) | Embedder | ~45 MB | MIT |
| `bge-reranker-base` (INT8 ONNX) | Reranker | ~280 MB | MIT |

### Source

Models are fetched from Hugging Face's CDN. `fastembed-rs` knows the canonical URLs and checksums internally; the build that ships in Tome **pins** the model version (not "latest") so that two Tome installs of the same version produce vector-comparable indexes. The pinned manifest lives at:

```
src/embedding/models.rs   # const MODEL_REGISTRY: &[ModelEntry]
```

Each `ModelEntry` carries:

- `name: &'static str`
- `version: &'static str`
- `download_url: &'static str` (HTTPS, no auth)
- `sha256: &'static str` (lowercase hex)
- `size_bytes: u64`
- `licence: &'static str`

CI fetches the same manifest on the manual end-to-end run and re-verifies the SHA-256; if the upstream moved, CI fails loudly and the manifest is updated in a PR.

### Download semantics (atomic; FR-020a)

1. Download to `${XDG_DATA_HOME}/tome/models/<model>/.partial/<filename>`.
2. Stream-hash with SHA-256 while writing.
3. On mismatch — delete the partial directory, return `ModelChecksumMismatch` exit code.
4. On success — `fsync` then `rename(.partial → final)` atomically. Write `manifest.json` last, also atomically.

A model is "installed" iff `manifest.json` exists, every file it references exists, and the size on disk equals the recorded size. The `tome status` command runs a full SHA-256 verification on demand; `tome models list` checks file existence and size cheaply unless `--verify` is passed.

### Licence visibility

`tome models list` displays each model's licence in its own column. The first run of `tome plugin enable` (which may trigger the model download prompt) shows the size and licence in the same prompt.

### Rationale

`fastembed-rs` already knows how to fetch BGE models; we pin to a known-good version. Atomic persist matches Phase 1's pattern. Licence visibility per NFR-002 puts an honest signal in front of the user.

### Alternatives considered

- **Bundle models in the binary.** ~325 MB. Rejected immediately.
- **Use fastembed-rs's own model cache directory.** Rejected: it defaults to `~/.cache/fastembed` (or similar), which violates FR-021 (must be data dir, not cache dir). We force the cache path explicitly to our XDG data path.
- **Self-host model mirror.** Adds infrastructure with no obvious benefit. Rejected.

### Open questions for review

None.

---

## R6. Embedding pipeline cancellation

### Decision

Phase 1's SIGINT handler (exit code 8) extends to Phase 2 unchanged.

- Inside `lifecycle::enable`, the embedding loop checks the global "interrupted" flag at every skill boundary. On the next check after an interrupt, the loop breaks; the surrounding transaction is dropped (no COMMIT), SQLite rolls back automatically.
- The model download loop in `embedding::download` checks the interrupted flag after every chunk read from `reqwest::blocking::Response`. On interrupt, the `.partial` directory is deleted.
- ONNX inference for a single skill cannot be cancelled — the FFI call is atomic from our perspective. FR-053 explicitly permits this; we promise to exit "within a bounded number of seconds" of the signal, which for `bge-small-en-v1.5` CPU is sub-second.

### Rationale

Matches FR-053. No async runtime is introduced; the signal handler simply flips an `AtomicBool`.

### Alternatives considered

- **Spawn embedding on a thread; channel-cancellation.** Adds threading complexity for sub-second latency reduction. Not worth it.

### Open questions for review

None.

---

## R7. Reranker score scale and strict-mode threshold

### Decision

- `bge-reranker-base` returns unbounded logits; the spec deliberately leaves the scale unstated. The query command displays the raw float in the `score` column.
- `--strict` mode threshold is an opt-in `--min-score=<float>` flag (FR-031). The default is unset (no threshold); when `--strict` is passed without `--min-score`, the default minimum is `0.0` for reranker scores (logits) and `0.5` for embedding cosine similarity. Both are documented in the command help text and in `quickstart.md`.
- When reranking is disabled, the displayed score is the embedding cosine similarity (range -1 to 1). Human output prefixes the table with a one-line banner: "Showing embedding-similarity scores (reranker disabled)." per FR-018.

### Rationale

The reranker logit scale is model-specific; a hardcoded default is wrong for any future model. Keeping the threshold explicit and `--strict` opt-in respects FR-031 without baking in magic numbers.

### Alternatives considered

- **Normalise reranker scores to [0, 1].** Adds a calibration step that's only useful if the user cares about the absolute scale; ranking is preserved either way. Defer.

### Open questions for review

None.

---

## R8. Embedding text composition

### Decision

Per spec FR-014 (and PRD §"Embedding text composition"), the text passed to the embedder for skill record S is exactly:

```
{name}

{description}
```

Two lines, blank line between. The PRD cites embedding-strategy research that short focused text outperforms verbose context. We hash the same composition into `content_hash` for diff detection (sha256, lowercase hex, stored in `skills.content_hash`).

### Rationale

Deterministic input means a re-enable of unchanged content always produces an identical hash, which is what FR-006's "state flip with no re-embedding" relies on.

### Alternatives considered

None — fixed by spec.

### Open questions for review

None.

---

## R9. Interactive flow library

### Decision

`inquire` is the prompt library (Select, MultiSelect, Confirm). It detects TTY internally and integrates with `console`/`indicatif`.

For the interactive `tome plugin` flow:

1. **Catalog selector** — `inquire::Select` populated from the registry, with a synthetic "Quit" trailing entry.
2. **Plugin browser** — `inquire::Select` with a "Back" trailing entry.
3. **Plugin view** — rendered as a static `comfy-table` plus a follow-up `inquire::Select` action prompt.
4. **Confirmation** — `inquire::Confirm` with default `false` for destructive operations (disable, model remove).

`inquire` is verified compatible with `indicatif` and `owo-colors` via their respective `console` backends.

`tome plugin` with no subcommand checks `std::io::stdout().is_terminal()` AND `std::io::stdin().is_terminal()` at entry. Either being false exits with `NotATerminal` (Phase 2 exit code reserved per FR-051).

### Rationale

`inquire` is the most maintained Rust prompt library, supports `console`-based terminal detection, and has small surface area. `dialoguer` is the main alternative; broadly equivalent but `inquire` has better keyboard navigation and a clean API.

### Alternatives considered

- **`dialoguer`** — would also work; `inquire` chosen for keyboard UX and maintenance activity.
- **Build prompts on top of `crossterm` directly** — too much code for the value.

### Open questions for review

None.

---

## R10. Stub embedder design

### Decision

A `#[cfg(test)]` `StubEmbedder` and `StubReranker` are provided behind the same trait surfaces as the real implementations:

```rust
// src/embedding/mod.rs (sketch)
pub trait Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;  // 384-dim
}

pub trait Reranker {
    fn rerank(&self, query: &str, candidates: &[Candidate]) -> Result<Vec<Scored>, RerankerError>;
}
```

The stub's behaviour:

- `embed(text)` = deterministic 384-dim vector derived from a SHA-256 of `text` (first 384 bytes of hash, treated as f32 little-endian after a fixed transform; or a simpler XOR-based hash that distributes well — exact construction in implementation). Same input always produces the same vector; different inputs produce vectors whose cosine similarity is < 0.99.
- `rerank(query, candidates)` = identity: scores returned are the input embedding-similarity scores. Tests that exercise reranker behaviour use a second stub variant `ReverseStubReranker` that reverses the input order, so tests can distinguish "did the reranker stage run?" from "did the embedder stage run?".

### Rationale

The constitution's principle VIII permits trait-shaped abstraction at the boundary of an external system. ONNX Runtime + the BGE models is squarely an external system. The stub is deterministic and small, lets every test assert exact retrieval results, and removes the need for the CI environment to have ~325 MB of model files. The complexity-tracking justification in plan.md captures this tradeoff.

### Alternatives considered

- **Real models in CI.** ~30+ min per CI run, network dependency, licence ambiguity around redistribution. Rejected.
- **Vendor a tiny test model.** Possible but adds licence and maintenance friction.

### Open questions for review

None.

---

## R11. CLI flag set and naming

### Decision (cross-reference for the contracts)

Global flags (inherited from Phase 1, unchanged):

- `--json` — structured output.
- `--no-color` / `NO_COLOR` env — disable colour.
- `-v` / `-vv` — verbosity for stderr logs.

New global flag:

- `--strict-tty` (default off) — for scripts that want a hard error rather than a graceful degradation when stdout is not a TTY. Not in the spec; available because it's free and makes scripting easier.

Per-command flags:

| Command | New flags |
|---|---|
| `tome plugin enable <id>` | (none — `--force` not needed; enable is non-destructive in spec model) |
| `tome plugin disable <id>` | `--force` |
| `tome plugin list` | `--catalog <name>`, `--enabled-only` |
| `tome plugin show <id>` | (none) |
| `tome query <text>` | `--top-k N` (default 10, FR-027), `--catalog <name>`, `--plugin <name>`, `--no-rerank`, `--strict`, `--min-score <float>` |
| `tome models download` | `--force` |
| `tome models list` | `--verify` (full SHA-256 check) |
| `tome models remove <name>` | `--force` |
| `tome reindex [scope]` | `--force` |
| `tome status` | (none — exit code indicates health) |
| `tome catalog remove <name>` | `--force` (extended semantics — cascades on Phase 2) |

### Rationale

Convention from Phase 1: `--force` is the universal "skip prompts and accept destructive defaults" flag. Reused everywhere. Same flag name everywhere per Phase 1 FR-021.

### Open questions for review

None.

---

## R12. Catalog refresh integration

### Decision

`tome catalog update` keeps its Phase 1 behaviour (git fetch / reset against the tracked ref). Phase 2 extends it with a post-refresh hook:

1. After every successful Git operation against a catalog, for every plugin in that catalog that is currently `enabled` in the index:
   1. Walk the plugin's skills directory.
   2. For each skill: compute the new `content_hash` from the (name, description) composition (R8).
   3. Compare to the stored `content_hash` in the index.
   4. If different → re-embed that single skill, update its row, update `indexed_at`.
   5. If a skill that used to exist is no longer present → delete its row (skill removed upstream).
   6. If a new skill has appeared → embed and insert.
2. If the plugin's `plugin.json` is missing or unparsable after refresh → mark plugin as disabled, drop all its skill rows, print a warning naming the plugin (FR-033). User opted in once; the plugin disappeared upstream; we are not silent about it.

The summary table at end of refresh shows: catalogs refreshed, plugins changed, skills added / modified / removed.

### Rationale

Matches FR-032 / FR-033 exactly. The work is bounded by what actually changed — small refreshes stay small.

### Open questions for review

None.

---

## R13. Catalog removal cascade

### Decision

`tome catalog remove <name>` (Phase 1 surface) gets a Phase 2 pre-check:

- Query the index for any `enabled = 1` row whose `catalog = <name>`.
- If any → refuse with `CatalogHasEnabledPlugins` exit code, list the enabled plugins, point at `tome plugin disable`.
- If `--force` is passed → for each enabled plugin in that catalog, run the disable flow (drop rows from `skills` and `skill_embeddings`), then proceed with the Phase 1 catalog-removal logic.

The cascade happens inside the index advisory lockfile boundary, so a concurrent reader sees pre-cascade state until the lockfile is released.

### Rationale

Matches FR-036 / US7 exactly.

### Open questions for review

None.

---

## R14. CI binary-size assertion

### Decision

`ci.yml` gains a step (Linux job only — macOS may have different stripped sizes; Linux is the conservative platform):

```yaml
- name: Verify binary size
  run: |
    cargo build --release --locked
    SIZE_BYTES=$(stat -c%s target/release/tome)
    SIZE_MB=$(echo "scale=2; $SIZE_BYTES / 1048576" | bc)
    echo "Binary size: ${SIZE_MB} MB"
    if (( SIZE_BYTES > 10485760 )); then
      echo "::error::Release binary ${SIZE_MB} MB exceeds 10 MB cap"
      exit 1
    fi
```

We measure `target/release/tome` *after* the release profile (which already includes `strip = "symbols"`), so the assertion is against the shipped size.

### Rationale

NFR-001 needs an automatic gate. A human "should be under 10 MB" rule is too easy to drift. The CI step is ~5 s overhead.

### Open questions for review

None.

---

## R15. MSRV verification under new deps

### Decision

Verify each new dep's MSRV against Phase 1's pinned `rust-version = "1.93"`:

| Crate | Declared MSRV at time of writing | OK? |
|---|---|---|
| `rusqlite` 0.31+ | 1.74 | ✓ |
| `sqlite-vec` 0.x | n/a (vendored C) | ✓ |
| `fastembed-rs` 4.x | typically ≥ 1.75 — verify in implementation | likely ✓ |
| `ort` 2.x | typically ≥ 1.78 — verify in implementation | likely ✓ |
| `indicatif` 0.17 | 1.66 | ✓ |
| `comfy-table` 7 | 1.70 | ✓ |
| `owo-colors` 4 | 1.70 | ✓ |
| `inquire` 0.7 | 1.70 | ✓ |
| `reqwest` 0.12 (blocking, rustls-tls) | 1.75 | ✓ |

CI's MSRV row runs against 1.93; any future dep bump that lifts MSRV requires a constitutional amendment of the rust-version line, which is a deliberate friction we want.

### Open questions for review

`fastembed-rs` and `ort` MSRVs change relatively often; verify on first commit and re-verify on every Renovate bump.

---

## Summary

All NEEDS CLARIFICATION items from the spec are resolved:

- **Binary-size strategy** is concrete (R1).
- **Concurrency model** is concrete (R2).
- **Schema migration** is concrete (R3).
- **Strictness boundary** is concrete (R4).
- **Model artefacts and licences** are concrete (R5).
- **Cancellation** is concrete (R6).
- **Strict-mode thresholds** are concrete (R7).
- **Embedding text composition** is concrete (R8).
- **Interactive flow library** is concrete (R9).
- **Stub embedder design** is concrete (R10).
- **CLI flag set** is concrete (R11).
- **Catalog refresh / removal integration** is concrete (R12 / R13).
- **CI binary-size gate** is concrete (R14).
- **MSRV** is verified (R15).

Phase 1 design proceeds.
