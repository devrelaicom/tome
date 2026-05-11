# Tome — Phase 1 PRD

## Overview

Tome is a Rust CLI (and, eventually, MCP server) that makes Claude Code's plugin
ecosystem work across other agentic coding harnesses (Cursor, Codex, Gemini CLI,
OpenCode, and friends). The full vision spans semantic search over skill
descriptions, cross-harness plugin installation, hook translation, and more — but
this PRD covers **Phase 1 only**: project foundations and catalog management.

Phase 1 has no MCP server, no plugin installation, no embeddings, no vector
store, no harness detection. It's the unglamorous foundational work that has to
land before any of the interesting bits can be built.

## Goals

1. A Rust project structured for serious contribution — linting, formatting, CI,
   conventional layout, dual licensing, and a contributor guide.
2. A CLI surface for managing **catalogs** (Git repositories that list plugins):
   `tome catalog add | remove | list | update | show`.
3. A minimal catalog manifest format that captures the essentials for v1 and is
   easy to extend later.

## Non-goals (Phase 1)

Explicitly out of scope:

- MCP server (`tome mcp`)
- Plugin installation (`tome install`) — catalogs are managed, but the plugins
  inside them cannot yet be installed into a harness
- Vector store, embedding model, semantic search
- Harness detection (Claude Code, Codex, Cursor, etc.)
- Skill, command, agent, or hook translation
- Release tooling (cross-platform binary builds, package-manager distribution)
- Authentication beyond what `git` already provides
- Claude Code marketplace format compatibility (deliberate — see §Catalog
  manifest format)

All deferred to later phases.

## Project scaffold

### Toolchain

- **Rust edition:** latest stable
- **MSRV:** current stable at project start; pinned in `Cargo.toml` and verified
  in CI
- **`rust-toolchain.toml`:** pin stable channel + `rustfmt` + `clippy` components

### Crate structure

Single binary crate `tome`. No workspace splitting yet — revisit once there's
enough code to justify it. Initial module layout:

```
tome/
├── Cargo.toml
├── rust-toolchain.toml
├── lefthook.yml
├── deny.toml
├── renovate.json
├── LICENSE-MIT
├── LICENSE-APACHE
├── README.md
├── CONTRIBUTING.md
├── CHANGELOG.md
├── CODE_OF_CONDUCT.md
├── .editorconfig
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── security.yml
├── src/
│   ├── main.rs           # entry point
│   ├── cli.rs            # clap derive definitions
│   ├── commands/
│   │   ├── mod.rs
│   │   └── catalog.rs    # `tome catalog` subcommands
│   ├── catalog/
│   │   ├── mod.rs
│   │   ├── manifest.rs   # catalog manifest schema + parsing
│   │   ├── store.rs      # on-disk storage (config + cache)
│   │   └── git.rs        # git clone/pull operations
│   ├── config.rs         # ~/.config/tome/config.toml
│   ├── paths.rs          # XDG-aware path resolution
│   └── error.rs          # crate-level error type
└── tests/
    ├── catalog_add.rs
    ├── catalog_list.rs
    └── ...
```

### Dependencies

- `clap` with `derive` feature — CLI parsing
- `serde` + `serde_derive` — config + manifest serialisation
- `toml` — config + manifest format
- `anyhow` — application-level error handling
- `thiserror` — typed errors for library-shaped modules
- `tracing` + `tracing-subscriber` — structured logging
- `directories` — XDG / platform-aware paths
- `sha2` — URL hashing for cache directory naming

No `tokio` yet — Phase 1 commands are synchronous. Pulled in only when async
genuinely appears (probably with the MCP server in a later phase).

**Git operations:** shell out to `git` via `std::process::Command` rather than
vendoring `git2`/libgit2. Every dev machine has Git already; libgit2 is several
MB of binary bloat for a capability we don't otherwise need.

### Repo hygiene

- `README.md` — what Tome is, install, quick example, link to phase docs
- `LICENSE-MIT` + `LICENSE-APACHE` (dual licence, `MIT OR Apache-2.0` in
  `Cargo.toml`)
