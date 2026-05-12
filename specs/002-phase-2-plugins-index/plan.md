# Implementation Plan: Phase 2 — Plugin Enable/Disable and Local Skill Index

**Branch**: `002-phase-2-plugins-index` | **Date**: 2026-05-11 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `specs/002-phase-2-plugins-index/spec.md`
**Source PRD** (HOW reference): [PRDs/phase-2.md](../../PRDs/phase-2.md)
**Constitution**: [CONSTITUTION.md](../../CONSTITUTION.md) — v1.0.1

## Summary

Phase 2 layers two new capabilities on top of the Phase 1 CLI:

1. **Plugin enable / disable lifecycle.** The developer opts into plugins from registered catalogs by their `<catalog>/<plugin>` identifier — non-interactively via `tome plugin enable | disable | list | show`, or interactively via `tome plugin` with no subcommand. Enable parses skill metadata, runs the embedding pipeline, and inserts records into a local search index. Disable flips an `enabled` flag without losing the embeddings. Re-enable of unchanged content is a state flip, not re-embedding.
2. **Local semantic skill index.** A single per-user SQLite database file with the `sqlite-vec` virtual-table extension stores skill rows plus their 384-dimensional embeddings. The index is queried by `tome query <text> [--top-k N] [--catalog X] [--plugin Y] [--no-rerank] [--json]` and ranked by `bge-reranker-base` (with `bge-small-en-v1.5` for the embedder). Models live in `${XDG_DATA_HOME}/tome/models/`, are downloaded on first need via `tome models download`, and are verified by SHA-256.

The technical approach stays inside Phase 1's constitutional envelope: sync Rust, shell out where the host system has it (no async runtime even though embedding workloads are compute-bound), strict TOML for tool-owned inputs, lenient parsing for third-party inputs, closed `TomeError` enum extended with Phase 2 variants, atomic state mutations, credential scrubbing intact. New libraries: `rusqlite` (bundled SQLite), the vendored `sqlite-vec` C extension compiled in via a build script, `fastembed-rs` for ONNX inference (bringing `ort` transitively), and the presentation set `indicatif` / `comfy-table` / `owo-colors` / `inquire`.

The most consequential design decisions for this plan are: (1) binary-size engineering — `ort` is the load-bearing dependency and how it links determines whether NFR-001 (the 10 MB stripped cap) is achievable; (2) the SQLite concurrency model — WAL + advisory lockfile + bounded `busy_timeout` produces the FR-040 contract; (3) the closed error-set extension — eighteen new enumerated variants, each with its own exit code, derived directly from FR-048.

## Technical Context

**Language/Version**: Rust stable. MSRV inherited from Phase 1 (`rust-version = "1.93"` per `Cargo.toml`). Phase 2 dependencies' MSRVs must be verified to stay at or below this in research; `fastembed-rs` and `ort` are the likely tighteners.

