# Phase 0 — Research

This document records the small, scoped decisions that came out of plan-time research. Each entry follows the **Decision / Rationale / Alternatives considered** format.

The Phase 1 PRD already resolves the big questions (Rust, sync only, dual licence, shell out to git, TOML, conventional commits via cocogitto, lefthook). This file covers only the residual choices the implementer needs before the first commit of source.

---

## R-1 — TTY / terminal detection

**Decision**: Use the `std::io::IsTerminal` trait, stabilised in Rust 1.70.

**Rationale**: Adding the `is-terminal` crate became unnecessary once `IsTerminal` shipped in `std`. Calling `std::io::stdin().is_terminal()` and `std::io::stderr().is_terminal()` requires no extra dependency and respects every platform Rust supports. This satisfies FR-021 (error on non-TTY when a prompt would be required) with zero binary-size cost.

**Alternatives considered**: The `is-terminal` crate (now redundant) and the older `atty` crate (unmaintained, soundness issues with stale Windows console handles). Both rejected — the std solution is strictly better and supports every supported platform.

---

## R-2 — Atomic file writes for the registry

**Decision**: Use `tempfile::NamedTempFile::persist` to atomically replace `config.toml`. Write to a temp file in the same directory as the target, then rename. Use a same-directory temp to guarantee the rename is on the same filesystem.

**Rationale**: This is the standard POSIX-compatible "write-and-rename" pattern; `rename(2)` on Linux/macOS is atomic for files on the same filesystem. `tempfile` is already a high-quality, widely-used crate; bringing it in adds ~30 KB to the binary and brings safe temp-directory handling, which we also need for the catalog cache (R-7).

**Alternatives considered**: Hand-rolling the temp-file + rename dance with `std::fs::rename` directly — works, but reimplements something `tempfile` does correctly and exposes our test code to file-descriptor leaks if we forget cleanup on the error path. `atomicwrites` crate — narrower, but the win over `tempfile` is small and we want `tempfile` anyway for the cache atomicity work (FR-017a) and integration tests (`tempfile::TempDir`).

---

## R-3 — Signal handling for `git clone` / `git fetch` cancellation

**Decision**: Use the `ctrlc` crate to install a SIGINT handler that flips an `AtomicBool`. The active subprocess is killed via `Child::kill()` when the flag flips, and the spawned `git` process is the same process group (default behaviour) so it dies cleanly. The handler returns control to `main`, which exits with code 8 (interrupted-by-user) per FR-026a and PRD §Exit codes.

**Rationale**: `ctrlc` is small (~10 KB), cross-platform (Linux, macOS, Windows), and does the bare minimum: install a handler, expose a cancellation flag. The plan does not need the full `signal-hook` machinery — there are no other signals to handle in Phase 1 — and we deliberately avoid `tokio-signal` since we are sync-only (constitution principle VI). The temp working directory used during the in-progress refresh is dropped via RAII on the `Drop` of `tempfile::TempDir`, satisfying FR-017a (no partially populated cache).

**Alternatives considered**: `signal-hook` — more capable, larger. Raw `libc::signal` — gnarly, easy to get wrong, foot-gun for portability. `tokio::signal` — requires async, violates VI.

---

## R-4 — Colour output and NO_COLOR support

**Decision**: Use `anstream` + `anstyle` (the modern stack maintained by the clap authors). `anstream` wraps stdout/stderr writers and auto-disables colour when (a) the writer is not a TTY, (b) `NO_COLOR` is set, or (c) `CLICOLOR=0` is set. Use `anstyle` constants for the few styles Tome emits (red errors, dim secondary text, bold names).

**Rationale**: `anstream` integrates cleanly with clap 4's existing colour machinery (clap is already a dependency), shares the same dependency tree, and satisfies NO_COLOR support without manual TTY checks at every print site. Binary cost is negligible because clap already pulls in `anstyle`. Satisfies FR-020.

**Alternatives considered**: `colored` — older, no NO_COLOR support out of the box, pulls `lazy_static`. `owo-colors` — competent but overlaps with what `anstream` already gives us for free.

---

## R-5 — `tracing` configuration: stderr-only, orthogonal to `--json`

**Decision**: Configure `tracing-subscriber` with:
- An `EnvFilter` reading `TOME_LOG` (or `RUST_LOG` as fallback) at startup.
- A `fmt` layer writing to `std::io::stderr` only.
- Verbosity controlled by `-v`/`-vv` flags on the CLI (mapped to `info` and `debug` respectively) **in addition to** the env filter.
- The fmt layer always emits unstructured human-readable lines, regardless of the `--json` flag. Structured-output mode applies to the *command's primary output* on stdout, not to diagnostic logs (FR-019b).

**Rationale**: This is the only configuration that satisfies FR-019b (diagnostic logs always stderr, orthogonal to `--json`) without conditional code paths that risk drifting apart. `EnvFilter` is the standard `tracing-subscriber` filter.

**Alternatives considered**: Routing tracing through `--json` when set — rejected, mixes log records into the structured stream and breaks scripts that consume stdout as data. Hand-rolled logging — rejected, reimplements `tracing` poorly.

---

## R-6 — MSRV pinning strategy

**Decision**: At `cargo init` time, the implementer runs `rustc --version`, takes the version string (e.g. `1.86`), and writes it as `package.rust-version = "1.86"` in `Cargo.toml`. CI's MSRV job uses `rust-toolchain` action with `1.86`. MSRV bumps follow the project's commit-message convention (`chore(deps): bump MSRV to ...`) and require a justification in the commit body — usually "a dependency we want requires it".