- `CONTRIBUTING.md` — local setup, Conventional Commits, PR conventions
- `CHANGELOG.md` — [Keep a Changelog](https://keepachangelog.com/) format
- `CODE_OF_CONDUCT.md` — Contributor Covenant
- `.editorconfig` — line endings, encoding, indent
- `.gitignore` — standard Rust (`target/`, etc.)

## Code quality automation

### Lefthook

Configuration in `lefthook.yml` at the repo root:

```yaml
pre-commit:
  parallel: true
  commands:
    fmt:
      run: cargo fmt --check
    clippy:
      run: cargo clippy --all-targets --all-features -- -D warnings
    typos:
      run: typos

commit-msg:
  commands:
    conventional:
      run: cog verify --file {1}

pre-push:
  commands:
    test:
      run: cargo test --workspace
```

Tools:

- `cargo fmt` (rustfmt) — formatting
- `cargo clippy` with `-D warnings` — lints promoted to errors
- `typos` — common-typo detection; cheap and high-signal
- `cocogitto` (`cog`) — Conventional Commits validation; Rust-native, idiomatic
- `cargo test` on pre-push — catch broken tests before sharing branches

Contributors run `lefthook install` once after cloning. Documented in
`CONTRIBUTING.md`.

### GitHub Actions

`.github/workflows/ci.yml` — runs on every PR and push to `main`:

- Matrix: `{macos-latest, ubuntu-latest}` × `{stable, MSRV}`
- Steps: checkout → `dtolnay/rust-toolchain` → `Swatinem/rust-cache` → fmt check
  → clippy → build → test
- Required for merge

`.github/workflows/security.yml` — runs weekly via cron + on PR:

- `cargo-audit` (RustSec advisory database)
- `cargo-deny check` (license allowlist, advisory check, source allowlist,
  duplicate-version warnings)

`deny.toml` committed up-front with:

- Licences: allow `MIT`, `Apache-2.0`, `MIT-0`, `BSD-3-Clause`, `BSD-2-Clause`,
  `ISC`, `Unicode-DFS-2016`, `Zlib`; deny `GPL-*`, `AGPL-*`, `LGPL-*`
- Advisories: deny vulnerable, warn on unmaintained
- Sources: allow only `crates.io`
- Bans: warn on duplicate transitive deps

### Dependency updates

Renovate (preferred over Dependabot for Rust ecosystems) via `renovate.json`:

- Auto-PR for patch updates
- Weekly schedule for minor and major updates
- Group related crate updates where sensible (e.g. all `clap-*` together)

### Coverage

`cargo-llvm-cov` + Codecov: nice to have, not Phase 1 blocking. Add when there
are enough tests to make a coverage number meaningful.

## CLI conventions

All commands — Phase 1 and beyond — follow consistent conventions for output
and scripting. These are cross-cutting; every new command must honour them.

### Output format

- **Default:** human-readable output on stdout. Tables, formatted text,
  sensible use of colour when stdout is a TTY.
- **`--json`:** structured JSON output on stdout, suitable for piping into
  `jq` or scripts. Available as a global flag on every command that produces
  output.
- **Errors:** always go to stderr. With `--json` set, errors are emitted as
  JSON objects on stderr; without, as plaintext. Exit code (see §Exit codes)
  indicates failure type.
- **Colour:** respect the `NO_COLOR` environment variable and auto-disable when
  stdout is not a TTY.

### Non-interactive execution

Any command that would otherwise prompt for input must support a flag-based
bypass so the CLI is scriptable. For Phase 1:

- Destructive operations (currently just `tome catalog remove`) prompt for
  confirmation by default and are bypassed with `--force`.
- Future commands that prompt the user must follow the same pattern: every
  interactive prompt has a non-interactive flag equivalent.

When stdin is not a TTY (e.g. running in CI), a command that would otherwise
prompt should error with a clear message rather than hang waiting for input.
Auto-confirming silently in that case is unsafe; failing loudly forces the
caller to opt into `--force` explicitly.

## Catalog management

A **catalog** is a Git repository that contains one or more plugins, with a
manifest at its root declaring what plugins ship in it and where to find them.

### CLI surface

```
tome catalog add <git-url|owner/repo|path> [--name <name>] [--ref <branch|tag|sha>]
tome catalog remove <name> [--force]
tome catalog list
tome catalog update [<name>]
tome catalog show <name>
```

All commands accept the global `--json` flag for structured output (see
§CLI conventions).

Behaviour:

- **`add`**: clones the catalog into the cache, parses its manifest, records it
  in config. Accepted URL forms:
  - `owner/repo` → expanded to `https://github.com/owner/repo`
  - `https://…` or `git@…` → used verbatim
  - `file:///path/to/catalog` or a bare local path → used directly (for catalog
    development)
  - `--name` overrides the catalog name (default: from manifest)
  - `--ref` pins a branch, tag, or commit (default: track the repo's default
    branch)
- **`remove`**: prompts for confirmation, then removes the entry from config
  and deletes the cache directory. Use `--force` to skip the prompt (required
  in non-TTY environments).
- **`list`**: prints a table — name, URL, ref, plugin count, last-synced
  timestamp.
- **`update`**: `git fetch` + fast-forward on a named catalog (or all of them);
  re-parses the manifest; updates the synced timestamp. When updating all
  catalogs, the command fails fast: the first failure aborts the run, no
  further catalogs are touched, and the process exits with the failing
  catalog's error code (typically `6` for Git failure or `5` for an invalid
  manifest). Stderr names which catalog failed and why. Partial-failure
  semantics (attempt-all, report-all) are deliberately not supported — they
  make exit codes ambiguous and complicate CI integration.
