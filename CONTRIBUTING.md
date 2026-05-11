# Contributing to Tome

Welcome. Tome is open source and accepts contributions of any size.

## Before you start

Read these once:

- [`CONSTITUTION.md`](./CONSTITUTION.md) — the principles every change must honour. Of particular note: error-code stability (Principle II), strict schemas (IV), KISS/YAGNI (VI), CI-gated merges (X), and never logging secrets (XIII).
- The active spec under [`specs/`](./specs/) — what we're currently building.

## Local setup (10 minutes from clone to green PR)

```sh
# 1. Clone
git clone https://github.com/devrelaicom/tome.git
cd tome

# 2. Install local hooks
lefthook install

# 3. Verify the toolchain
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If any of step 3 fails on a fresh clone, that's a bug — please open an issue.

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
- **Run the local gates** before pushing. The `pre-push` hook runs `cargo test`; the `pre-commit` hook runs fmt/clippy/typos in parallel.
- **CI must be green** on both `macos-latest` and `ubuntu-latest`, on both `stable` and the MSRV (`1.93` at time of writing).
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

If you spot a security issue (credential leak, path traversal, etc.), do not file a public issue. Email <security@example.invalid> instead (placeholder — update when published).

## Code of Conduct

This project follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Be kind, be specific, be in good faith.