**Rationale**: This is the same policy as the rest of the Rust ecosystem (e.g. `tokio`, `serde`). Concrete pin → CI catches regressions → bumps are deliberate.

**Alternatives considered**: Tracking "latest stable" with no pin — silently fragile, CI passes locally but breaks for contributors on older toolchains. Pinning to a very old version for compatibility — pointless for a new project; no downstream cares yet.

---

## R-7 — Compile-time enforcement of `deny_unknown_fields`

**Decision**: There is no clean compile-time mechanism to force every `serde::Deserialize` struct in the project to carry `#[serde(deny_unknown_fields)]`. Use a **test-time guard**: a unit test in `tests/manifest_strictness.rs` greps `src/catalog/manifest.rs` and `src/config.rs` for every `#[derive(.*Deserialize.*)]` and asserts the next struct definition is preceded by a `#[serde(deny_unknown_fields)]` attribute. The test runs in CI and catches regressions.

**Rationale**: Procedural macro alternatives exist (a custom derive that wraps `serde`'s) but the maintenance burden is far higher than a 50-line test. The grep-based test is brittle to file reorganisation but the file paths are stable and any reorganisation is itself a deliberate change that should re-run tests. Satisfies FR-010 enforcement.

**Alternatives considered**: A wrapper derive macro — too much machinery for the safety it adds, more code to audit. Trusting code review — failing on humans is the whole reason to write the test.

---

## R-8 — Credential-bearing-pattern detection

**Decision**: Scrubbing is implemented in `src/catalog/git.rs::scrub_credentials` and applied to **every** byte stream captured from a spawned `git` process before that stream reaches `tracing`, `anyhow::Error`, or any display path. The scrubber applies a small ordered list of regex substitutions:

1. `https?://[^/@\s]+@` → `https://`  (URL-embedded `user:token@`)
2. `git@[^\s:]+:` → `git@<host>:` (SSH URL with login)
3. `(?i)\b(token|password|api[-_]?key|bearer|authorization)\s*[:=]\s*\S+` → `<scrubbed>` (key=value pairs in `git` helper output)
4. Long hex sequences (40+ chars) flanked by word boundaries → `<scrubbed>` (defence in depth against pasted tokens; SHA1s of 40 chars are tolerated by being unflanked by `:` or `=`)

**Rationale**: Closed list, easy to test (table-driven), no dependency on a regex monstrosity. The `regex` crate is the only addition needed; ~150 KB to the binary, well within budget. Each rule has an integration test with a worked example.

**Alternatives considered**: A general "secret scanner" library (truffleHog-style) — order of magnitude larger, false positives on URL paths and SHAs. Manual `str::replace` — works but the URL-embedded-credential case needs anchored matching that's easier with regex.

---

## R-9 — Test fixtures: local-file-URL catalogs

**Decision**: Every integration test that exercises a catalog operation builds a fresh fixture catalog inside a `tempfile::TempDir` and `cd`'s into it (or registers it via `file://`) before invoking the `tome` binary built by `cargo build`. The fixture is constructed by writing `tome-catalog.toml` + `plugins/<name>/` directories, then running `git init -q && git add -A && git commit -m init -q` inside the temp dir using `Command::new("git")`. No mocking of Git.

**Rationale**: Faithful end-to-end coverage of the parts most likely to break (Git invocation, path resolution, manifest parsing). Real `git` is available on every dev and CI machine. `tempfile` cleans up reliably.

**Alternatives considered**: A canned tar archive checked into `tests/fixtures/` — works, but ages badly (tar with a committed `.git/` is fragile across `git` versions). Mocking the Git binary — outright forbidden by constitution VIII ("never mock the things they hide").

---

## R-10 — Decision matrix summary

| Topic | Choice | Cost |
|---|---|---|
| TTY detection | `std::io::IsTerminal` | 0 |
| Atomic registry write | `tempfile::NamedTempFile::persist` | ~30 KB |
| Catalog cache temp | `tempfile::TempDir` (same crate) | (shared) |
| Signal handling | `ctrlc` | ~10 KB |
| Colour | `anstream` + `anstyle` (already via clap) | 0 |
| Logging | `tracing-subscriber` `fmt` layer + `EnvFilter` | (already listed in STACK.md) |
| Regex (scrubber) | `regex` | ~150 KB |
| MSRV | Pinned literally in `Cargo.toml` | 0 |

Total **new** runtime crate dependencies introduced by Phase 0 research, on top of STACK.md: `tempfile`, `ctrlc`, `regex`, `anstream` (already transitively via `clap`). Estimated binary delta: ~200 KB. Comfortably inside the 10 MB cap.

---

## Open questions left for `/sdd:tasks` or implementation

- Concrete MSRV (`x.y`) — resolved at `cargo init` time when `rustc --version` is observed.
- Exact lint set in `clippy.toml` — start with `pedantic` enabled and `allow`-list the few clippy-pedantic lints that fight idiomatic Rust (e.g. `module_name_repetitions`). Refine when noisy.
- Exact `rustfmt.toml` settings — leave default; bikeshedding deferred.
- Exact `_typos.toml` allowlist — start empty; add as false positives appear.

These are deliberately deferred — the answers don't shape the design.
