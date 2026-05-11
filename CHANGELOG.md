# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

User-visible
- `tome catalog add <source> [--name] [--ref] [--json]` — register a remote
  catalog. `<source>` accepts `owner/repo`, full Git URLs, or local paths
  (auto-converted to `file://`). SHA-shaped `--ref` values are pinned.
- `tome catalog list [--json]` — alphabetical table (human) or NDJSON
  records (JSON).
- `tome catalog show <name> [--json]` — manifest + registration metadata.
- `tome catalog update [<name>] [--json]` — refresh one or every catalog;
  SHA-pinned catalogs are a documented no-op.
- `tome catalog remove <name> [--force] [--json]` — confirmation prompt
  on TTY; `--force` required when stdin is not a TTY.
- Global `--json` and `-v`/`-vv` flags on every command; `--help` and
  `--version` provided automatically by clap.
- Closed-and-exhaustive exit codes: 0 success, 1 internal, 2 usage, 3
  catalog not found, 4 catalog already exists, 5 manifest invalid, 6 git
  failed, 7 I/O, 8 interrupted.

Project-level
- Initial project scaffold: Cargo crate, dual MIT/Apache licence,
  lefthook (`fmt`, `clippy -D warnings`, `typos`, `cog verify`,
  `cargo test`), GitHub Actions CI matrix
  (`{ubuntu,macos} × {stable,MSRV}`), security workflow (`cargo audit`,
  `cargo deny`), 10 MB stripped-binary CI gate, `deny.toml` with the
  constitution's licence allowlist, `renovate.json`.
- Strict TOML parsing (`#[serde(deny_unknown_fields)]`) on every
  manifest and config struct. A structural-grep test rejects regressions.
- Credential scrubbing at the process-output boundary: every byte stream
  captured from a spawned `git` process passes through
  `catalog::git::scrub_credentials` before it reaches `tracing`,
  `anyhow::Error`, or any display path.
- Atomic registry persistence via `tempfile::NamedTempFile::persist`.
- Signal-aware `git` shell-outs: SIGINT during `clone` / `fetch` /
  `reset` kills the child and returns exit code 8.
- XDG-aware path resolution (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`)
  honoured on macOS and Linux.
- Phase 1 specification under `specs/001-phase-1-foundations/`.
- Project constitution (`CONSTITUTION.md` v1.0.1).