- **`show`**: prints catalog manifest contents — name, description, owner,
  version, plugin list.

### On-disk layout

Config: `${XDG_CONFIG_HOME:-~/.config}/tome/config.toml`

```toml
[catalogs.midnight-experts]
url = "https://github.com/midnight/midnight-experts"
ref = "main"
path = "~/.local/share/tome/catalogs/a3f9c1b2…"
last_synced = "2026-05-11T14:23:00Z"
```

Cache: `${XDG_DATA_HOME:-~/.local/share}/tome/catalogs/<sha256-of-url>/`

Hashing the URL prevents collisions between catalogs with the same name from
different sources.

`config.toml` is parsed strictly: unknown top-level fields and unknown keys
within `[catalogs.<name>]` tables are rejected with an error that names the
offending field and the file. Same rule as the catalog manifest (§Catalog
manifest format) — strictness applies to all declarative input.

### Catalog manifest format

File: `tome-catalog.toml` at the catalog repo root.

Schema for Phase 1:

```toml
name = "midnight-experts"
description = "Expert plugins for working with the Midnight privacy chain"
version = "0.1.0"

[owner]
name = "Midnight Labs"
email = "plugins@midnight.network"

[[plugins]]
name = "midnight-compact-expert"
source = "./plugins/midnight-compact-expert"

[[plugins]]
name = "midnight-dapp-expert"
source = "./plugins/midnight-dapp-expert"
```

Constraints (Phase 1):

