---
description: "Phase 1 — Project Foundations and Catalog Management: implementable tasks organised by user story"
---

# Tasks: Phase 1 — Project Foundations and Catalog Management

**Input**: Design documents from `/specs/001-phase-1-foundations/`
**Prerequisites**: plan.md (loaded), spec.md (loaded), research.md (loaded), data-model.md (loaded), contracts/ (5 CLI subcommands + manifest schema)

**Tests**: Tests are explicitly required by the spec — integration tests per command, plus dedicated suites for manifest strictness, path validation, exit codes, credential scrubbing, and atomicity. Every test task below is mandated by the spec's Success Criteria.

**Organization**: Tasks are grouped by user story (US1 catalog management, US2 manifest authoring, US3 contributor onboarding) to enable independent implementation and testing. Setup (Phase 1) and Foundational (Phase 2) precede all stories.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: User story label (US1, US2, US3); omitted for Setup, Foundational, Polish, and [GIT] tasks
- **[GIT]**: Git workflow task (branch, commit, push, PR, CI verification)
- All file paths are relative to the repository root

## Path Conventions

Single binary crate. Source under `src/`, integration tests under `tests/`, fixtures under `tests/fixtures/`, retros under `retro/`, CI under `.github/workflows/`.

## Phase Mapping (plan.md ↔ tasks.md)

`plan.md` and this document use the word "Phase" with different scopes. The mapping below avoids whiplash:

| plan.md phase                              | Lives in tasks.md as                  |
| ------------------------------------------ | ------------------------------------- |
| Phase 0 — Research (already produced `research.md`) | Inputs — not re-executed in tasks    |
| Phase 1 — Design & Contracts (already produced `data-model.md`, `contracts/`) | Inputs — not re-executed in tasks    |
| Phase 2 — Local Development Environment    | tasks.md **Phase 1: Setup**           |
| (n/a — new in tasks.md)                    | tasks.md **Phase 2: Foundational** (cross-cutting code modules) |
| (n/a — new in tasks.md)                    | tasks.md **Phase 3–5** (user stories) |
| (n/a — new in tasks.md)                    | tasks.md **Phase 6: Polish**          |

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Bring the existing scaffold to a known-good state and add the few remaining tooling files that the constitution and quality gates depend on. Most scaffold files already exist (`Cargo.toml`, `rust-toolchain.toml`, `lefthook.yml`, `deny.toml`, `renovate.json`, `rustfmt.toml`, `clippy.toml`, `_typos.toml`, `LICENSE-*`, `README.md`, `CHANGELOG.md`, `.editorconfig`, `.gitignore`, `src/main.rs`).

### Phase Start (Git)

- [ ] T001 [GIT] Verify on `main` branch and working tree is clean (`git branch --show-current` returns `main`; `git status --porcelain` is empty). If not, abort with a clear error before any further work.
- [ ] T002 [GIT] Fetch and merge `origin/main` to ensure local `main` is up to date.
- [ ] T003 [GIT] Create feature branch `001-phase-1-foundations` from `main` (`git checkout -b 001-phase-1-foundations`). If the branch already exists locally, switch to it and confirm it matches `origin/001-phase-1-foundations`.

### Setup Tasks

- [ ] T004 Verify the existing scaffold builds cleanly: `cargo build`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`. Fix any pre-existing breakage before adding new code (use `devs:rust-dev` agent if anything fails).
- [ ] T005 Install `cocogitto` (`cargo install cocogitto` or `brew install cocogitto`) and verify `cog --version` runs — the `commit-msg` hook depends on it; without `cog` on PATH, lefthook either no-ops or hard-fails (constitution §IX is non-decorative). Then run `lefthook install` to wire `pre-commit`, `commit-msg`, and `pre-push` hooks into `.git/hooks/`. Confirm the hooks are present and executable.
- [ ] T006 Create `.github/workflows/ci.yml` implementing the matrix `{macos-latest, ubuntu-latest} × {stable, MSRV}` per plan.md §Constitution Check (Principle X — CI Gates Every Merge) and STACK.md (use `dev-specialisms:init-local-tooling` skill). Steps: checkout, `dtolnay/rust-toolchain` (channel from matrix, `rust-version` from `Cargo.toml` for MSRV), `Swatinem/rust-cache`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build`, `cargo test`.
- [ ] T007 [P] Create `.github/workflows/security.yml` running on every PR and on a weekly cron (use `dev-specialisms:init-local-tooling` skill). Steps: `cargo install cargo-audit cargo-deny` (cached), `cargo audit`, `cargo deny check`.
- [ ] T008 [P] Add `.github/dependabot.yml` or confirm `renovate.json` covers `cargo` and `github-actions` ecosystems per the constitution's dependency-hygiene posture.
- [ ] T009 Add a CI step that asserts the stripped release binary is under 10 MB on Linux runners (SC-010): `cargo build --release && size=$(stat -c%s target/release/tome) && [ "$size" -lt 10485760 ]`. Add it to `ci.yml` only on the `stable + ubuntu-latest` matrix cell.
- [ ] T010 [GIT] Commit: `chore(ci): scaffold CI matrix, security workflow, and 10 MB binary-size gate`.
- [ ] T011 [GIT] Push branch to `origin` (ensures pre-push hooks pass). If hooks fail, fix and re-push without `--no-verify`.
- [ ] T012 [GIT] Open the draft PR to `main` titled `Phase 1: Project Foundations and Catalog Management` with the spec link in the body. Mark as draft until the final phase.