**Primary Dependencies** (additions on top of Phase 1's eight):
- `rusqlite` (`bundled` feature) — statically linked SQLite, no system dependency.
- `sqlite-vec` — vendored as a single C file, compiled via a Cargo `build.rs`; static link into the same SQLite that `rusqlite` ships.
- `fastembed-rs` — ONNX-based embedder/reranker wrapper. Transitively pulls `ort` (ONNX Runtime). CPU-only execution provider; CUDA / CoreML / DirectML providers explicitly disabled.
- `indicatif` — progress bars and spinners.
- `comfy-table` — table rendering for human output.
- `owo-colors` — terminal colours, native `NO_COLOR` support.
- `inquire` — interactive Select / MultiSelect / Confirm prompts.
- `console` — transitive (via `indicatif`); used for TTY feature detection.
- `reqwest` (`blocking` feature, `rustls-tls`, no default features) — model downloads (HTTPS only, no system OpenSSL).
- `flate2` and/or `tar` — only if model artefacts ship as archives; resolved in Phase 0 research.

**Storage**:
- Configuration: unchanged from Phase 1 (`${XDG_CONFIG_HOME}/tome/config.toml`).
- Catalog cache: unchanged from Phase 1 (`${XDG_DATA_HOME}/tome/catalogs/<sha256-of-source-url>/`).
- **New** — index database: `${XDG_DATA_HOME}/tome/index.db`. Single global file. WAL mode at open. Schema versioned via `meta` table. Vector storage via `sqlite-vec` virtual table `skill_embeddings(skill_id INTEGER PRIMARY KEY, embedding FLOAT[384])`.
- **New** — model directory: `${XDG_DATA_HOME}/tome/models/{embedder|reranker}/` containing the ONNX file(s), tokenizer config, and a Tome-owned `manifest.json` with name/version/source URL/SHA-256/size. Data dir not cache dir — OS cache-cleanup must not sweep them.
- **New** — index lockfile: `${XDG_DATA_HOME}/tome/index.lock` for advisory write locking on top of SQLite's own WAL/busy-timeout semantics. See research for full concurrency model.

**Testing**: `cargo test`, extending Phase 1's integration discipline.
- New integration suites: `tests/plugin_enable.rs`, `tests/plugin_disable.rs`, `tests/plugin_list.rs`, `tests/plugin_show.rs`, `tests/query.rs`, `tests/models.rs`, `tests/reindex.rs`, `tests/catalog_update_reindex.rs`, `tests/catalog_remove_cascade.rs`, `tests/status.rs`.
- New crosscutting suites: `tests/concurrency.rs` (two-process invocations against a `TempDir` data dir, asserting reader-during-writer succeeds, second writer waits then exits with the busy code), `tests/schema_migrations.rs` (forward migration in WAL boundary, rollback on injected failure), `tests/atomicity.rs` extension (interrupt during enable; interrupt during model download).
- Real model fixtures are too heavy for CI. Use a **stub embedder** behind a `#[cfg(test)]` trait boundary: returns deterministic 384-dim vectors derived from a hash of the input. Reranker stub returns inputs unchanged. One end-to-end test on a developer machine with the real model gates SC-001 / SC-002 manually; CI does not download real models.
- Property-style tests for the frontmatter parser (table-driven: missing name, missing description, both missing, malformed header, valid + extra fields).

**Target Platform**: macOS arm64 and Linux x86_64 — same CI matrix as Phase 1, plus `{macos-latest, ubuntu-latest}` cross-checked for the `ort`/`fastembed-rs` native-deps build.

**Project Type**: Single. One binary crate `tome` (workspace splitting still deferred — Phase 2 does not justify the friction).

**Performance Goals** (from spec SCs):
- Enable a small plugin (~10 skills) including model warm-up: under 10 s on a recent laptop (SC-001).
- Top-3 retrieval quality on a representative query: relevant skill in top 3 (SC-002).
- Re-enable of unchanged content: under 1 s (SC-004).
- Status command: under 200 ms in the healthy case (no SC but a NFR by Unix-tool convention).

**Constraints**:
- Release binary stripped: ≤ 10 MB. Hard ceiling per constitution + NFR-001. The plan's binary-size strategy is documented in research and verified in CI via a `du -sh` assertion on `target/release/tome`.
- Sync only — no `tokio`, no `async`. ONNX inference is a synchronous in-process FFI call; model downloads use `reqwest`'s `blocking` API.
- Closed-error-set principle from Phase 1 holds. Eighteen new variants per FR-048; no generic `Other`.
- Atomic state mutations. Index DB writes via SQLite transactions; model directory mutations via `tempfile::persist` (same pattern as Phase 1 registry writes).
- Credential scrubbing extends to any new Git-output surfaces and to model download URLs (which may carry tokens in some hosting configs).
- Sensitive material policy: model download URLs themselves are not secret, but query strings on them (rare, but possible for signed URLs) MUST be scrubbed before logging or surfacing in errors. Use the same scrubber.

**Scale/Scope**:
- Catalog count per user: 1–20 (carried from Phase 1).
- Enabled plugins per user: 1–50 (a heavy power-user scenario).
- Skills per plugin: typically 5–30; pathological case up to ~200.
- Index size: ~1.6 kB per skill row + 1.5 kB per 384-dim embedding ≈ 3 kB per skill. 50 plugins × 30 skills ≈ 4.5 MB. Negligible vs OS overhead.
- Embedding latency: 5–20 ms per skill on bge-small-en-v1.5 CPU. 1000 skills ≈ 5–20 s total — bounded by hardware, not Tome.

## Constitution Check

*GATE: must pass before Phase 0. Re-checked after Phase 1.*

| # | Principle | Status | How this plan satisfies it |
|---|---|---|---|
| I | Unix Philosophy | ✓ | Every Phase 2 command keeps the Phase 1 convention: human form on stdout, errors to stderr, `--json` global flag (FR-041), `NO_COLOR` honoured, TTY auto-detection (FR-046). Each new subcommand has one purpose. Interactive `tome plugin` is the explicit exception — guarded by FR-051 (refuses without a TTY). |
| II | Predictable Exit Codes (NON-NEGOTIABLE) | ✓ | FR-048 enumerates 18 new failure classes; each gets its own variant in the closed `TomeError` enum and its own dedicated exit code. Phase 1 codes are untouched (FR-047). Integration tests assert exit code per category. |
| III | Scriptable by Default | ✓ | Every interactive flow has a non-interactive subcommand of equivalent power (FR-052). Non-interactive callers without `--force` get a clear error rather than a hang (FR-051, FR-007). Models prompt in TTY contexts, error with a dedicated code in non-TTY (FR-024, FR-025). |
| IV | Strict Schemas, Helpful Errors | ✓ | Strictness is split (FR-013a): **Tome-owned** inputs (`models/manifest.json`, the `meta` table, index DB rows) parse strictly with `#[serde(deny_unknown_fields)]`. **Third-party** inputs (SKILL.md frontmatter, plugin.json) parse leniently — forward-compatible additions don't break Tome. This is a deliberate amendment of the Phase 1 blanket rule; documented in research and reflected by attribute placement. The constitution's principle IV requires strictness for "declarative input"; the spec FR-013a defines the precise boundary. No conflict. |
| V | Fail Fast, Fail Clear | ✓ | `anyhow::Context` at every boundary. New error display messages format "what / where / next" (e.g., "Embedding model `bge-small-en-v1.5` is missing. Run `tome models download` or `tome models download --force`."). Schema drift, model drift, and DB-busy each name what changed and what to do. |
| VI | KISS / YAGNI | ✓ | One embedder, one reranker, one DB file, one model directory. No pluggable backends, no async, no workspace split. Reranker on by default (PRD decision); `--no-rerank` is debug-only. Single global index — workspaces deferred to Phase 3. Reuse Phase 1's `tempfile::persist` pattern verbatim. |
| VII | Modular by Boundary | ✓ | New modules organised by capability: `src/index/` (db open, schema, migrations, vector ops), `src/embedding/` (model registry, embedder, reranker, stub trait), `src/plugin/` (manifest parser, SKILL.md frontmatter, lifecycle), `src/commands/plugin.rs`, `src/commands/query.rs`, `src/commands/models.rs`, `src/commands/reindex.rs`, `src/commands/status.rs`. Each module's public surface is enumerated; no cross-module backdoors. `thiserror` inside modules; `anyhow` at the application boundary. |
| VIII | Test What Matters | ✓ | Integration tests per CLI command. No mocks of the filesystem, the DB engine, or Git. The embedder/reranker is the lone exception — gated behind a `#[cfg(test)]` trait to keep CI fast and deterministic; one manual end-to-end with the real model verifies SC-001/SC-002 outside CI. This deviation is justified in Complexity Tracking. |
| IX | Conventional Commits | ✓ | Unchanged. `cocogitto` in lefthook `commit-msg` already gates Phase 1 commits and will gate Phase 2. |
| X | CI Gates Every Merge | ✓ | `ci.yml` extends to install build deps for `ort` on Ubuntu (libstdc++ static link via `ort`'s default; document if anything is needed beyond default). Binary-size CI step extended to assert ≤ 10 MB. `security.yml` unchanged. Renovate continues; new deps inherit the policy. |
| XI | Documentation Is Part of the Change | ✓ | `quickstart.md` updated for Phase 2 commands. README gets a Phase 2 section. Command help-text for every new subcommand. CHANGELOG entries. Constitution-relevant note about FR-013a (lenient parsing of third-party inputs) is recorded in research and CHANGELOG, not in the constitution itself — the constitution principle stays as written; FR-013a is the operational boundary for "declarative input." |
| XII | Inherit, Don't Reimplement | ✓ | SQLite — we inherit the world's most-deployed embedded DB rather than write our own. Static linkage means we keep the inheritance property (no system SQLite version drift) without sacrificing it on user installs. `sqlite-vec` is a thin extension — far less code than reimplementing HNSW or IVF. `fastembed-rs` wraps `ort` rather than us writing tokenizer + ONNX glue. Git remains shelled out. Where the host does the job, we shell out; where it doesn't, we statically link a minimal upstream rather than reimplement. |
| XIII | Never Log Secrets | ✓ | Credential scrubber from Phase 1 (`src/catalog/git.rs::scrub_credentials`) is extended to a process-wide boundary applied to: Git stderr (Phase 1), model download URL display (new), and `reqwest` error chains (new). Unit tests cover signed-URL query-string scrubbing, HTTPS-with-token URLs, and the existing Phase 1 cases. |

**Operational Constraints check**:
- Lints unchanged. New code must pass `clippy -D warnings`.
- **Dependencies** — eight new direct, with written justification per crate:

  | Crate | Justification | Licence | Binary impact (estimated) |
  |---|---|---|---|
  | `rusqlite` (bundled) | Embed SQLite without a system dependency. Required by FR-038. | MIT | ~1.1 MB |
  | `sqlite-vec` | Vector-search virtual table; vendored single-file C source. Required by FR-038. | Apache-2.0 / MIT | ~250 KB |
  | `fastembed-rs` | ONNX-based embedder + reranker wrapper. Required by FR-014 / FR-018. | Apache-2.0 | thin wrapper |
  | `ort` (transitive) | ONNX Runtime bindings. CPU EP only. Required by `fastembed-rs`. | MIT | ~5–8 MB (load-bearing — see binary-size research) |
  | `indicatif` | Progress bars / spinners. Required by FR-042 / FR-043. | MIT | ~200 KB |
  | `comfy-table` | Table rendering. Required by FR-044. | MIT | ~80 KB |
  | `owo-colors` | Terminal colours + `NO_COLOR`. Required by FR-045. | MIT | ~30 KB |
  | `inquire` | Interactive prompts. Required by FR-050. | MIT | ~300 KB |
  | `reqwest` (`blocking`, `rustls-tls`, no defaults) | Model downloads. Required by FR-019. | Apache-2.0 / MIT | ~700 KB (rustls only) |

  Every crate is inside the constitution's licence allowlist (MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode-DFS-2016). `cargo-deny check` enforces. Renovate-managed.

- **Async** — sync only. Confirmed: `reqwest::blocking`, `rusqlite` is sync, `fastembed-rs` exposes a sync API. No `tokio` introduction.
- **Binary size** — load-bearing concern. The plan and research commit to: (a) `ort` linked **statically** with only the CPU execution provider; (b) `panic = "abort"` + `lto = "thin"` + `codegen-units = 1` for release; (c) `strip = "symbols"` in `Cargo.toml`; (d) optional model downloads (NOT bundled in the binary). The CI binary-size step asserts ≤ 10 MB stripped on both `macos-latest` and `ubuntu-latest`. If the cap is breached, the plan revises — per NFR-001, the cap cannot be waived. See research §Binary size budget for the worst-case projection (~9.2 MB) and contingencies.
- **Paths** — XDG-aware via `directories`; index DB and models dir live under `${XDG_DATA_HOME}` not `${XDG_CACHE_HOME}` (FR-021).
- **Licensing** — MIT OR Apache-2.0 unchanged. Downloaded model files: BGE family is MIT — documented in `tome models list` output (FR-022 mention; resolved in research).

**Result: PASS.** One deviation needs justification in Complexity Tracking: the embedder/reranker stub-trait in `#[cfg(test)]` builds (constitution principle VIII says "mocks are a last resort"). Justification below.

## Project Structure

### Documentation (this feature)

```text
specs/002-phase-2-plugins-index/
├── plan.md              # This file (/sdd:plan output)
├── spec.md              # Feature specification (/sdd:specify output)
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output — CLI command contracts (not HTTP)
│   ├── plugin-commands.md       # enable / disable / list / show / interactive
│   ├── query.md
│   ├── models-commands.md       # download / list / remove
│   ├── reindex.md
│   ├── status.md
│   ├── catalog-extensions.md    # Phase 2 additions to catalog update / remove
│   ├── version-output.md        # --version surface extension
│   ├── exit-codes.md
│   └── index-schema.sql
├── checklists/
│   └── requirements.md  # Spec quality checklist (PASS)
└── tasks.md             # Phase 2 output of /sdd:tasks (NOT created here)
```

### Source code (repository root)

New modules in **bold**; Phase 1 modules left intact where untouched.

```text
tome/                                # repo root
├── Cargo.toml                       # extended: new deps, profile.release tuning, build script for sqlite-vec
├── Cargo.lock
├── build.rs                         # NEW — compiles vendored sqlite-vec.c against the rusqlite-bundled headers
├── vendor/
│   └── sqlite-vec/                  # NEW — pinned single-file C source + LICENSE
│       ├── sqlite-vec.c
│       ├── sqlite-vec.h
│       └── LICENSE
├── rust-toolchain.toml
├── lefthook.yml
├── deny.toml                        # extended: new transitive licences enumerated
├── rustfmt.toml
├── clippy.toml
├── _typos.toml
├── LICENSE-MIT
├── LICENSE-APACHE
├── README.md                        # extended: Phase 2 section
├── CHANGELOG.md                     # extended: Phase 2 entries
├── .editorconfig
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml                   # extended: binary-size assertion ≤ 10 MB
│       └── security.yml             # unchanged
├── src/
│   ├── main.rs                      # extended: new dispatch arms
│   ├── lib.rs                       # extended: re-exports
│   ├── cli.rs                       # extended: new subcommands + global flags carry through
│   ├── config.rs                    # unchanged
│   ├── paths.rs                     # extended: index_db_path(), models_dir(), index_lock_path()
│   ├── output.rs                    # extended: table rendering, progress bar wrappers, status formatter
│   ├── logging.rs                   # unchanged
│   ├── error.rs                     # extended: 18 new TomeError variants + ExitCode mapping
│   ├── catalog/                     # Phase 1
│   │   ├── manifest.rs              # unchanged
│   │   ├── store.rs                 # extended: catalog-remove cascade hook calls plugin::cleanup
│   │   └── git.rs                   # extended: scrub_credentials now also runs over model URLs
│   ├── commands/
│   │   ├── catalog.rs               # extended: update calls plugin::reindex_changed; remove enforces FR-036
│   │   ├── plugin.rs                # NEW — enable, disable, list, show, interactive
│   │   ├── query.rs                 # NEW
│   │   ├── models.rs                # NEW — download, list, remove
│   │   ├── reindex.rs               # NEW
│   │   └── status.rs                # NEW
│   ├── plugin/                      # NEW capability module
│   │   ├── mod.rs                   # public surface
│   │   ├── manifest.rs              # plugin.json parser (lenient)
│   │   ├── frontmatter.rs           # SKILL.md YAML header parser (lenient + fallbacks)
│   │   ├── components.rs            # component-count walks (skills/agents/commands/hooks/.mcp.json)
│   │   ├── identity.rs              # <catalog>/<plugin> address parsing
│   │   └── lifecycle.rs             # enable/disable orchestrator (atomic at plugin granularity)
│   ├── index/                       # NEW capability module
│   │   ├── mod.rs                   # public surface
│   │   ├── db.rs                    # rusqlite open, WAL config, busy_timeout
│   │   ├── schema.rs                # schema version meta, CREATE statements
│   │   ├── migrations.rs            # forward-migrations table (currently empty; first migration is v1→v1)
│   │   ├── vec_ext.rs               # sqlite-vec extension load (compiled in via build.rs)
│   │   ├── skills.rs                # insert / update / disable / hash-diff queries
│   │   ├── query.rs                 # vector search + reranker invocation
│   │   ├── meta.rs                  # embedder/reranker drift detection
│   │   ├── integrity.rs             # PRAGMA integrity_check wrapper for status
│   │   └── lock.rs                  # advisory lockfile (fs-locked tempfile)
│   ├── embedding/                   # NEW capability module
│   │   ├── mod.rs                   # public surface; defines the Embedder + Reranker traits
│   │   ├── fastembed.rs             # fastembed-rs-backed implementation
│   │   ├── stub.rs                  # #[cfg(test)] deterministic stub
│   │   ├── registry.rs              # model manifest.json (strict)
│   │   ├── download.rs              # reqwest::blocking + SHA-256 verify + atomic persist
│   │   └── runtime.rs               # ort EnvironmentBuilder + CPU EP only
│   └── presentation/                # NEW capability module — wraps comfy-table, indicatif, owo-colors, inquire
│       ├── mod.rs
│       ├── tables.rs                # comfy-table helpers
│       ├── progress.rs              # indicatif wrappers (auto-suppress for non-TTY stderr)
│       ├── colour.rs                # owo-colors + NO_COLOR detection
│       └── prompt.rs                # inquire wrappers (refuse on non-TTY)
└── tests/
    ├── catalog_*.rs                 # Phase 1 (carried forward; one extended for cascade)
    ├── manifest_strictness.rs       # Phase 1 (extended: model manifest is strict)
    ├── path_validation.rs           # Phase 1
    ├── exit_codes.rs                # extended: 18 new exit codes
    ├── scrubbing.rs                 # extended: model URL scrubbing
    ├── atomicity.rs                 # extended: interrupt during enable, interrupt during model download
    ├── plugin_enable.rs             # NEW
    ├── plugin_disable.rs            # NEW
    ├── plugin_list.rs               # NEW
    ├── plugin_show.rs               # NEW
    ├── plugin_interactive.rs        # NEW — uses inquire's test harness or pty
    ├── query.rs                     # NEW
    ├── models_download.rs           # NEW
    ├── models_list.rs               # NEW
    ├── models_remove.rs             # NEW
    ├── reindex.rs                   # NEW
    ├── status.rs                    # NEW
    ├── catalog_update_reindex.rs    # NEW
    ├── catalog_remove_cascade.rs    # NEW
    ├── concurrency.rs               # NEW — two-process readers / second-writer-busy
    ├── schema_migrations.rs         # NEW
    ├── version_output.rs            # NEW — assert embedder + reranker named in --version
    ├── frontmatter.rs               # NEW — table-driven parser cases
    └── fixtures/
        ├── sample-catalog/          # Phase 1
        └── sample-plugin/           # NEW — minimal plugin with .claude-plugin/plugin.json + skills/*/SKILL.md
```

**Structure Decision**: Single binary crate, capability-organised modules. Phase 2 adds three new capability modules (`plugin`, `index`, `embedding`) plus a `presentation` sibling that wraps the four UI dependencies. The Phase 1 module layout is preserved and extended in place where Phase 2 changes behaviour (`error.rs`, `catalog/store.rs`, `catalog/git.rs`, `commands/catalog.rs`, `paths.rs`, `output.rs`, `cli.rs`). No workspace split — the constitution's "defer until justified by code size" still applies; rough estimate puts Phase 2 around 5–7 kLOC of Rust which a single crate handles cleanly.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| `#[cfg(test)]` stub for the embedder + reranker traits | CI must run integration tests for `plugin enable`, `query`, `reindex`, `catalog update` with real DB I/O but bounded latency. The real models are ~325 MB total and load in 1–3 s; downloading them in CI would cost 30+ minutes per run and require network access in the test environment, both of which would either invalidate the matrix or force us to bundle fixtures we cannot legally redistribute without licence audit. The stub is deterministic: a SHA-256-derived 384-dim vector lets every test assert exact retrieval results. | Real-model CI rejected: too slow (30+ min CI), network-dependent, licence-redistribution friction. In-process mock of `rusqlite` or `sqlite-vec` rejected: violates "no mocking of the filesystem / DB engine" (we use a real `TempDir`-backed DB throughout). The only mock is the embedder, which is the legitimate "external system" boundary that principle VIII permits a trait-shaped abstraction for. One manual end-to-end run with the real model on a developer machine verifies SC-001 and SC-002 outside CI. |

## Plan history

| Date | Event |
|---|---|
| 2026-05-11 | Initial plan written (this commit). |
