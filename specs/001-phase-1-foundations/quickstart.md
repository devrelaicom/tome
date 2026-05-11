# Quickstart — Tome Phase 1

This is the contributor on-ramp for Phase 1. It targets the **10-minute new-contributor PR** success criterion (SC-002) and the **scriptable-by-default** scenario from the spec (User Story 1, P1).

There are two audiences:
1. **A new contributor** who wants to clone, set up, and submit a PR.
2. **A developer trying Tome** who wants to register a catalog and inspect it.

Both audiences are covered below.

---

## Prerequisites

| Tool | Why | How to check |
|---|---|---|
| Rust stable (≥ MSRV in `Cargo.toml`) | The compiler | `rustc --version` |
| `git` | Tome shells out to the system Git client | `git --version` |
| **Optional**: `lefthook` | Local quality-gate runner | `lefthook --version` |

`lefthook` is installed automatically by Cargo during the test step below; the explicit prereq exists for contributors who want to run hooks outside of `cargo test`.

---

## For contributors — 10 minutes from clone to green PR

```sh
# 1. Clone
git clone https://github.com/<owner>/tome.git
cd tome

# 2. Install local hooks (one-time)
lefthook install     # or: cargo run --bin install-hooks

# 3. Verify the toolchain
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

# 4. Make a change. The pre-commit hook will run fmt/clippy/typos in parallel;
#    the commit-msg hook will validate Conventional Commits via cocogitto.
git checkout -b fix/your-thing
$EDITOR src/...           # or docs/, etc.
git add -A
git commit -m "fix(catalog): handle empty manifest gracefully"

# 5. The pre-push hook runs `cargo test`. Push when it's green.
git push origin fix/your-thing

# 6. Open the PR. CI runs the matrix
#    {macos-latest, ubuntu-latest} × {stable, MSRV}
#    plus weekly cargo-audit and cargo-deny. All required for merge.
```

**Conventional Commits cheat sheet**:
```
feat(scope):     a new user-visible behaviour
fix(scope):      a user-visible bug fix
docs(scope):     documentation only
chore(scope):    plumbing, tooling, dep bumps
test(scope):     tests only
refactor(scope): no user-visible behaviour change
ci(scope):       CI configuration
```

If your commit message is rejected, the `commit-msg` hook prints a pointer to https://www.conventionalcommits.org and a one-line fix.

---

## For developers using Tome

```sh
# Install from the repo (Phase 1 install path — no binary release yet)
cargo install --path .

# Verify
tome --version
tome --help

# Register a catalog
tome catalog add midnight/midnight-experts
# → Added catalog `midnight-experts` (ref: main, plugins: 2).

# List
tome catalog list

# Inspect
tome catalog show midnight-experts

# Refresh
tome catalog update midnight-experts

# Refresh everything
tome catalog update

# Remove (prompts)
tome catalog remove midnight-experts
# Remove catalog 'midnight-experts' and its local cache at /Users/…? [y/N]

# Scriptable form
tome catalog list --json | jq -r '.name'
tome catalog remove midnight-experts --force --json
```

### Pinning to a tag or commit

```sh
# Tag pin (still tracking — `update` reattaches to the tag's commit)
tome catalog add midnight/midnight-experts --ref v0.3.1

# SHA pin (frozen — `update` is a no-op with an informational message)
tome catalog add midnight/midnight-experts --ref a64f3c1
```

### Running against a local catalog (catalog development)

```sh
# From inside your catalog repo
git init && git add . && git commit -m init
tome catalog add . --name my-local-catalog
tome catalog show my-local-catalog
```

After editing `tome-catalog.toml`, commit the change and run `tome catalog update my-local-catalog` to re-parse.

---

## Where things live

| Thing | Path |
|---|---|
| Registry | `${XDG_CONFIG_HOME:-~/.config}/tome/config.toml` |
| Catalog cache | `${XDG_DATA_HOME:-~/.local/share}/tome/catalogs/<sha256-of-url>/` |
| Log output | stderr, controlled by `-v`/`-vv` or `TOME_LOG` / `RUST_LOG` |
| Colour | auto, suppressible via `NO_COLOR=1` |

---

## Common errors

| Symptom | Likely cause | Fix |
|---|---|---|
| `git failed for ...: fatal: could not read Username` | The catalog is private and your `git` has no credential helper configured. | Configure `git` credentials (SSH key, PAT, etc.) — Tome inherits whatever `git` knows. |
| `manifest invalid: ... 'plugins[].source' ... contains '..'` | Catalog author tried to point at a sibling directory. | Move the plugin inside the catalog repo, or restructure. |
| `manifest invalid: unknown field 'X' in ...` | Author added a field that isn't in the Phase 1 schema. | See [contracts/catalog-manifest.schema.toml](./contracts/catalog-manifest.schema.toml). |
| `'tome catalog remove' requires --force in non-interactive contexts` | You're running in CI / a pipe / a non-TTY shell. | Pass `--force`. |

---

## Setting up the project from scratch (one-time, for the very first contributor)

This is the step that initialises the Cargo crate. It runs once, then never again.

```sh
# From the empty repo root
cargo init --name tome --vcs none
# Edit Cargo.toml:
#   - add `rust-version = "<current stable>"`
#   - add `license = "MIT OR Apache-2.0"`
#   - add `repository`, `description`, `categories`
#   - declare the dependencies from .sdd/codebase/STACK.md
#   - declare dev-dependencies: tempfile (already runtime), pretty_assertions (optional)
# Add rust-toolchain.toml pinning stable + rustfmt + clippy
# Add lefthook.yml, deny.toml, renovate.json
# Add .github/workflows/ci.yml and security.yml
# Add LICENSE-MIT, LICENSE-APACHE, README.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md, CHANGELOG.md
# Add .editorconfig and .gitignore
# Run `cargo build` to verify the toolchain
# Commit with: chore: bootstrap rust crate and tooling
```

This is the work `/sdd:tasks` will sequence as the first batch of tasks.

---

## How the test suite is organised

Once the implementation is in place:

```
tests/
├── catalog_add.rs           # P1 user story acceptance: register a catalog
├── catalog_remove.rs        # P1 user story acceptance: remove a catalog
├── catalog_list.rs          # P1 user story acceptance: list catalogs
├── catalog_update.rs        # P1 user story acceptance: refresh
├── catalog_show.rs          # P1 user story acceptance: show manifest
├── manifest_strictness.rs   # P2 acceptance: every malformed manifest variant
├── path_validation.rs       # P2 acceptance: source-path validator
├── exit_codes.rs            # Every TomeError variant maps to its documented code
├── scrubbing.rs             # Credential scrubber table-driven cases
├── atomicity.rs             # Interrupted writes leave registry recoverable
└── fixtures/sample-catalog/ # Used by all of the above
```

Each integration test builds a fresh fixture catalog in a `tempfile::TempDir` and invokes the `tome` binary built by `cargo build`. No mocks of Git or the filesystem.
