# Contributing to Tome

Welcome. Tome is open source and accepts contributions of any size.

## Before you start

Read these once:

- [`CONSTITUTION.md`](./CONSTITUTION.md) — the principles every change must honour. Of particular note: error-code stability (Principle II), strict schemas (IV), KISS/YAGNI (VI), CI-gated merges (X), and never logging secrets (XIII).
- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — the module map and the load-bearing rules (sync boundary, closed error set, atomic writes, strictness boundary).
- [`CHANGELOG.md`](./CHANGELOG.md) — what has shipped, and what is currently in flight under `[Unreleased]`.

## Local setup (10 minutes from clone to green PR)

### Prerequisites

- **Rust ≥ 1.93** — the pinned MSRV (`rust-version` in `Cargo.toml`). CI verifies both `stable` and the MSRV.
- **System `git`** on the executable path — Tome shells out to it; there is no vendored Git library.
- **A C/C++ toolchain and CMake** — `llama.cpp` (via `llama-cpp-2`) and the vendored `sqlite-vec` extension compile from source in `build.rs`.
- **Network access on the first build** — the `ort` crate downloads the ONNX Runtime library at build time.
- **`cargo install cocogitto`** — the `commit-msg` hook execs `cog verify` and hard-fails without the `cog` binary.
- **`cargo install typos-cli`** — the `pre-commit` and `pre-push` hooks run `typos` and hard-fail without it.

The first cold build takes considerably longer than an incremental build — the C/C++ dependencies above dominate it. Subsequent builds are incremental and fast.

```sh
# 1. Clone
git clone https://github.com/devrelaicom/tome.git
cd tome

# 2. Wire up the versioned git hooks (one-time, per clone)
git config core.hooksPath .githooks

# 3. Verify the toolchain
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
CARGO_INCREMENTAL=0 cargo test --no-fail-fast
```

If any of step 3 fails on a fresh clone, that's a bug — please open an issue.

### Running tests

```sh
CARGO_INCREMENTAL=0 cargo test --no-fail-fast    # the full suite
cargo test --test workspace                      # one grouped binary (tests/workspace.rs + tests/workspace/)
```

- A bare `cargo test` is fail-fast per test **binary**: the first failing binary stops the run and masks every later binary's failures. `--no-fail-fast` collects them all in one pass.
- `CARGO_INCREMENTAL=0` avoids a deadlock between clippy and the test build on the shared incremental-compilation directory.
- If you kill a run mid-flight, clean up leftover test processes (`pkill -f 'target/debug/deps/'`) before the next run — stale processes can hold the index advisory lock and deadlock the next run's lock tests.
- Integration tests that touch process-global state (`$HOME`, env vars, the harness-module override slot) must hold the mutexes documented in `tests/common/mod.rs` (`HOME_MUTEX`, `HARNESS_OVERRIDE_MUTEX`, …).

#### Real-model release gates

Three `#[ignore]`d tests download the real models and exercise the production inference path end to end; the fast suite runs a stub embedder and never touches them. Run them before proposing changes to the model or embedding pipeline:

```sh
cargo test model_download_complete -- --ignored
cargo test reranker_cpu_inference -- --ignored
cargo test search_knn_recall_realmodel -- --ignored
```

They download real model weights, so expect network use and some patience.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org). The `commit-msg` hook runs `cog verify` and rejects non-conforming messages.

```
feat(catalog): support --ref pinning on `tome catalog add`
fix(git): scrub URL-embedded credentials from stderr
docs(readme): document the file:// catalog form
chore(deps): bump clap to 4.6
```

The conventional-commit format powers our changelog. Squash-and-merge PRs must still produce a conventional commit message.

## Pull requests

- **Small batches.** Keep PRs under ~400 lines and ideally under two modules. If your change is bigger, split it.
- **Run the local gates** before pushing. The `pre-commit` and `pre-push` hooks both run the same fast chain — `cargo fmt --check`, `typos`, `cargo clippy --all-targets --all-features -- -D warnings`. Neither hook runs the test suite: duplicating it locally costs 30+ minutes for no signal CI doesn't already produce, so CI is the test gate.
- **CI must be green** on both `macos-latest` and `ubuntu-latest`, on both `stable` and the MSRV (`1.93` at time of writing). CI removes `rust-toolchain.toml` at the start of every job (the rustup proxy otherwise races the first `cargo` call); local builds use the pinned file, while CI resolves its own `stable` and MSRV toolchains.
- **Documentation lands in the same PR** as the behaviour change — README, CONTRIBUTING, command help text, and the changelog.

## Coding conventions

- **Comments explain why, not what.** The reader knows Rust.
- **Modules are capability-shaped**, not layer-shaped: `catalog`, `config`, `paths`, `error`, `output`, `logging`.
- **Errors**: `thiserror` inside modules; `anyhow` at the application boundary. The `TomeError` enum in `src/error.rs` is the source of truth for exit codes — adding a category there is a constitutional change (see Principle II).
- **TOML deserialisation** uses `#[serde(deny_unknown_fields)]` on every struct. Adding a struct without it is a regression caught by `tests/manifest_strictness.rs`.
- **Process boundary scrubbing.** Any `std::process::Command` output that may carry credentials passes through `catalog::git::scrub_credentials` before it reaches `tracing`, `anyhow::Error`, or any display path.

## Tests

- Integration tests live in `tests/`. Each builds a fresh fixture catalog in `tempfile::TempDir`, invokes the `tome` binary, and asserts behaviour.
- Don't mock `git` or the filesystem — use the real things. (Constitution principle VIII.)
- Property-style coverage for parsers and validators is welcome.

## Security

If you spot a security issue (credential leak, path traversal, etc.), do not file a public issue. See [`SECURITY.md`](./SECURITY.md) for how to report it privately.

## Code of Conduct

This project follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Be kind, be specific, be in good faith.