- All fields are required.
- `plugins[].source` MUST be a relative path within the catalog repo. The
  parser rejects any value that is not a normalised relative path: no `..`
  components, no leading `/`, no Windows-style drive prefixes (`C:\`), no URL
  scheme (`https://`, `file://`, etc.). The resulting path, joined to the
  catalog root, must still resolve inside the catalog directory after
  canonicalisation. Errors name the field, the offending value, and the
  manifest file path.
- Other source kinds (URL, Git submodule, registry pointer) are deferred —
  this validation locks the parser shape so Phase 2 doesn't have to retrofit
  it under time pressure.

**Compatibility note.** This format is deliberately *not* compatible with Claude
Code's `.claude-plugin/marketplace.json`. We're starting simple to keep the
Phase 1 surface tiny; adding a compat shim that reads Claude Code marketplace
files is a Phase 2 decision once we know whether it's worth the maintenance
burden.

Parser behaviour:

- Reject unknown top-level fields with an error that names the field and points
  at the expected schema version. Strictness now makes evolution easier later.

### Git plumbing

Shell out to `git`:

- Clone: `git clone --depth 1 <url> <path>` (shallow by default; full history
  is not needed)
- Pin ref: `git checkout <ref>` after clone if `--ref` is provided
- Update: `git fetch` + `git reset --hard origin/<ref>` (fast, idempotent)

Failure modes to handle explicitly:

- Network unavailable
- Repository not found / authentication failed (surface `git`'s stderr prefixed
  with Tome context — after scrubbing, see below)
- Local catalog cache missing — auto-recover by re-cloning on `update`
- Manifest missing or invalid — fail with a clear pointer at the expected schema

**Credential scrubbing (Principle XIII).** Git's stderr can carry credential
material — URLs with embedded auth (`https://user:token@github.com/…`), credential
helper names, partial tokens from HTTPS auth failures. Before surfacing any
upstream stderr to the user (or to tracing spans), `src/catalog/git.rs` must
scrub it. At minimum: strip `https://[^@]+@` URL prefixes and replace with
`https://`. The same scrubbing applies to any `std::process::Command` argument
list captured for logging — never instrument raw command arguments that may
contain a credential-bearing URL.

**Cache ownership.** The catalog cache directory is owned exclusively by Tome.
`tome catalog update` performs `git reset --hard` and does **not** preserve
local modifications inside the cache; this is intentional and is not classed
as a destructive operation under §CLI conventions (no `--force` required).
Catalog development should happen against a checkout outside the cache, added
via `tome catalog add file:///path/to/catalog`.

**SHA-pinned refs.** When a catalog is added with `--ref <sha>` (a full or
abbreviated commit SHA, not a branch or tag), `tome catalog update` does not
attempt `git fetch` + `git reset --hard origin/<sha>` — a SHA is not a valid
remote ref name. Instead, `update` no-ops with an informational message
("catalog `<name>` is pinned to `<sha>`; use `tome catalog add --ref` to
change") and exits `0`. Detection is structural: any `--ref` value matching
`^[0-9a-f]{7,40}$` is treated as a SHA.

### Authentication

Inherit whatever the user's `git` is configured for: SSH keys, HTTPS PATs via
credential helper, Git config aliases, etc. Tome does not store, prompt for, or
manage credentials.

### Exit codes

- `0` — success
- `1` — internal / unexpected error (last-resort fallback only; classified
  failures always use 3–7)
- `2` — usage error (bad CLI args)
- `3` — catalog not found
- `4` — catalog already exists (on `add`)
- `5` — manifest invalid
- `6` — Git operation failed
- `7` — I/O / filesystem error (permission denied, disk full, missing parent
  directory after recovery attempt, etc.)

Code `1` is reserved for genuine programmer-facing surprises — panics caught at
the top level, invariants violated. Any failure that has a name in the table
above must use its named code, not `1`. Phase 2's `tome doctor` relies on this
discipline.

## Success criteria

Phase 1 is done when:

- `cargo install --path .` from a fresh clone produces a working `tome` binary
  on macOS arm64 and Linux x86_64.
- A fixture catalog can be added (`tome catalog add ./fixtures/sample-catalog`),
  listed, shown, updated, and removed, with sensible output at each step.
- A remote catalog can be added via GitHub shorthand
  (`tome catalog add owner/repo`) and behaves the same.
- Invalid manifests produce errors that name the problem and the expected
  schema.
- All lints, tests, and CI checks pass green on both stable and MSRV.
- A new contributor can clone the repo, run `lefthook install`, and submit a
  green PR within 10 minutes of setup (documented in `CONTRIBUTING.md`).
- Binary size < 10 MB stripped on release builds.

## Resolved decisions

| Question | Decision |
|---|---|
| Naming of the marketplace concept | `catalog` |
| Licence | MIT/Apache-2.0 dual |
| Catalog manifest fields (Phase 1) | name, description, owner (name, email), version, plugins (name, source) |
| Plugin source types in v1 | relative path only |
| Release tooling | deferred |
| Conventional commits | enforced via cocogitto pre-commit hook |
| Manifest file format | TOML (revisit if Claude Code marketplace compat becomes a goal) |
| Git access | shell out to `git`, do not vendor libgit2 |
| Async runtime | not yet; sync only until MCP server lands |
| CLI output | human-readable stdout by default, `--json` for structured output |
| Interactive prompts | must have non-interactive flag equivalents (e.g. `--force`); error rather than hang when stdin is not a TTY |
| Exit code `1` | reserved for internal/unexpected errors; named failure classes always use 3–7 (added 2026-05-11 per constitution review) |
| Credential scrubbing | required on all Git stderr surfacing and tracing instrumentation (added 2026-05-11 per constitution review) |
| Cache directory ownership | Tome-owned; `git reset --hard` on update is not a destructive operation under §CLI conventions (added 2026-05-11 per constitution review) |
| `tome catalog update` partial failure | fail fast on first error; no attempt-all mode (added 2026-05-11 per constitution review) |
| SHA-pinned `--ref` on update | no-op with informational message; never pass a SHA as a remote ref to Git (added 2026-05-11 per constitution review) |
| `config.toml` strict parsing | unknown fields rejected, same as catalog manifest (added 2026-05-11 per constitution review) |

## Phase 2 preview

Out of scope here, but worth signposting:

- `tome install <plugin>` — install a plugin from a registered catalog into a
  target harness's native plugin directory
- Harness detection and `tome doctor`
- MCP server skeleton (`tome mcp`) with `search_skills` + `get_skill` tools
- Vector store + embedding model bootstrap (specific choices deferred to the
  Phase 2 PRD, which will include the dependency and binary-size justification
  required by the constitution's Operational Constraints)