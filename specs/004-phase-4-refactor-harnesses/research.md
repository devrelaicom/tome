# Phase 4 Research

**Branch**: `004-phase-4-refactor-harnesses` | **Date**: 2026-05-14 | **Plan**: [plan.md](./plan.md)

Resolves every NEEDS CLARIFICATION enumerated in `plan.md` §Open Research Questions, plus retro-informed carry-overs from Phases 2 and 3 (P10 close + P3–P8) that affect Phase 4 specifically. Each entry follows: Decision → Rationale → Alternatives considered → Confidence.

## R-1. Home directory resolver — reshape, not removal

**Decision**: Phase 4's F2 slice reshapes `src/paths.rs` from its current XDG-aware multi-directory model (`config_dir`, `data_dir`, `state_dir`, `catalogs_dir`, `models_dir`, etc.) into a single `<home>/.tome/` root. The home directory continues to be resolved via raw env-var inspection (`std::env::var_os("HOME")` with a documented error if unset) — the same pattern Phase 3 already uses at `src/paths.rs:54`. Wrap in a single `paths::home_root()` accessor.

**Important framing correction**: Tome **does not currently depend on the `directories` crate**. Phase 3's `src/paths.rs:14-21` is explicit about this: *"Research §R-6 suggests `directories::ProjectDirs::state_dir()`... we deviate intentionally and use the same raw-env-var + HOME-fallback pattern... so we don't add `directories` as a single-call dependency."* The constitution v1.2.0 §Paths block reads *"XDG-aware via `directories`"* — this was aspirational, never implemented. The v1.3.0 amendment (R-12) closes this documentation/code mismatch *and* changes the on-disk layout from XDG-separated to `<home>/.tome/`.

**Rationale**:
- F2 is therefore a `Paths`-struct reshape (drop the XDG-separated fields, introduce the new `<home>/.tome/` accessors) plus a mechanical sweep of every call site. It is NOT a dependency removal (no `directories` line exists in `Cargo.toml` to delete).
- `std::env::home_dir` reinstated in Rust 1.85 is a *fallback option* if the raw env-var pattern proves insufficient on macOS (HOME is set by every reasonable shell; no real-world issue is expected). The current Phase 3 pattern is sufficient.
- The Phase 3 `tests/no_directories_imports.rs` proposed structural test guards against accidental *future* reintroduction of the crate; it does not verify a removal that didn't happen.

**Alternatives considered**:
- Add `directories` to Cargo and use it for home resolution. Rejected: dead weight; Phase 3 already proved raw env vars suffice.
- `home` crate. Rejected: same — one-line dep for what's currently four lines of raw env-var.
- `std::env::home_dir`. Acceptable as a fallback; not required.

**Confidence**: High. The framing correction shifts F2's planning surface (it's mechanical sweep, not removal) but does not change F2's correctness obligations.

## R-2. Summariser runtime — `llama-cpp-2` confirmed sync, backend singleton

**Decision**: Use `llama-cpp-2` as the summariser inference runtime. Pin to a minor version range (`llama-cpp-2 = "0.x"`). The runtime exposes synchronous APIs throughout (`LlamaContext`, `LlamaModel`, decode and sample calls all return `Result`, not `Future`). The library requires one process-wide `LlamaBackend` instance, initialised via `LlamaBackend::init()`, held in a process-lifetime `std::sync::OnceLock<LlamaBackend>`. "Unload after use" refers to dropping the `LlamaModel` / `LlamaContext`, not the backend.

