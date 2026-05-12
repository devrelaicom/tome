# Phase 2 — Quickstart

End-to-end walkthrough for the Phase 2 surface. Assumes Tome Phase 1 is installed (`cargo install --path .` from the repo root) and at least one catalog is registered.

---

## Development setup

### Prerequisites

- Rust stable, MSRV ≥ 1.93 (verified in CI).
- `git` on `PATH` (Phase 1 requirement, unchanged).
- macOS arm64 or Linux x86_64.

### Install dependencies and run tests

```sh
cargo build                                        # debug build
cargo test                                         # full suite (uses stub embedder; fast)
cargo build --release                              # release build (used by CI binary-size check)
```

### Quality gates

Run before pushing — lefthook will run these automatically:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
typos
cargo test
```

### Binary size

```sh
cargo build --release --locked
strip target/release/tome   # only if Cargo.toml profile lacks `strip = "symbols"`
du -h target/release/tome   # MUST be ≤ 10 MB; CI fails the build otherwise
```

### Conventional commits

```sh
cog verify --file <commit-msg-file>   # invoked by lefthook commit-msg hook
```

---

## End-user walkthrough

### 1. Download the embedding and reranking models

```sh
tome models download
```

First run: ~325 MB total. With a 50 Mbit/s connection, expect ~60 s. Models land in `${XDG_DATA_HOME}/tome/models/`. Both are MIT-licensed BGE artefacts.

To re-download a corrupt model:

```sh
tome models download --force
```

To inspect installed models:

```sh
tome models list                # cheap (size/existence)
tome models list --verify       # full SHA-256 check
```

### 2. Register a catalog (Phase 1; reproduced for context)

```sh
tome catalog add midnight-experts https://github.com/example-org/midnight-experts
```

### 3. Browse plugins interactively and enable one

```sh
tome plugin
```

You'll get a catalog selector, then a plugin list, then a plugin view with metadata and a component breakdown. Enable from the action prompt — a progress bar shows embedding progress.

Or non-interactively:

```sh
tome plugin enable midnight-experts/compact-expert
```

### 4. Query the index

```sh
tome query "how do I write a compact circuit"
```

Default top-10. To bias toward one catalog or plugin:

```sh
tome query "merkle tree usage" --plugin midnight-experts/compact-expert
```

JSON output for scripting:

```sh
tome query "merkle tree usage" --json | jq '.results[0]'
```

To compare reranked vs raw embedding similarity:

```sh
tome query "merkle tree usage" --no-rerank
```

### 5. Refresh upstream changes and re-index

```sh
tome catalog update              # refreshes every registered catalog
tome catalog update midnight-experts
```

The refresh detects changed skills and re-embeds only those.

### 6. Force a full reindex

After a Tome upgrade that bumps the embedder version:

```sh
tome reindex --force
```

Scoped to a single plugin:

```sh
tome reindex midnight-experts/compact-expert --force
```

### 7. Check overall health

```sh
tome status                      # quick health report
tome status --verify             # full SHA-256 check of models, integrity check of DB
tome status --json | jq         # structured for scripts
```

`tome status` exits non-zero if any subsystem is unhealthy or degraded — useful as a CI pre-flight.

### 8. Disable / re-enable a plugin

```sh
tome plugin disable midnight-experts/compact-expert     # prompts for confirmation
tome plugin disable midnight-experts/compact-expert --force
tome plugin enable midnight-experts/compact-expert      # cheap re-enable; embeddings cached
```

### 9. Remove a catalog whose plugins are enabled

```sh
tome catalog remove midnight-experts
# error: catalog 'midnight-experts' has 2 enabled plugins…

tome catalog remove midnight-experts --force            # cascades: disables + drops + removes
```

### 10. Inspect Tome version and the models that produced your vectors

```sh
tome --version
# tome 0.2.0
# embedder: bge-small-en-v1.5 1.5
# reranker: bge-reranker-base base

tome --version --json
```

---

## Common error → recovery table

| Error message starts with | Exit | What to run |
|---|---|---|
| `Embedding model … is missing` | 30 | `tome models download` |
| `Embedding model … is corrupt` | 31 | `tome models download --force` |
| `Stored vectors were produced by a different embedder` | 41 / 42 | `tome reindex --force` |
| `Schema on disk is newer than this Tome…` | 52 | Upgrade Tome (`cargo install --path .`) |
| `Another tome process is updating the index` | 50 | Wait for the other process to finish, then retry |
| `Plugin … not found` | 20 | `tome plugin list` to see what's available |
| `Plugin … is already enabled` | 21 | No-op; check `tome plugin show` |
| `catalog … has N enabled plugins` | 53 | `tome plugin disable …` first, or `--force` |
| `requires a terminal` | 54 | Use the non-interactive subcommand equivalent |

`tome status` will tell you what to run when in doubt.

---

## Scripting recipes

### Enable every plugin from one catalog

```sh
tome plugin list --catalog midnight-experts --json \
  | jq -r '.[] | select(.status != "enabled") | "\(.id.catalog)/\(.id.plugin)"' \
  | xargs -I {} tome plugin enable {}
```

### Find the most-relevant skill for a build error

```sh
tome query "$(cat error.log | head -c 500)" --top-k 3 --json \
  | jq -r '.results[0].path'
```

### Force a full reindex of every catalog

```sh
tome reindex --force --json | jq '.skills_re_embedded'
```

---

## Where things live on disk

| What | Path |
|---|---|
| Catalogs (Phase 1) | `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` |
| Index database | `${XDG_DATA_HOME}/tome/index.db` |
| Index write lock | `${XDG_DATA_HOME}/tome/index.lock` |
| Models | `${XDG_DATA_HOME}/tome/models/<name>/` |
| Configuration | `${XDG_CONFIG_HOME}/tome/config.toml` |

`${XDG_DATA_HOME}` defaults to `~/.local/share` on Linux and `~/Library/Application Support` on macOS.