**Checkpoint**: Toolchain green locally and on CI; PR open in draft; remaining phases proceed against this branch.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the cross-cutting modules every user story depends on — the closed error enum, exit-code mapping, XDG paths, output formatter, tracing wiring, CLI skeleton, credential scrubber, signal-aware Git shell-outs, manifest parser, atomic registry store, and a minimal `main.rs` dispatch loop. No catalog subcommand work begins until this phase is complete.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Phase Start

- [ ] T013 [GIT] Verify working tree is clean before starting Phase 2.
- [ ] T014 Create `retro/P2.md` from the retro template (sections: What Worked Well, What Didn't Work, Workarounds & Solutions, Packages & Dependencies, Patterns & Code, For Next Time).
- [ ] T015 [GIT] Commit: `chore(retro): initialise Phase 2 retro`.

### Error model (the closed set that everything else depends on)

- [ ] T016 Implement `src/error.rs` with the `TomeError` enum and `ManifestInvalid` sub-enum per data-model.md §4, plus `impl TomeError { pub fn exit_code(&self) -> i32 }` (use `devs:rust-dev` agent).
- [ ] T017 Add `tests/exit_codes.rs` with an exhaustive `match` over every `TomeError` variant asserting the documented exit code (1/2/3/4/5/6/7/8). The match must be exhaustive — adding a variant must break this test (use `devs:rust-dev` agent).
- [ ] T018 [GIT] Commit: `feat(error): closed TomeError enum with exit-code mapping`.

### Paths

- [ ] T019 [P] Implement `src/paths.rs` (`Paths::resolve`, `Paths::cache_dir_for(url)`) using the `directories` crate and `sha2` per data-model.md §5 (use `devs:rust-dev` agent).
- [ ] T020 [GIT] Commit: `feat(paths): XDG-aware path resolution with sha256 cache addressing`.

### Logging and output

- [ ] T021 [P] Implement `src/logging.rs` wiring `tracing-subscriber` with `EnvFilter` (`TOME_LOG` → `RUST_LOG` fallback), `fmt` layer to `stderr` only, verbosity from `-v`/`-vv`, orthogonal to `--json` per research.md R-5 and FR-019b (use `devs:rust-dev` agent).
- [ ] T022 [GIT] Commit: `feat(logging): stderr-only tracing-subscriber with EnvFilter`.
- [ ] T023 [P] Implement `src/output.rs` providing a small formatter abstraction with `Mode::{Human, Json}`, NO_COLOR / CLICOLOR handling via `anstream` + `anstyle`, and stdin/stdout/stderr TTY detection via `std::io::IsTerminal` per research.md R-1 and R-4 (use `devs:rust-dev` agent). UTF-8-only output regardless of locale (FR-019a).
- [ ] T024 [GIT] Commit: `feat(output): human/json formatter with NO_COLOR and TTY detection`.

### CLI skeleton

- [ ] T025 Implement `src/cli.rs` with `clap` derive definitions for the top-level binary, global flags (`--json`, `-v`/`-vv`, `--force` consistently across every command per FR-021), and the `catalog` subcommand group with placeholder `add`/`remove`/`list`/`update`/`show` variants (use `devs:rust-dev` agent). `--help` and `--version` are provided by clap automatically (FR-021a).
- [ ] T026 [GIT] Commit: `feat(cli): clap derive defs with global flags and catalog subcommand group`.

### Credential scrubbing (lives at the boundary)

- [ ] T027 Implement `src/catalog/git.rs::scrub_credentials(bytes: &[u8]) -> Vec<u8>` applying the four ordered regex substitutions in research.md R-8 (use `devs:rust-dev` agent). Module is `src/catalog/mod.rs` + `src/catalog/git.rs`.
- [ ] T028 Add `tests/scrubbing.rs` with table-driven cases: `https://user:token@host/` → `https://host/`; `git@host:owner/repo` host preserved, login removed; `token=…`, `password=…`, `bearer …` redacted; 40-char hex sequences flanked by word boundaries redacted; SHA1s in `:` or `=` context preserved (use `devs:rust-dev` agent). FR-024, FR-025, SC-006.
- [ ] T029 [GIT] Commit: `feat(catalog): credential scrubber at the process-output boundary`.

### Signal handling and Git shell-outs

- [ ] T030 Extend `src/catalog/git.rs` with `Git` helper wrapping `std::process::Command` for `clone --depth 1`, `fetch`, `reset --hard`, `checkout`. Install a `ctrlc` handler that flips an `AtomicBool`; any in-flight `Child` is killed and the function returns `TomeError::Interrupted` (FR-026a, research.md R-3) (use `devs:rust-dev` agent). Every captured stderr passes through `scrub_credentials` before any further handling.
- [ ] T031 [GIT] Commit: `feat(catalog): signal-aware git shell-outs with boundary scrubbing`.

### Manifest parser (types + strict parse + validation)

- [ ] T032 Implement `src/catalog/manifest.rs` with `CatalogManifest`, `Owner`, `PluginDeclaration` (all `#[serde(deny_unknown_fields)]`) and a public `parse_and_validate(file: &Path, bytes: &[u8]) -> Result<CatalogManifest, ManifestInvalid>` implementing the six validation steps in data-model.md §3 (use `devs:rust-dev` agent). Path-validation algorithm is included but the exhaustive negative corpus is in US2.
- [ ] T033 [GIT] Commit: `feat(manifest): strict catalog manifest parser with path validation algorithm`.

### Config / registry persistence (atomic)

- [ ] T034 Implement `src/config.rs` with `Config { catalogs: BTreeMap<String, CatalogEntry> }` and `CatalogEntry { name, url, ref_, path, last_synced }` per data-model.md §1–2 (use `devs:rust-dev` agent). All structs `#[serde(deny_unknown_fields)]`. Use the `time` crate for `last_synced` (RFC 3339 UTC).
- [ ] T035 Implement `src/catalog/store.rs` with `load() -> Result<Config, TomeError>` and `save(&Config) -> Result<(), TomeError>` using `tempfile::NamedTempFile::persist` for atomic same-directory rename per research.md R-2 and FR-017b (use `devs:rust-dev` agent).
- [ ] T036 Add `tests/atomicity.rs` with interruption-injecting tests: kill a write mid-flight, assert the on-disk file is either the pre-state or the post-state, never partial (SC-012) (use `devs:rust-dev` agent). Cover both registry mutation (T035) and the cache-tempdir → final-dir rename used by `add` (covered later in US1 but the helper lives in `store.rs`).
- [ ] T037 [GIT] Commit: `feat(catalog): atomic registry persistence with interruption-injecting tests`.

### Strictness guard

- [ ] T038 Add the structural-grep guard portion of `tests/manifest_strictness.rs` from research.md R-7 — a unit test that greps `src/catalog/manifest.rs` and `src/config.rs` and asserts every `#[derive(...Deserialize...)]` struct is annotated `#[serde(deny_unknown_fields)]` (use `devs:rust-dev` agent). The exhaustive bad-manifest corpus is in US2.
- [ ] T039 [GIT] Commit: `test(manifest): strict-parse guard test`.

### Main dispatch

- [ ] T040 Wire `src/main.rs` (replacing the existing stub) to: parse CLI via `cli::Cli::parse()`, initialise `logging`, dispatch to `commands::catalog::run(args)`, map any returned `TomeError` to its exit code, and emit errors per `output::Mode` (use `devs:rust-dev` agent). Sub-commands are still stubbed at this point — full implementations land in US1.
- [ ] T041 Create `src/commands/mod.rs` and `src/commands/catalog.rs` with stub variants returning `unimplemented!()` so the binary builds and dispatches (use `devs:rust-dev` agent).
- [ ] T042 [GIT] Commit: `feat(cli): main dispatch loop with TomeError → exit code mapping`.

### Phase 2 close-out

- [ ] T043 Run `/sdd:map incremental` to refresh `.sdd/codebase/` documents for the foundational modules just added.
- [ ] T044 [GIT] Commit: `docs(codebase): update codebase documents after Phase 2`.
- [ ] T045 Review `retro/P2.md` and extract genuinely cross-cutting learnings to `CLAUDE.md` (be conservative — only patterns applicable across the whole project, not Phase 2-local details).
- [ ] T046 [GIT] Commit: `chore(retro): finalise Phase 2 retro`.
- [ ] T047 [GIT] Push branch (`git push`). Pre-push hooks must pass — if they fail, fix and re-push.
- [ ] T048 [GIT] Update PR body with Phase 2 completion summary (`gh pr edit`).
- [ ] T049 [GIT] Verify all CI checks are green (`gh pr checks`). Fix and re-push on any failure; do not proceed to user stories until green.

**Checkpoint**: Foundation ready — every cross-cutting concern (errors, exit codes, paths, logging, output, signal handling, scrubbing, manifest parsing, atomic registry persistence) is in place and tested. User story work can begin.

---

## Phase 3: User Story 1 — Register and inspect a remote catalog (Priority: P1) 🎯 MVP

**Goal**: Deliver the five catalog subcommands (`add`, `remove`, `list`, `update`, `show`) so a developer can register a public Git-hosted catalog, list it, inspect its manifest, refresh it, and remove it. Every command supports `--json` and produces the documented exit codes.

**Independent Test**: From a fresh install, a developer runs `tome catalog add <local-fixture-or-public-repo>`, sees the catalog in `tome catalog list`, inspects `tome catalog show <name>`, refreshes via `tome catalog update`, and removes with `tome catalog remove --force`. Each step produces both human-readable and (with `--json`) machine-readable output, and the documented exit codes hold for each acceptance scenario in spec.md §User Story 1.

### Phase Start

- [ ] T050 [GIT] Verify working tree is clean before starting Phase 3.
- [ ] T051 Create `retro/P3.md` from the retro template.
- [ ] T052 [GIT] Commit: `chore(retro): initialise Phase 3 retro`.

### Test fixtures (shared across US1 commands)

- [ ] T053 [US1] Create `tests/fixtures/sample-catalog/` containing a well-formed `tome-catalog.toml` (matching the schema in `contracts/catalog-manifest.schema.toml`) and two plugin subdirectories (`plugins/midnight-compact-expert/`, `plugins/midnight-dapp-expert/`) per research.md R-9 (use `devs:rust-dev` agent). The fixture is consumed by integration tests via a helper that copies it into a `tempfile::TempDir`, runs `git init && git add -A && git commit -m init`, and returns the `file://` URL.
- [ ] T054 [US1] Add `tests/common/mod.rs` (declared from each integration test) exposing the fixture helper and a builder that invokes the `tome` binary with controlled `HOME` / `XDG_CONFIG_HOME` / `XDG_DATA_HOME` env vars pointing inside a `TempDir` (use `devs:rust-dev` agent).
- [ ] T055 [GIT] Commit: `test(fixtures): sample catalog fixture and integration test harness`.

### `tome catalog add` (the entry point)

- [ ] T056 [US1] Implement `commands::catalog::add` per `contracts/catalog-add.md`: source resolution (`owner/repo` → `https://github.com/owner/repo`, bare path → `file://`, URLs verbatim); cache-path collision check; `git clone --depth 1 [--branch <ref>]` into a tempdir alongside the final cache dir; SHA detection via `^[0-9a-f]{7,40}$`; manifest parse-and-validate; atomic rename of tempdir to cache dir; registry update via `store::save` (use `devs:rust-dev` agent). Display name override (`--name`) takes precedence over manifest `name`. Duplicate display name → `CatalogAlreadyExists` (exit 4).
- [ ] T057 [US1] Add `tests/catalog_add.rs` covering: happy path with local file:// fixture; `--name` override; `--ref` branch tracking; `--ref` SHA pinning; duplicate registration (exit 4); missing manifest (exit 5); git failure with credential-bearing URL (exit 6, scrubbed); SIGINT mid-clone (exit 8 + no orphaned cache) (use `devs:rust-dev` agent).
- [ ] T058 [GIT] Commit: `feat(catalog): tome catalog add with atomic cache and registry writes`.

### `tome catalog list`

- [ ] T059 [US1] Implement `commands::catalog::list` per `contracts/catalog-list.md`: load registry; iterate in `BTreeMap` order; emit fixed-width human table with auto-truncating URL column, or NDJSON in `--json` mode; zero-catalogs message in human mode, empty stdout in JSON mode (use `devs:rust-dev` agent). `LAST SYNCED` is local-tz in human mode, RFC 3339 UTC in JSON.
- [ ] T060 [US1] Add `tests/catalog_list.rs` covering: zero catalogs (both modes); two catalogs in alphabetical order; JSON mode emits one record per line, no enclosing array; exit code 0 in all success cases (use `devs:rust-dev` agent).
- [ ] T061 [GIT] Commit: `feat(catalog): tome catalog list with human and ndjson output`.

### `tome catalog show`

- [ ] T062 [US1] Implement `commands::catalog::show` per `contracts/catalog-show.md`: registry lookup → `CatalogNotFound` if absent (exit 3); read and re-parse the cached `tome-catalog.toml`; emit the manifest in human or JSON form with registration metadata (`registered.url`, `registered.ref`, `registered.last_synced`) (use `devs:rust-dev` agent).
- [ ] T063 [US1] Add `tests/catalog_show.rs` covering: happy path; unregistered name (exit 3); cache file deleted (exit 7); cache manifest corrupted (exit 5); JSON output schema matches contracts (use `devs:rust-dev` agent).
- [ ] T064 [GIT] Commit: `feat(catalog): tome catalog show with registered metadata`.

### `tome catalog update`

- [ ] T065 [US1] Implement `commands::catalog::update` per `contracts/catalog-update.md`: single-catalog and refresh-all (sequential, `BTreeMap` order); SHA-pin no-op with informational message (FR-008, exit 0); `git fetch origin && git reset --hard origin/<ref>` (or `refs/tags/<ref>` for tag pins); re-parse manifest; atomic `last_synced` update; fail-fast on refresh-all (FR-007) (use `devs:rust-dev` agent). Stderr from git passes through `scrub_credentials`.
- [ ] T066 [US1] Add `tests/catalog_update.rs` covering: single happy path with "advanced N commits" counter; SHA-pinned no-op; refresh-all with two catalogs both succeeding; refresh-all with the second failing (first stays refreshed, exit code is the failure's category per FR-007); manifest-broken-after-fetch case (exit 5, `last_synced` not updated); SIGINT mid-fetch (exit 8) (use `devs:rust-dev` agent).
- [ ] T067 [GIT] Commit: `feat(catalog): tome catalog update with single and refresh-all modes`.

### `tome catalog remove`

- [ ] T068 [US1] Implement `commands::catalog::remove` per `contracts/catalog-remove.md`: registry lookup; TTY check on stdin (`std::io::stdin().is_terminal()`); interactive `[y/N]` prompt defaulting to no; non-TTY without `--force` → `Usage` exit 2 with the documented message; atomic registry write removing the entry; best-effort recursive cache removal (failures logged at WARN, not surfaced as exit codes) (use `devs:rust-dev` agent).
- [ ] T069 [US1] Add `tests/catalog_remove.rs` covering: interactive `y` accepts; interactive `n` / empty / `N` declines; non-TTY without `--force` → exit 2 (FR-021 + spec acceptance scenario 5); `--force` non-TTY happy path; unregistered name → exit 3; cache-already-missing succeeds with WARN log; JSON mode confirmation record (use `devs:rust-dev` agent).
- [ ] T070 [GIT] Commit: `feat(catalog): tome catalog remove with confirmation prompt and --force`.

### Phase 3 close-out

- [ ] T071 [US1] Run the full test suite (`cargo test`) and confirm every US1 acceptance scenario in spec.md §User Story 1 is covered by an integration test. Cross-reference SC-001 (3 commands, under 2 minutes) by hand-walking the quickstart path.
- [ ] T072 [US1] Run `/sdd:map incremental` to refresh codebase documents for Phase 3.
- [ ] T073 [GIT] Commit: `docs(codebase): update codebase documents after Phase 3`.
- [ ] T074 [US1] Review `retro/P3.md` and extract critical learnings to `CLAUDE.md` (conservative — only patterns reusable across phases/features).
- [ ] T075 [GIT] Commit: `chore(retro): finalise Phase 3 retro`.
- [ ] T076 [GIT] Push branch and verify pre-push hooks pass.
- [ ] T077 [GIT] Update PR body with Phase 3 completion summary.
- [ ] T078 [GIT] Verify all CI checks pass.

**Checkpoint**: User Story 1 fully functional — every catalog subcommand works against real local fixtures and emits the documented exit codes. The MVP is shippable on its own. Stop and validate before continuing.

---

## Phase 4: User Story 2 — Author a catalog that the tool accepts (Priority: P2)

**Goal**: Make the catalog-manifest contract enforceable. Every invalid manifest variant in the spec (unknown field, missing required field, URL-shaped plugin source, absolute path, parent-traversal, symlink-escape, Windows-drive prefix) produces a precise error naming the offending field, the value, and the manifest file path. Catalog authors get clear feedback before their catalog ever reaches a developer.

**Independent Test**: An author copies `contracts/catalog-manifest.schema.toml`, fills in their values, and registers the catalog — it succeeds. They then introduce each of the documented failure modes one at a time and confirm registration fails with an error that names the field, value, and file path per FR-023 and SC-005. The path validator's algorithm is identical across macOS and Linux (FR-013).

### Phase Start

- [ ] T079 [GIT] Verify working tree is clean before starting Phase 4.
- [ ] T080 Create `retro/P4.md` from the retro template.
- [ ] T081 [GIT] Commit: `chore(retro): initialise Phase 4 retro`.

### Path validator: exhaustive corpus

- [ ] T082 [P] [US2] Expand `src/catalog/manifest.rs::validate_source` if needed to fully implement data-model.md §3 step 6: URL-scheme rejection (any `://` or `git@`), Windows drive prefix rejection (`<letter>:`), absolute-path rejection, `..` component rejection, symlink resolution via `canonicalize()`, ancestry check against the canonicalised catalog root (use `devs:rust-dev` agent). Verify identical behaviour across macOS and Linux.
- [ ] T083 [US2] Add `tests/path_validation.rs` as a table-driven test with one row per rejection case: `https://example/`, `file:///abs`, `git@host:repo`, `/etc/passwd`, `C:\plugins`, `../escape`, `./plugins/../escape` (normalised but still rejected), a symlink pointing outside the catalog root (created at runtime in the test) (use `devs:rust-dev` agent). Each row asserts the precise `ManifestInvalid` variant and that the error message contains the file path, the field name (e.g. `plugins[0].source`), and the offending value (FR-023, SC-005).
- [ ] T084 [GIT] Commit: `test(manifest): exhaustive plugins[].source rejection corpus`.

### Manifest strictness: exhaustive corpus

- [ ] T085 [US2] Expand `tests/manifest_strictness.rs` (already containing the structural grep guard from T038) with a corpus of bad manifests asserting strict rejection of: unknown top-level field; unknown field in `[owner]`; unknown field in `[[plugins]]`; missing `name`; missing `description`; missing `version`; missing `owner`; missing `owner.email`; missing `plugins[].name`; missing `plugins[].source`; duplicate `plugins[].name`; non-semver `version`; non-email `owner.email`; malformed TOML syntax (use `devs:rust-dev` agent). Each case asserts the exit code (5), the precise `ManifestInvalid` variant, and that the error names the field and file (SC-005).
- [ ] T086 [US2] Add an equivalent strictness corpus for `config.toml` parsing in `tests/manifest_strictness.rs` (unknown top-level field, unknown field inside `[catalogs.<name>]`) — confirms FR-016 is enforced (use `devs:rust-dev` agent).
- [ ] T087 [GIT] Commit: `test(manifest): exhaustive strict-parse rejection corpus`.

### User-facing error quality

- [ ] T088 [US2] Audit every `ManifestInvalid` variant's `#[error("...")]` string to confirm it satisfies FR-023 (names what failed, where it failed, and what the user can do next, where possible). Adjust any message that omits the file path, the field name, or the offending value (use `devs:rust-dev` agent).
- [ ] T089 [US2] Add `tests/error_messages.rs` (or extend `exit_codes.rs`) with assertions on the user-facing display of each `TomeError` variant against the FR-023 / SC-003 criteria (use `devs:rust-dev` agent).
- [ ] T090 [GIT] Commit: `feat(error): user-facing error messages name field, value, and file`.

### Phase 4 close-out

- [ ] T091 [US2] Run `cargo test` and confirm every US2 acceptance scenario in spec.md §User Story 2 is covered by an integration test row. Confirm SC-005 (100% rejection of the malformed-input corpus) is satisfied.
- [ ] T092 [US2] Run `/sdd:map incremental` to refresh codebase documents for Phase 4.
- [ ] T093 [GIT] Commit: `docs(codebase): update codebase documents after Phase 4`.
- [ ] T094 [US2] Review `retro/P4.md` and extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T095 [GIT] Commit: `chore(retro): finalise Phase 4 retro`.
- [ ] T096 [GIT] Push branch and verify pre-push hooks pass.
- [ ] T097 [GIT] Update PR body with Phase 4 completion summary.
- [ ] T098 [GIT] Verify all CI checks pass.

**Checkpoint**: User Story 2 fully functional — every documented manifest failure mode produces a precise error. Catalogs can be authored confidently.

---

## Phase 5: User Story 3 — Onboard as a contributor (Priority: P3)

**Goal**: Make the project contributor-ready. A new contributor can clone, run the documented setup, make a change, and open a green PR in under ten minutes. Local hooks reject violations before push; CI verifies on the supported OS × toolchain matrix; security and licence scans run weekly and on every PR.

**Independent Test**: A developer who has never seen the repository before clones it, runs the documented setup (`lefthook install`, then `cargo test`), makes a trivial docs change, commits and pushes; the pre-commit hook formats and lints, the commit-msg hook validates the Conventional Commit message via `cog verify`, and the pre-push hook runs `cargo test`. The PR's CI matrix passes on first attempt (SC-002). `cargo-audit` and `cargo-deny check` run on the security workflow.

### Phase Start

- [ ] T099 [GIT] Verify working tree is clean before starting Phase 5.
- [ ] T100 Create `retro/P5.md` from the retro template.
- [ ] T101 [GIT] Commit: `chore(retro): initialise Phase 5 retro`.

### Local quality gates (verify they fire as documented)

- [ ] T102 [P] [US3] Audit `lefthook.yml`: `pre-commit` runs `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `typos` in parallel; `commit-msg` runs `cog verify --file {1}`; `pre-push` runs `cargo test --workspace`. Fix any divergence from the constitution and STACK.md (use `dev-specialisms:init-local-tooling` skill).
- [ ] T103 [US3] Verify locally that a deliberately bad commit (non-Conventional message, formatting error, clippy warning, typo) is rejected by the appropriate hook with a clear remediation message. Record the verification result in `retro/P5.md`.
- [ ] T104 [GIT] Commit: `chore(lefthook): verify local quality gates fire as documented` (only if hook changes were needed).

### CI matrix (verify and finalise)

- [ ] T105 [P] [US3] Audit `.github/workflows/ci.yml` (created in Phase 1): matrix `{macos-latest, ubuntu-latest} × {stable, MSRV}`; MSRV value sourced from `Cargo.toml`'s `rust-version`; required steps (fmt check, clippy `-D warnings`, build, test, 10 MB binary-size assertion on `stable + ubuntu-latest`); job name + branch protection compatible (use `dev-specialisms:init-local-tooling` skill).
- [ ] T106 [P] [US3] Audit `.github/workflows/security.yml`: runs `cargo audit` and `cargo deny check` on every PR and on a weekly cron; failing the workflow fails the PR; `deny.toml` reflects the licence allowlist from the constitution (MIT, Apache-2.0, MIT-0, BSD-2/3-Clause, ISC, Unicode-DFS-2016, Zlib) and the GPL/AGPL/LGPL denylist (use `dev-specialisms:init-local-tooling` skill).
- [ ] T107 [GIT] Commit: `ci: verify ci.yml and security.yml match constitution and STACK.md` (only if workflow changes were needed).

### Onboarding path verification

- [ ] T108 [US3] Audit `README.md` for: project description, install path (`cargo install --path .`), the five `tome catalog` commands, dual-licence statement, and a link to `CONSTITUTION.md`.
- [ ] T109 [US3] Confirm `LICENSE-MIT` and `LICENSE-APACHE` are both at the repo root and referenced by `Cargo.toml`'s `license = "MIT OR Apache-2.0"` (FR-032).
- [ ] T110 [US3] Verify by hand the 10-minute on-ramp path from `quickstart.md`: clone → `lefthook install` → `cargo test` → make a trivial change → commit (Conventional Commits) → push → PR. Record the timing in `retro/P5.md`. Do not modify on-ramp markdown documents in this task — record any divergence in the retro instead.
- [ ] T111 [GIT] Commit: `docs(readme): align README.md with quickstart and licence requirements`.

### Phase 5 close-out

- [ ] T112 [US3] From a temporary clone in `/tmp`, time the path from `git clone` to "PR opened with green CI". Record the result in `retro/P5.md` and verify SC-002 (under 10 minutes).
- [ ] T113 [US3] Run `/sdd:map incremental` to refresh codebase documents for Phase 5.
- [ ] T114 [GIT] Commit: `docs(codebase): update codebase documents after Phase 5`.
- [ ] T115 [US3] Review `retro/P5.md` and extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T116 [GIT] Commit: `chore(retro): finalise Phase 5 retro`.
- [ ] T117 [GIT] Push branch and verify pre-push hooks pass.
- [ ] T118 [GIT] Update PR body with Phase 5 completion summary.
- [ ] T119 [GIT] Verify all CI checks pass.

**Checkpoint**: All three user stories independently functional. Contributor onboarding is documented, hooked, and verified.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final cross-cutting verification, documentation polish, and the explicit handoff for review.

### Phase Start

- [ ] T120 [GIT] Verify working tree is clean before starting Phase 6.
- [ ] T121 Create `retro/P6.md` from the retro template.
- [ ] T122 [GIT] Commit: `chore(retro): initialise Phase 6 retro`.

### Cross-cutting success criteria

- [ ] T123 [P] Verify SC-006 by running the full integration suite and confirming `tests/scrubbing.rs` plus the credential-bearing test cases in `tests/catalog_add.rs` and `tests/catalog_update.rs` collectively prove no credential material reaches any user-facing surface, log line, or `--json` record.
- [ ] T124 [P] Verify SC-007 by adding an end-to-end test row per command that runs the same scenario in human mode and `--json` mode and asserts identical exit codes and side effects.
- [ ] T125 [P] Verify SC-008 by adding a non-TTY harness (a pseudo-terminal shim or `bash -c` with redirected stdin) that drives every interactive prompt via its flag equivalent.
- [ ] T126 [P] Verify SC-011 by extending `tests/atomicity.rs` (or adding `tests/interruption.rs`) with an interruption-injecting test for every command that invokes `git`, asserting exit code 8 and no orphaned child processes (FR-026a).
- [ ] T127 Add a CHANGELOG entry under `[Unreleased]` summarising Phase 1 user-visible changes (catalog management, manifest schema, exit codes) and project-level changes (dual licence, CI matrix). If `CHANGELOG.md` does not already contain a `[Unreleased]` header, insert one above the most recent versioned section.
- [ ] T128 [GIT] Commit: `test: cross-cutting verification for SC-006, SC-007, SC-008, SC-011`.

### Documentation polish

- [ ] T129 [P] Sweep doc-comments on every public item in `src/error.rs`, `src/cli.rs`, `src/catalog/manifest.rs`, `src/catalog/store.rs`, `src/catalog/git.rs`, `src/config.rs`, `src/paths.rs`, `src/output.rs`, `src/logging.rs`. Comments explain *why*, not *what* (per the constitution).
- [ ] T130 [P] Verify every command's `--help` output reads naturally and references the relevant contract document name where appropriate.
- [ ] T131 [GIT] Commit: `docs: doc-comment sweep and --help text polish`.

### Phase 6 close-out

- [ ] T132 Run `/sdd:map incremental` to refresh codebase documents for Phase 6.
- [ ] T133 [GIT] Commit: `docs(codebase): final codebase document refresh`.
- [ ] T134 Review `retro/P6.md` and extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T135 [GIT] Commit: `chore(retro): finalise Phase 6 retro`.

### Final PR handoff

- [ ] T136 [GIT] Push branch to `origin` (pre-push hooks must pass).
- [ ] T137 [GIT] Mark the PR as Ready for Review (`gh pr ready`). Update the PR body with the final summary listing all six phases.
- [ ] T138 [GIT] Verify all CI checks pass (`gh pr checks`). Fix and re-push on any failure until every check is green.
- [ ] T139 [GIT] Report PR ready status using the exact format from the SDD git workflow:

```
**PR #<n> READY FOR MERGE. AWAITING LGTM**

<pr-url>
```

After outputting this message, **STOP**. Do not merge. Wait for human LGTM.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: T001–T012. No prerequisites. Establishes branch, CI scaffolding, and verifies the existing tooling boots.
- **Phase 2 (Foundational)**: T013–T049. Requires Phase 1. **BLOCKS** every user story phase.
- **Phase 3 (US1)**: T050–T078. Requires Phase 2. Delivers the MVP.
- **Phase 4 (US2)**: T079–T098. Requires Phase 2. Independent of Phase 3 *in principle*, but in practice US2's tests register catalogs through the same path US1 implements, so US2 should follow US1.
- **Phase 5 (US3)**: T099–T119. Requires Phase 1 (CI scaffolding) and benefits from Phase 2+ being in place so the matrix actually has something to test. Can run in parallel with Phase 4 if a second contributor is available.
- **Phase 6 (Polish)**: T120–T139. Requires all user story phases complete.

### Within Each Phase

- Module implementation precedes its tests; the test task references its implementation by file path.
- `[P]` tasks within a single phase can be executed concurrently (different files, no shared state).
- `[GIT]` tasks are serial — each commit captures the work since the previous commit, in execution order.
- A phase's close-out tasks (`/sdd:map incremental`, retro review, push, PR update, CI verification) are strictly serial and end the phase.

### Parallel Opportunities

- **Setup**: T007 and T008 in parallel with T006 (different workflow files).
- **Foundational**: T019 (paths), T021 (logging), T023 (output) are independent and can run in parallel after T016–T017 (error module). T027 (scrubber) is independent of T019/T021/T023.
- **US1**: The five command implementations could theoretically run in parallel, but they share `src/commands/catalog.rs`; in practice keep them serial unless that file is split.
- **US2**: T082 (path validator polish) and T085–T086 (strictness corpus) operate on different files and can run in parallel.
- **US3**: T102 (lefthook audit), T105 (ci.yml audit), T106 (security.yml audit) all operate on different files.
- **Polish**: T123–T126 all add tests in different files and can run in parallel.

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 (Setup) and Phase 2 (Foundational).
2. Complete Phase 3 (US1).
3. **STOP and VALIDATE**: a developer can `tome catalog add <fixture> && tome catalog list && tome catalog show <name> && tome catalog update <name> && tome catalog remove --force <name>` and every step produces the documented output and exit code.
4. The PR at this point delivers a usable MVP — the spec's "register and inspect a remote catalog" user value is fully realised.

### Incremental Delivery

1. Setup + Foundational → toolchain green, no user-visible behaviour yet.
2. US1 → MVP shippable.
3. US2 → catalog authoring becomes safe; every malformed manifest produces a precise error.
4. US3 → contributor onboarding is documented and enforced by CI.
5. Polish → cross-cutting success criteria verified end-to-end.

### Parallel Team Strategy

With two contributors after Phase 2:

- Developer A: Phase 3 (US1) — the five catalog commands + their integration tests.
- Developer B: Phase 4 (US2) — the path validator's negative corpus and manifest strictness corpus, against the same module Developer A is extending. Coordinate via small commits on the shared branch.
- Either developer: Phase 5 (US3) — CI workflow audit and contributor docs, no source-code coupling.

---

## Notes

- `[P]` tasks operate on different files and have no in-phase dependencies; safe to dispatch concurrently.
- `[Story]` labels (US1, US2, US3) map every implementation task to the user story it serves.
- `[GIT]` tasks enforce the workflow: clean branch start, atomic per-task commits, push with hooks, PR update, CI verification, explicit "ready" handoff.
- Every commit must pass pre-commit hooks (`fmt`, `clippy -D warnings`, `typos`, `cog verify`) and every push must pass pre-push hooks (`cargo test`). Never use `--no-verify`.
- The closed-set guarantee on `TomeError` is compiler-enforced via the exhaustive `match` in `tests/exit_codes.rs`. Adding a variant requires updating the spec's FR-022 and the PRD's exit-code table in the same change.
- Credential scrubbing applies at the boundary (where `git`'s stderr is captured). No downstream surface — `tracing`, `anyhow::Error`, `--json` records — can carry unscrubbed material.
- Atomic writes use `tempfile::NamedTempFile::persist` for files and `tempfile::TempDir` + rename for the cache directory. Same filesystem required.
- The integration tests use real `git` against `tempfile::TempDir` fixtures; never mock the filesystem or the Git binary (constitution principle VIII).