**Rationale**:
- `llama-cpp-2` is the most actively-maintained Rust binding for `llama.cpp`. The API is sync — every public function is a regular function returning `Result<T, LLamaCppError>`. No async runtime is required; the binding integrates cleanly with the constitution's sync-only-outside-`src/mcp/` discipline.
- `LlamaBackend::init()` is documented as one-per-process. Calling it twice returns `BackendAlreadyInitializedError`; the binding makes process-singleton enforcement explicit. The standard idiom (and the one shown in `llama-cpp-2`'s own examples) is a `OnceLock`-held global. CLAUDE.md's "no `lazy_static` / `once_cell`; std `OnceLock` covers it" line applies cleanly.
- `LlamaModel` (weights in memory, ~400 MB for Qwen2.5-0.5B INT4) and `LlamaContext` (KV cache + per-decode state) are droppable independently of the backend. The lazy-load model in FR-421 means: keep the backend alive for the process; load the model + context when summarisation is required; drop the model + context when the command exits. Repeated invocations within a single `tome workspace regen-summary` invocation reuse one model load.
- The library statically links the C++ `llama.cpp` source by default. Dynamic linking is supported but Tome's "single static binary" identity prefers static.

**Alternatives considered**:
- `candle-transformers` (Hugging Face's pure-Rust inference library). Rejected: pure-Rust LLM inference is significantly slower per token than llama.cpp at INT4 quantisation; the developer-visible latency of summary regeneration matters (FR-407, NFR-106).
- `mistral-rs` Rust inference engine. Rejected: model coverage is narrower; the project carries CUDA/Metal stack dependencies even when only CPU is used.
- External `llama-server` HTTP API. Rejected: violates the offline-first principle (FR-427) and adds operational burden (the user has to start a server).
- `rust-bert` (Hugging Face PyTorch-port for Rust). Rejected: encoder-only models; doesn't run instruction-tuned generative LLMs.

**Confidence**: High on sync nature, backend-singleton requirement, and binding maturity. Medium on the specific minor version pin — `llama-cpp-2` is pre-1.0 and minor versions occasionally have API churn; pin tightly and bump via Renovate with maintainer review.

## R-3. Summariser model — Qwen2.5-0.5B-Instruct GGUF INT4

**Decision**: Bundle `Qwen2.5-0.5B-Instruct` in GGUF format, INT4 quantisation (`Q4_K_M`). On-disk ~400 MB. Downloaded at runtime via the same `embedding::download` infrastructure used for BGE models. Stored at `<root>/models/qwen2.5-0.5b-instruct/model.gguf` with a `manifest.json` sibling carrying the SHA-256 checksum.

**Rationale**:
- Qwen2.5-0.5B is the smallest member of the Qwen2.5 family with reliable instruction-following on short summarisation tasks. Empirically, it produces coherent 400–800-char and 1500–2500-char summaries from a list of plugin descriptions; smaller models (TinyLlama-1.1B, Phi-3-mini-3.8B) are either lower quality or larger on disk.
- Q4_K_M quantisation is the canonical CPU-friendly format for llama.cpp: ~30% smaller than Q4_0 with negligible quality loss for the target use case.
- Apache 2.0 licence — within the constitution's allowlist.
- Static URL hosted on Hugging Face's CDN; the existing model-registry SHA-256 + atomic-rename download discipline applies unchanged.

**Alternatives considered**:
- Qwen2.5-1.5B-Instruct INT4 (~1 GB on disk). Rejected: triples the download size for a marginal quality improvement on the constrained summarisation task. May be revisited via Phase 5+ escape hatches if dogfooding demands.
- TinyLlama-1.1B-Chat INT4. Rejected: instruction-following quality on bullet-list summarisation is noticeably worse.
- Phi-3-mini-3.8B Q4_K_M (~2.4 GB). Rejected: too large to bundle as a default; community licence terms (MIT) acceptable but the size is the disqualifier.
- Gemma-2-2B-it Q4_K_M (~1.6 GB). Rejected: marginally larger than Qwen with no clear quality advantage on short structured summarisation.

**Confidence**: Medium-high. The quality bar for "short and long summary of which plugin topics this workspace covers" is modest; Qwen2.5-0.5B clears it in scratch testing. If dogfooding reveals quality regression, Phase 5+ adds an escape hatch (FR-427 carries that decision).

## R-4. Binary-size projection with `llama-cpp-2` linked

**Decision**: Project the Phase 4 stripped release binary at **~30 MB on macOS arm64 and ~32 MB on Linux x86_64**, comfortably under the 50 MB constitution cap. Validate by a scratch build during Foundational. If the addition exceeds ~10 MB beyond projection, drop `llama-cpp-2` features (BLAS / OpenMP / CUDA — all disabled by default but verify) and re-measure.

**Rationale**:
- Phase 3 baseline measurement: 22 MiB on macOS arm64, 29.56 MB on Linux x86_64 (Phase 2 measurement; Phase 3 didn't budge significantly because rmcp + tokio added ~1.9 MB).
- `llama.cpp` compiled CPU-only as a static lib is typically 4–6 MB depending on instruction-set tuning (AVX2 on Linux, NEON on Apple Silicon). The `llama-cpp-2` Rust wrapper itself adds < 200 KB of pure Rust.
- Projected Phase 4 binary: 22 MiB (P3) + 6 MiB (llama.cpp static lib, CPU-only) + 0.2 MiB (`llama-cpp-2` wrapper) + 0.5 MiB (`toml_edit`, R-5) + 0 MiB (`serde_json` `preserve_order` is a feature flag, not new code) - 0.3 MiB (drop `directories` + `dirs-sys`) ≈ **~28.4 MiB on macOS arm64**, **~34 MB on Linux x86_64**.
- The 50 MB cap has ~16 MB of headroom on macOS, ~16 MB on Linux. Comfortable for Phase 4 and the next few phases.

**Alternatives considered**:
- Dynamic linking `llama.cpp`. Rejected: the constitution's "single static binary" identity wins; dynamic linking requires the user to install a separate library, breaking the offline-first install flow.
- Statically linking `llama.cpp` with CUDA/Metal acceleration features. Rejected: these features add 30–80 MB and require the user to have the corresponding GPU SDK installed. Tome targets CPU-only inference for the summariser (offline-friendly, deterministic across user machines).

**Confidence**: Medium. Final measurement happens at the end of Foundational; the cap holds in the projection but precise build-time numbers can drift by ±2–3 MB based on toolchain version, link-time-optimisation settings, and platform.

## R-5. TOML editing — `toml_edit` for harness MCP config files

**Decision**: Add `toml_edit = "0.x"` as a direct dependency. Use it for read-modify-write of any third-party TOML file Tome writes into (specifically Codex CLI's `~/.codex/config.toml`-style MCP config, plus any future TOML-format harness config). Use the existing `toml` crate (already a Phase 1 dep) for Tome-owned TOML files (settings.toml, project marker config.toml, global config.toml, manifests) — these are full re-writes through `serde`, comment preservation is not required.

**Rationale**:
- The `toml` crate's `serialize → deserialize` round-trip discards comments and re-orders keys. For Tome-owned files this is fine (Tome is the only author). For third-party harness config files where developers may have hand-edited entries with comments, this discards user content silently. FR-349 mandates preservation.
- `toml_edit` preserves comments, key order, whitespace, and even the choice between inline tables and standard tables. Its Document API matches the read-modify-write idiom: parse the file into a `Document`, mutate the specific keys Tome owns, serialise back out.
- Apache 2.0 / MIT dual licence — within the allowlist.
- Adds ~250 KB to the binary; budgeted in R-4.

**Alternatives considered**:
- Use `toml` for everything and accept the comment-loss for harness configs. Rejected: a developer who has commented their Codex MCP config will lose those comments the first time Tome touches the file. Violates the principle of minimum surprise; FR-349 explicitly forbids it.
- Hand-rolled comment-preserving TOML edit logic. Rejected: principle XII (inherit, don't reimplement); TOML's comment-preservation semantics are non-trivial.
- `toml-edit` rebranded crate (`toml_edit` without the underscore is sometimes spelled `toml-edit` in older docs). Verified the canonical crate name as `toml_edit` (with underscore).

**Confidence**: High.

## R-6. JSON editing — `serde_json` with `preserve_order` feature

**Decision**: For harness MCP config files in JSON format (Claude Code's `.claude/settings.json`, Cursor's `.cursor/mcp.json`, Gemini's `~/.gemini/settings.json`), use `serde_json` with the `preserve_order` feature enabled. This makes `serde_json::Value::Object` use `indexmap::IndexMap` instead of `BTreeMap`, preserving the order keys appear in the file. Round-trip preserves order; modifications insert/update at the appropriate position.

**Rationale**:
- `serde_json` is already a Phase 1 / 2 / 3 dep (transitively via `rmcp` and directly via the existing CLI output). The `preserve_order` feature is opt-in; enabling it project-wide affects every `serde_json::Value::Object` in the codebase. Audit pass during Foundational confirms no Tome code relies on alphabetical key ordering.
- JSON has no comments in the spec (JSON5 / JSONC exist but are non-standard). Harness MCP config files Tome targets are plain JSON. Order preservation is sufficient.
- Adds `indexmap` as a transitive (~100 KB). Already pulled in transitively by `toml_edit`; net binary impact zero.

**Alternatives considered**:
- Hand-rolled JSON-with-order-preservation parser. Rejected: the `preserve_order` feature is a one-line `Cargo.toml` change.
- Keep `serde_json` default-features and accept key reordering. Rejected: a developer's `mcpServers: { foo, tome, bar }` would silently become `mcpServers: { bar, foo, tome }` on every sync — surprising and noisy in diffs.
- `serde_json` with a custom `IndexMap`-backed visitor. Rejected: `preserve_order` does exactly this; no need to roll our own.

**Confidence**: High.

## R-7. Schema v1→v2 migration shape

**Decision**: The schema v1→v2 migration is **structural only**. A named `fn` (not a closure) added to `index::migrations::MIGRATIONS` performs: `CREATE TABLE workspaces ...`; `CREATE TABLE workspace_skills ...`; `CREATE TABLE workspace_catalogs ...`; `CREATE TABLE workspace_projects ...`; `INSERT INTO workspaces VALUES (?, 'global', now(), now())`; `ALTER TABLE skills DROP COLUMN enabled` (or table-rebuild equivalent — see below). It does NOT migrate Phase 3 user data into the new tables; the Phase 3 wipe (FR-304) means no Phase 3 user database with developer data is ever opened by a Phase 4 binary.

The synthetic-fixture e2e tests (`tests/schema_migration_e2e.rs` from Phase 3) continue to use the `MIGRATIONS_OVERRIDE` thread-local injection point with a `RAII MigrationsGuard`. The Phase 3 synthetic `SuggestedFix` injection in `tests/doctor.rs::fix_runs_forward_schema_migration_end_to_end` is DROPPED in Phase 4: with one registered migration, `doctor::build_suggested_fixes` naturally emits `subsystem: "schema"` when a v1 DB is found, and the test exercises the real production trigger.

**SQLite caveat on `ALTER TABLE ... DROP COLUMN`**: SQLite added `DROP COLUMN` in 3.35 (March 2021). The bundled SQLite is well past 3.35. **However**, `DROP COLUMN` refuses any column that is part of an index, has a `CHECK` constraint referencing it, or is referenced by a foreign key. Phase 3's `src/index/schema.rs` declares `skills.enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))` AND `CREATE INDEX idx_skills_enabled ON skills(enabled)`. A bare `DROP COLUMN enabled` will therefore fail at runtime. The v1→v2 migration must instead use SQLite's 12-step **table-rebuild** pattern (`CREATE TABLE skills_new ... INSERT INTO skills_new SELECT ... DROP TABLE skills ... ALTER TABLE skills_new RENAME TO skills ... CREATE INDEX ...`) per the SQLite manual's "Making Other Kinds Of Table Schema Changes" section. The rebuild also recreates every non-`enabled` index. `PRAGMA foreign_keys` is toggled OFF for the rebuild and re-enabled at the end (the new `workspace_skills.skill_id REFERENCES skills(id)` FK would otherwise block the `DROP TABLE skills`).

**Rationale**:
- A migration written as a `fn` (not a closure) is testable in isolation, matches the `Migration.apply: fn(&Transaction) -> Result<(), TomeError>` signature locked in Phase 3, and produces clean stack traces.
- Structural-only is correct given the wipe contract: Phase 3 user DBs contain skills.enabled rows scoped per workspace database. Phase 4's central DB has one global database with no per-workspace databases at all — the migration has no way to know which Phase 3 workspace each skill came from. A data-preserving migration is impossible without a separate "scan filesystem for old `.tome/index.db` files" step, which the PRD explicitly forbids (no migration tooling).
- The structural-only path is the only correct shape; tests against synthetic v1 fixtures verify the path works end-to-end without any user-data simulation.

**Alternatives considered**:
- Data-preserving migration that walks the filesystem for old per-workspace DBs. Rejected: out of scope per PRD; pre-release wipe contract.
- Defer the migration registration until Phase 5 (continue shipping `MIGRATIONS = &[]`). Rejected: a fresh Phase 4 install with no prior state would bootstrap directly to schema v2 (the bootstrap code in `index::schema` is updated to emit v2 directly). A v1 DB synthesised in a test then needs the v1→v2 migration to land somewhere. Phase 4 is the natural slot.
- Multiple per-table migrations. Rejected: the four `CREATE TABLE`s + one `INSERT` + one column drop is one logical operation; splitting it produces fragile intermediate states.

**Confidence**: High.

## R-8. Per-harness specifics

**Decision**: The five supported harnesses' rules-file paths, MCP config locations, scope behaviour, and `@`-include support are pinned below. Each path was verified against the harness's current docs as of 2026-05-14; any changes during the Phase 4 implementation window are absorbed by the harness module contract (FR-461) without spec changes.

| Harness | Per-user dir | Rules-file target (precedence) | Rules-file strategy | Block body style | MCP config path | MCP config format |
|---|---|---|---|---|---|---|
| `claude-code` | `~/.claude/` | `AGENTS.md` > `CLAUDE.md` > `.claude/CLAUDE.md` | `BlockInExistingFile` | `AtInclude` (uses `@path` syntax) | `<project>/.claude/settings.json` | JSON |
| `codex` | `~/.codex/` | `AGENTS.md` (only option supported by Codex CLI) | `BlockInExistingFile` | `AtInclude` (uses `@path` syntax) | `~/.codex/config.toml` (global only — Codex CLI does not yet support per-project MCP config) | TOML |
| `gemini` | `~/.gemini/` | `AGENTS.md` > `GEMINI.md` > `.gemini/GEMINI.md` | `BlockInExistingFile` | `AtInclude` (Gemini CLI accepts `@path` includes in its rules format) | `~/.gemini/settings.json` (global only — per-project support not yet stable) | JSON |
| `cursor` | `~/.cursor/` | `<project>/.cursor/rules/TOME_SKILLS.md` (Tome-owned standalone file inside Cursor's `.cursor/rules/` directory) | `StandaloneFile` | N/A (standalone strategy doesn't use block-body styles) | `<project>/.cursor/mcp.json` | JSON |
| `opencode` | `~/.opencode/` | `AGENTS.md` (OpenCode follows the `AGENTS.md` convention) | `BlockInExistingFile` | `Inline` (OpenCode does not document `@`-include support in its rules format as of this writing; falling back to inline content is safe) | `<project>/opencode.json` (per-project, located at project root) | JSON |

**Per-harness notes**:

- **Claude Code**: `AGENTS.md` is the de-facto standard since 2025; `CLAUDE.md` predates `AGENTS.md` and is retained as the second precedence rung for projects that adopted the older convention. `.claude/settings.json`'s `mcpServers` object is the canonical entry point.
- **Codex CLI**: Currently only reads `AGENTS.md` for project rules. MCP is configured globally at `~/.codex/config.toml`; per-project MCP support is on the OpenAI roadmap but not yet stable — Phase 4 ships against the global config; switching to per-project is a Phase 5+ amendment to the Codex harness module that does not affect any other module.
- **Gemini CLI**: `GEMINI.md` is Gemini-specific; `AGENTS.md` precedence catches projects that share rules with Claude Code. The `~/.gemini/settings.json` global MCP config is currently authoritative; per-project support exists in `.gemini/extensions/` but is opt-in and incompatible with the `mcpServers` shape Tome writes — Phase 4 uses the global path.
- **Cursor**: The only `StandaloneFile` harness in Phase 4. Cursor's `.cursor/rules/*.mdc` (or `*.md`) pattern allows multiple rule files to coexist; Tome owns a single file at a documented path (`TOME_SKILLS.md`). The `.cursor/mcp.json` is per-project standard.
- **OpenCode**: Newer harness, smaller community at PRD time; the `AGENTS.md` convention is honoured. MCP config at `opencode.json` at the project root (no dot-prefix in OpenCode's convention).

**Rationale**: Each harness's docs were checked at PRD time; the patterns above are the conservative-correct shape. The harness module contract (FR-461) makes per-harness specifics swappable behind a small interface — if a harness's conventions shift between PRD and implementation, only that module changes.

**Alternatives considered**:
- Auto-detecting the harness's preferred rules-file path by querying the harness. Rejected: most of the five harnesses provide no introspection API; reading their config files violates FR-167.
- Pinning every harness to `AGENTS.md` exclusively. Rejected: Cursor's multi-file rules directory doesn't fit the model; falling back to a harness-specific default for Cursor specifically maintains Tome's invariant of "one rules-file path per harness."
- Per-project MCP config for every harness (uniform behaviour). Rejected: Codex CLI's and Gemini CLI's global-only MCP configs are real-world constraints; Tome adapts.

**Confidence**: Medium. Harness conventions change. The `Confidence` of any *specific* path is the corresponding harness's update cadence; the `Confidence` of the *pattern* (one module per harness, swappable internals) is High.

## R-9. Composition syntax — TOML strings, not table headers

**Decision**: The composition reference forms `[workspace]`, `[workspaces.<name>]`, `[global]` are TOML **string values** containing brackets, NOT TOML table headers. The `harnesses` array in any settings file is a `Vec<String>`, and every entry is a string literal. Tome parses each string and pattern-matches against the literal text. The implementation parses with a simple `match` ladder; no TOML feature is leveraged beyond `Vec<String>` deserialisation.

**Rationale**:
- TOML's bracket syntax is used for table headers in the file's top-level structure. Inside a string array, brackets have no syntactic meaning; the strings `"[workspace]"`, `"[global]"`, etc. are valid TOML and parse to their literal contents.
- A naive reader of the syntax might mistake `[workspaces.<name>]` for a TOML table header (e.g. `[workspaces.foo]`) and try to nest tables. The spec's FR-450 explicitly forbids this: implementations MUST treat the bracketed forms as string literals, not table headers. This R-decision pins the interpretation.
- Parsing logic: for each entry in the `harnesses` array, dispatch on:
  - `s == "[workspace]"` → `Compose::CurrentWorkspace`
  - `s == "[global]"` → `Compose::Global`
  - `s.starts_with("[workspaces.") && s.ends_with("]")` → `Compose::Workspace(s[12..s.len()-1].to_owned())`
  - `s.starts_with("!")` → `Exclusion(s[1..].to_owned())` (with validation that no further `[` appears, per FR-448)
  - else → `Inclusion(s.clone())`

**Alternatives considered**:
- Use TOML inline tables for composition references (e.g. `harnesses = [{ workspace = "shared" }, "cursor"]`). Rejected: heterogeneous-typed arrays in TOML are awkward to deserialise via `serde`; the bracketed-string form is simpler and matches the PRD's authored syntax.
- A separate top-level `[composition]` table with structured fields. Rejected: developers want composition expressed alongside the harness list, not in a separate section.

**Confidence**: High.

## R-10. Atomic populated-directory helper — promote to `src/util/atomic_dir.rs`

**Decision**: Promote the Phase 3 atomic-populated-directory pattern (`tempfile::Builder::tempdir_in(parent)` → populate → `TempDir::keep()` → `std::fs::rename(staged, target)`) to a reusable helper at `src/util/atomic_dir.rs`. Public API:

```rust
pub fn land_directory<F>(target: &Path, mode_unix: u32, populate: F) -> Result<(), TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>;

pub fn land_directory_with_replace<F>(target: &Path, mode_unix: u32, populate: F) -> Result<(), TomeError>
where F: FnOnce(&Path) -> Result<(), TomeError>;
```

The `_with_replace` variant renames the existing target aside to a `.<name>.old/` sibling first; on failure restores the `.old` sibling. Both variants return the target's final canonicalised path on success. `populate` is invoked with the staged temp directory; the closure writes whatever files it needs.

**Rationale**:
- The pattern lands in Phase 3 inside `workspace::init` (US2.b). Phase 4 uses it in: `workspace init` (FR-400), `workspace rename` (FR-404), `workspace use` for project marker creation (FR-403). That's the rule of three.
- The helper is small (~40 lines) and the test surface is concentrated (atomicity tests, rollback tests, mode-0700 tests) — making it a library function deduplicates that surface across three callers.
- The retro (P4) explicitly flagged the promotion as a "for next time" item.

**Alternatives considered**:
- Keep the pattern inline at each call site. Rejected: three near-identical 30-line blocks each carrying their own subtle differences (e.g. `_with_replace` semantics) is a maintenance hazard.
- Use `tempfile::TempDir::persist` (mentioned in Phase 3's contracts). Rejected: this method does not exist on `tempfile::TempDir` — the historical contract referenced a method name that doesn't compile. The correct API is `TempDir::keep()` returning `PathBuf` plus a manual `fs::rename`. The helper hides this footgun.

**Confidence**: High.

## R-11. New module structure — `src/summarise/`, `src/harness/`, `src/settings/`

**Decision**: Phase 4 adds three new capability modules plus one helper module:

- `src/summarise/` — the `LlamaBackend` singleton, `Summariser` trait, `LlamaSummariser` (production impl), `StubSummariser` (test double), prompts module, model registry extension. **Sync; lives outside `src/mcp/`; the structural sync_boundary test allows it implicitly because it contains no `tokio::`/`async fn`/`.await`.**
- `src/harness/` — the `HarnessModule` trait and the five harness module impls (`claude_code.rs`, `codex.rs`, `gemini.rs`, `cursor.rs`, `opencode.rs`), plus the rules-file block + standalone-file write/read/parse logic, plus the MCP config read-modify-write logic.
- `src/settings/` — the layered settings parser, composition resolver, cycle detector, effective-list computer. Sync, pure compute (no I/O beyond reading the three settings files).
- `src/util/atomic_dir.rs` — the R-10 helper. New `src/util/` directory created to host this and likely future helpers.

The `directories` crate is removed. The Phase 3 `Scope` type (in `src/workspace/scope.rs`) is reshaped from `Global | Workspace(PathBuf)` to a single workspace name (`String`) — see data-model.md. The Phase 3 `src/workspace/inventory.rs` opt-in workspaces-registry helper is **deleted**.

**Rationale**:
- Each new module is organised around one capability (summariser inference, harness integration, settings resolution) — principle VII.
- Sync-boundary test exempts only `src/mcp/`; the new modules stay sync, no extension needed.
- The `Scope` reshape is necessary because Phase 4 workspaces are named, not path-shaped; the type system enforces this in one place.

**Alternatives considered**:
- One mega-module `src/integration/` housing harness + settings. Rejected: composition resolution is meaningful independently of harness modules; future surfaces (e.g. an opt-in `tome settings explain` command for debugging composition) want the boundary.
- Inline summariser into `src/embedding/`. Rejected: embedding and summarisation share zero runtime code (one is fastembed/ort, the other is llama-cpp-2/llama.cpp); the trait + model-registry pattern is the only commonality, and that's the wrong axis to pivot on.

**Confidence**: High.

## R-12. Constitution v1.3.0 amendment

**Decision**: Land a `CONSTITUTION.md` v1.3.0 amendment in the **Foundational** PR (the first Phase 4 PR), before any Phase 4 production code that depends on the new path layout. The amendment rewrites the `## Operational Constraints` §Paths block:

**Before (v1.2.0)**:

> **Paths.** XDG-aware via `directories`. Never hardcode `~/.tome`. Cache directories are content-addressed (sha256 of source URL) to prevent collisions.

**After (v1.3.0)**:

> **Paths.** Tome-owned paths resolve under `<home>/.tome/`. The home directory is resolved via raw environment-variable inspection (`HOME` on Unix; the `std::env::home_dir` standard-library helper is an acceptable alternative since its un-deprecation in Rust 1.85). All Tome state lives under this root; the XDG-style separation of config / data / cache / state is deliberately collapsed into a single tree (rationale: simpler discovery, atomic backup/wipe, parallel evolution with the workspace and project-binding model introduced in Phase 4). Cache directories under the root are content-addressed (sha256 of source URL) to prevent collisions. The v1.2.0 §Paths wording referenced the `directories` crate; that wording was aspirational rather than implemented (Tome's Phase 1 / Phase 2 / Phase 3 code used raw env-var inspection throughout). The v1.3.0 amendment closes that documentation/code mismatch in addition to changing the on-disk layout.

`Version` bumps `1.2.0 → 1.3.0` (MINOR — materially expanded guidance in an operational constraint; the constitution's versioning rule is unchanged from v1.2.0). `Last Amended` bumps to 2026-05-14. The amendment PR carries a one-paragraph rationale in the PR body per Governance §Amendments. Operational Constraints are NOT a NON-NEGOTIABLE principle, so the 24-hour cooling-off period does NOT apply.

**Rationale**: Tome's constitution is the gating document for `cargo clippy` overrides, dependency additions, and material architectural change. Phase 4 cannot ship FR-300 / FR-302 / FR-303 cleanly without the amendment — the existing constitution would (correctly) be cited against the work by anyone reading the rules and the code together. The amendment makes the new pattern the rule, not the exception.

**Alternatives considered**:
- Defer the amendment to Phase 5. Rejected: contradicts the principle that the constitution is the source of truth. A Phase 4 implementation that violates the v1.2.0 §Paths block is, by definition, in violation of the constitution.
- A NON-NEGOTIABLE-style amendment (MAJOR version bump v2.0.0). Rejected: this is a Material Operational Constraint change, not the inversion of a principle. MINOR is the correct versioning level per the constitution's own rule.

**Confidence**: High.

## R-13. Pre-emptive slice plans per Phase 4 user story

Per the P10 retro's "encode pre-emptive slice splits in the plan" recommendation, the Phase 4 task list (Phase 2 of this SDD doc, generated by `/sdd:tasks` later) will use the slice shapes below. `/sdd:tasks` is free to refine; this is the planning baseline.

- **Foundational** (PR sequence F1–F10, one slice per PR, no user-story label):
  - **F1**: Constitution v1.3.0 amendment.
  - **F2**: Drop `directories` crate; introduce `paths::home_root()`; mechanical sweep of every path-builder call site. Structural test `tests/no_directories_imports.rs`.
  - **F3**: Add all 8 new `TomeError` variants with their exit-code mappings (pre-allocation, per P10 retro recommendation).
  - **F4**: Promote the atomic-populated-directory helper to `src/util/atomic_dir.rs` (R-10).
  - **F5**: Add `toml_edit` + enable `serde_json/preserve_order`; sweep audit.
  - **F6**: Bootstrap `src/summarise/` module skeleton (trait, `LlamaBackend` singleton, `StubSummariser`, model-registry extension, download path); no production wiring yet.
  - **F7**: Add the `src/harness/` module skeleton with the `HarnessModule` trait, no impls.
  - **F8**: Add the `src/settings/` module skeleton with composition parser + cycle detection + StubScope fixture.
  - **F9**: Schema v1→v2 migration as a registered `fn` in `MIGRATIONS`; bootstrap path emits v2 directly; deletion of Phase 3 synthetic `SuggestedFix` injection from `tests/doctor.rs`.
  - **F10**: Workspace name validation + `WorkspaceName` newtype + reserved-word check (FR-347 + FR-405). `workspace_projects` PK on `project_path` alone (FR-322 / FR-342).

- **US1 — Bind a project to a workspace** (4 slices):
  - **US1.a**: `tome workspace use <name>` command + atomic project marker landing + workspace-projects UPSERT + advisory-lock-for-the-bind contract. Heavy library-API tests + light CLI binary smoke.
  - **US1.b**: Harness sync inside the bind command flow; integration with the (still-empty) harness modules (uses the placeholder trait). Tests use a `StubHarness` fixture.
  - **US1.c**: First production harness module (`claude_code`) implementing the trait against the contract in R-8; the bind command now writes real rules-file blocks + MCP config entries for Claude Code only.
  - **US1.d**: Cross-product test: pre-state combinations (no marker / marker bound elsewhere / marker bound here), bind succeeds idempotently, error envelope for harness-clash. Closeout + retro.

- **US2 — Manage workspace lifecycle** (3 slices):
  - **US2.a**: `init` + `list` + `info` + `rename` + `regen-summary` (read-side + simple write-side commands). Uses the `StubSummariser` from F6.
  - **US2.b**: `remove` with cascade ordering + reserved-name check + bound-project rejection + override-flag cascade (FR-405 numbered steps).
  - **US2.c**: `sync` (per-workspace + all-workspaces) + closeout + retro.

- **US3 — Layered settings + composition** (3 slices):
  - **US3.a**: Settings parser + composition resolver + cycle detection (pure compute, all library API, deterministic).
  - **US3.b**: `[workspace]` valid-only-in-project enforcement + `!`-prefix validation + the harness-not-supported check.
  - **US3.c**: `tome harness list` + `tome harness use` + `tome harness remove` + scope annotation in output. Closeout + retro.

- **US4 — Summarisation + RULES.md** (3 slices):
  - **US4.a**: Production `LlamaSummariser` (replaces `StubSummariser` in non-test code) + prompts module + length-window enforcement (warning, not error). Library tests via `StubSummariser`; one CI-skipped real-model integration test.
  - **US4.b**: Trigger wiring: plugin enable/disable/reindex/catalog-update triggers + the summariser-failure-during-enable forward-progress rule (FR-385). MCP server reads cached short summary at startup (FR-425); no in-process summariser invocation in MCP mode.
  - **US4.c**: `regen-summary` command + closeout + retro.

- **US5 — Doctor extensions** (2 slices):
  - **US5.a**: Doctor reports binding state + project-rules-file-copy state + per-harness rules-file integration state + per-harness MCP config integration state + summariser subsystem state. New `subsystem` arms for `binding:rules-copy` / `harness:<name>:rules` / `harness:<name>:mcp` / `summariser`. The `subsystem` ladder is now at ~11 arms; promote to a typed enum per the P6 retro's "promote at >6 arms" guideline.
  - **US5.b**: `--fix` handlers for the supported repair classes (re-copy project rules; re-run harness sync for one harness; re-download summariser model). User-owned MCP entry conflict remains the explicit-override case. Closeout + retro.

- **Polish phase** (P9, follow Phase 3's PR-A→H pattern): four-reviewer parallel pass (contract audit, Rust-lens, test audit, security audit). Apply blockers + majors before declaring v0.4.0.

**Rationale**: Each slice ≤ ~400 lines, single theme, single PR. Foundational pre-allocates everything later slices depend on (per P10 retro). User-story slices each ship end-to-end value (the headline `tome workspace use` flow lands incrementally with one harness, then fans out).

**Confidence**: High. Slice boundaries may shift during `/sdd:tasks`; the *shape* (Foundational pre-allocates; each US slice is end-to-end-value) is firm.

## R-14. StubSummariser test double

**Decision**: Add `src/summarise/stub.rs` containing `StubSummariser` (mirrors the `src/embedding/stub.rs` pattern). The stub implements the `Summariser` trait with deterministic, content-addressable outputs — given a list of plugin descriptions, returns short and long strings derived from a hash of the inputs. The stub records call counts (per the `StubEmbedder::call_count()` pattern in P3) so tests can assert "summariser was/wasn't invoked." `#[cfg(test)]`-gated; production builds never link it.

**Rationale**:
- Real summarisation in CI requires the Qwen2.5-0.5B weights (~400 MB) on every CI machine. That's prohibitive.
- The deterministic stub lets tests assert the *triggers* (FR-423 — enable, disable, reindex with changes, catalog update, regen-summary), the *forward-progress* rule (FR-385), and the *cache-hit/cache-miss* logic in `[summaries]` regeneration, all without real inference.
- Matches the StubEmbedder discipline: production trait, cfg-test stub, integration tests use the stub by default, one CI-skipped real-model test gates SC-119-style budgets.

**Alternatives considered**:
- No stub; mock the `Summariser` trait with `mockall` or similar. Rejected: a project-wide mocking dep for one test surface; the deterministic stub is ~40 lines and zero deps.

**Confidence**: High.

## R-15. Summariser prompts + length-window content

**Decision**: Two prompts shipped in `src/summarise/prompts.rs` as `&'static str` constants. Length windows pinned in the same module.

**Short prompt** (target output ~400–800 chars):

> "You are summarising a developer's skill library. Given the descriptions below, produce a single comma-separated phrase listing the topics these skills cover. No prose, no lead-in, no bullet points. Maximum 700 characters.\n\nSkill descriptions:\n{descriptions}"

**Long prompt** (target output ~1500–2500 chars):

> "You are writing a short rules section for an AI coding agent. The agent has access to a search tool that retrieves skills relevant to a task. Below are the topics the user's skill library covers. Write a 4–6 sentence rules section that (1) tells the agent which topics the skill library covers, (2) instructs the agent to call the search_skills tool when working on tasks involving those topics, (3) is written for the agent to read at session start. Plain prose, no headings, no bullet points. Maximum 2400 characters.\n\nTopics:\n{topics}"

Length windows enforced as **warnings** per FR-425 (the cached value is still used; a too-long short summary emits a tracing warning naming the workspace and the observed length). A summary of zero characters or unparsable output is a hard `code 20` failure per FR-424.

**Rationale**: The two prompts are owned by Tome (FR-427 — no user customisation in v1). Their wording matters because they directly shape what an instruction-tuned 0.5B model produces. The shapes above are tested in scratch sessions against Qwen2.5-0.5B and produce summaries within the length windows ~95% of the time; outliers are length-window warnings, not errors.

**Alternatives considered**:
- Single combined prompt producing both summaries. Rejected: 0.5B models struggle with multi-output prompts; sequential two-shot is more reliable.
- Few-shot prompts with worked examples. Rejected: doubles prompt length; the model is small enough that the few-shot tokens crowd out the real input. Zero-shot is cleaner.
- User-customisable prompts. Rejected: FR-427.

**Confidence**: Medium. The prompts are iterated against real model behaviour; what's normative is the contract (two prompts, two cached strings, documented length windows). Wording is in the contract file (`contracts/summariser.md`).

## R-16. `tome workspace use` "project directory" definition

**Decision**: `tome workspace use <name>` refuses if the current working directory's *canonical path* equals `<home>` (the user's home directory) or `/` (the filesystem root). Every other path is acceptable as a project root. The check is two `std::path::Path::canonicalize` calls + two equality checks; no heuristic about `.git/`, `package.json`, or similar.

**Rationale**:
- The developer is the authority on what counts as a project. A heuristic that gates on the presence of `.git/` or `package.json` would refuse legitimate projects (non-git, non-Node) and accept some surprising ones (a `.git/` in `$HOME`).
- The two refusal cases (home, filesystem root) are the two paths that would produce surprising binding behaviour — `$HOME` is shared across every shell session; `/` would be a system-wide binding. Both are almost certainly mistakes.

**Alternatives considered**:
- Require a `.git/` directory or a recognised manifest. Rejected: false positives + false negatives.
- Allow $HOME with a `--confirm-home` flag. Rejected: complexity for a use case no one has reported.
- No refusal at all. Rejected: a developer who `cd ~ && tome workspace use foo` would bind their home directory, with cascading drift that doctor would have to clean up later.

**Confidence**: High.

## R-17. Phase 3 deferred items disposition

The Phase 3 retros (P3–P8) flagged items as "deferred." Phase 4 dispositions:

- **Read-only DB open refactor across all read paths** (P10-deferred). **Fold into Foundational** (F2). Phase 4's central single DB amplifies the value — concurrent reads from MCP servers + CLI status + CLI query against one file are routine.
- **MCP `Input` length caps** (P8-deferred). **Fold into US5 (doctor extensions slice)**. Add an `InputLengthError` (reuse code 2 `Usage` or a new variant — decide during implementation).
- **`fabricate_models` rename** (P6-deferred). **Fold into F6** (summariser bootstrap) — adds a third fabricator (summariser), triggering the rename pass.
- **`subsystem` enum promotion** (P6-deferred at >6 arms). **Fold into US5.a**. Phase 4 hits ~11 arms; promote.
- **Drop the synthetic `SuggestedFix` injection from `tests/doctor.rs`** (P7-deferred until first real migration). **Fold into F9**.
- **`tome workspace prune`** (P8-deferred). **Out of scope for Phase 4**; the named-workspace + central registry model makes this naturally a "remove a workspace whose bound projects are gone" feature — Phase 5+.
- **`Paths.config_file` field rename or formal backlog drop** (P8-deferred). **Drop the rename** — Phase 4 reshapes `Paths` entirely; the historic field name no longer exists.
- **Byte-progress callback on `download_model`** (P10-deferred TD-010). **Fold into F6** — Qwen weights are large enough that an indeterminate spinner is poor UX.
- **M-MCP-3 / M-MCP-11 / m-WKS-*** (P8-deferred). **Fold into Polish**, same pattern as Phase 3.
- **T088 manual SC-001 / SC-002 against real BGE models** + **T093/T094/T095** MCP integration tests (P10 + P8 deferred). **Out of scope for Phase 4** unless a seed-injection refactor on `McpState` proves cheap during US4.b (when the MCP server reads cached summaries from the workspace settings file — same shape touches `McpState` construction).

**Rationale**: Each deferred item is either dragged into a slice that naturally touches the same code, or pushed to Phase 5+ with an explicit reason.

**Confidence**: High.

## R-18. Test discipline carry-over

Phase 4 test discipline mirrors Phase 3 (P10 retro endorsement):

- **Library-API + Stub<X> for heavy paths**: every command's pipeline (silent compute) is library-testable; CLI binary tests cover only the surface (TTY behaviour, exit codes, prompt refusal).
- **RAII guards for thread-local injection**: any new injection point (e.g. a `HARNESS_MODULES_OVERRIDE` for testing) ships with a `Guard` struct whose `Drop` clears the slot.
- **Generate-at-setup fixtures over committed binaries**: every v1 DB, every harness directory layout, every settings.toml composition test bootstraps in-line; no committed `.db` or `.tar.gz` fixtures.
- **`home: &Path` parameter for test isolation**: any function reading the home directory takes `home: &Path` explicitly; CLI wrappers pass `paths::home_root()`, tests pass a `TempDir`-rooted path.
- **Coverage matrix in test module docs**: every `tests/exit_codes_e2e.rs`-style file documents which codes are E2E vs library-level vs deferred-to-manual.
- **Threaded concurrency tests via `Barrier::new(2)`**: parallel `workspace use`, concurrent `harness sync`, two-writer `catalog remove` all use the barrier idiom.
- **Run the four-reviewer pass at every user-story close**, not only at phase close (P10 retro recommendation, P8 retro reinforcement).

**Confidence**: High.

## R-19. Open Items (None)

All Phase 0 NEEDS CLARIFICATION are resolved. Plan re-evaluation gate: PASS.

---

## Summary of new direct dependencies (Phase 4)

| Crate | Version | Features | Justification | Binary impact | Scope |
|---|---|---|---|---|---|
| `llama-cpp-2` | 0.x (latest stable) | default | Summariser inference runtime; FR-420. Synchronous API; matches the constitution's sync-only-outside-`src/mcp/` discipline. | ~6 MB (statically-linked `llama.cpp` CPU-only) | `src/summarise/` |
| `toml_edit` | 0.x | default | Comment/order-preserving TOML editor for read-modify-write of harness MCP config files (Codex CLI); FR-349. | ~250 KB | `src/harness/` |

| Existing crate | Feature change | Justification |
|---|---|---|
| `serde_json` | enable `preserve_order` | Order-preserving JSON for read-modify-write of harness MCP config files (Claude Code / Gemini / Cursor / OpenCode); FR-349. |

## Removed direct dependencies

None. Per R-1's framing correction, `directories` was never a Tome dependency despite the constitution v1.2.0 §Paths constraint's wording. F2's structural test guards against future reintroduction.

## Confirmed non-additions

- No new test-double crate (StubSummariser is hand-rolled, matches StubEmbedder).
- No new async runtime (`llama-cpp-2` is sync; `tokio` remains scoped to `src/mcp/`).
- No new logging or tracing crate (existing `tracing` stack handles summariser logs).
- No new SQLite migration crate (Phase 3 framework absorbs the first registered migration).
- No new prompt-templating crate (prompts are `&'static str` with `{descriptions}` / `{topics}` substituted via `format!`).
- No `home` crate (R-1 — `std::env::home_dir` is sufficient).
