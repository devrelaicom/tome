# Tome

**Tome is a Rust CLI _and_ MCP server** that makes Claude Code's plugin ecosystem work across other agentic coding harnesses — Cursor, Codex CLI, Gemini CLI, OpenCode, and friends.

You register **catalogs** (Git-hosted collections of plugins), enable the **plugins** you want, and Tome builds a local semantic index of their skills and commands. From there you can search that index from the command line, organise work into named **workspaces**, and wire the whole thing into up to five coding harnesses — both as rules-file / MCP-config integration and as a live **MCP server** that exposes search and your plugins' user-invocable prompts over the Model Context Protocol.

## What Tome does

- **Catalogs → plugins → index.** `tome catalog add` registers a Git-hosted catalog; `tome plugin enable` indexes a plugin's skills and commands into a local SQLite + vector store. `tome query` runs semantic (KNN + reranker) search across everything enabled.
- **Named workspaces.** Central storage lives under `<home>/.tome/workspaces/<name>/`; a project binds to a workspace with a tiny `.tome/config.toml` pointer, so different projects can see different sets of plugins.
- **Harness integration across five harnesses.** Tome writes each harness's rules file and MCP-config entry (Claude Code, Codex CLI, Cursor, Gemini CLI, OpenCode), and propagates per-plugin guardrails, hooks, and agent translations where the harness supports them.
- **An MCP server.** `tome mcp` runs a stdio Model Context Protocol server backed by the resolved workspace's index. It exposes `search_skills`, `get_skill`, and `get_skill_info` tools, plus your enabled plugins' user-invocable commands as MCP prompts (and, optionally, agent personas). This server shipped in Phase 3 — it is part of the tool today, not a roadmap item.

## Supported platforms

Linux and macOS, on both `x86_64` and `aarch64`. **Windows is untested** — it may build, but no support is offered and CI does not cover it.

## Privacy / network

**Tome has no telemetry.** It never phones home. The only network access is Git operations against the catalogs you explicitly register and one-time downloads of the pinned inference models (see [Models](#models)). Everything else — the index, embeddings, and summaries — is computed and stored locally under `<home>/.tome/`.

## Install

### From crates.io

The crate is published as `tome-mcp`; the installed binary is named `tome`:

```sh
cargo install tome-mcp
```

### From Homebrew

Prebuilt binaries (produced by the cargo-dist release pipeline for Linux and macOS) are distributed through a Homebrew tap. Replace `<your-tap>` with the tap this project publishes to:

```sh
brew install <your-tap>/tome
```

### From source

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
cargo install --path .
```

### Build prerequisites

Building Tome from source (`cargo install tome-mcp` or `--path .`) needs:

- **Rust ≥ 1.93** (the pinned MSRV; edition 2024) and a system **`git`** on the executable path.
- **A C/C++ toolchain and CMake.** Tome statically links `llama.cpp` (via `llama-cpp-2`) and a vendored `sqlite-vec` extension, both compiled from source by `build.rs`.
- A network connection at build time: the `ort` crate (ONNX Runtime) **downloads the ONNX Runtime shared library during the build**.

The prebuilt Homebrew binaries ship with everything baked in, so they have none of the above build-time requirements.

## Models

Tome downloads three pinned models on demand into `<home>/.tome/models/`. Each download is verified against a pinned SHA-256; a mismatch aborts the install.

| Model | Role | Format | Approx. size | Licence |
|-------|------|--------|--------------|---------|
| `bge-small-en-v1.5` (Xenova INT8) | Embedder | ONNX | ~34 MB | MIT |
| `bge-reranker-base` | Reranker | ONNX | ~279 MB | MIT |
| `qwen2.5-0.5b-instruct` (Q4_K_M) | Workspace summariser | GGUF | ~491 MB | Apache-2.0 |

- The first `tome plugin enable` on a fresh machine prompts to download **all three pinned models — embedder + reranker + summariser, ~804 MB total** (run with `--yes` to skip the prompt). The model-presence check is registry-wide: any model missing from `<home>/.tome/models/` is offered up front, so the summariser is fetched here too, not deferred.
- Once the summariser is on disk it is used to generate workspace summaries. If you decline it at enable time, summary generation (e.g. on enable, or via `tome workspace regen-summary`) is silently skipped until the model is present; everything else — indexing and search — still works with just the embedder + reranker.

`tome models list` shows what is installed; add `--verify` to re-hash each artefact against its pinned SHA-256.

## Getting started

The walkthrough below uses the public **Midnight Expert** catalog as a concrete example; swap the source for any Git-hosted catalog (an `owner/repo` shorthand, a full Git URL, or a `file://` path to a local clone). On a fresh machine the first `tome plugin enable` prompts to download all three pinned models (~804 MB total — see [Models](#models)); that step needs a network connection and a little patience.

```sh
# 1. Register a catalog (owner/repo shorthand, a Git URL, or a file:// path).
tome catalog add devrelaicom/midnight-expert-tome
tome catalog list                              # confirm it registered

# 2. See which plugins the catalog offers.
tome catalog show midnight-expert-tome         # lists every plugin in the catalog

# 3. Enable a plugin — indexes its skills/commands (downloads models on first run).
tome plugin enable midnight-expert-tome/compact-expert
tome plugin list                               # enabled plugins + per-plugin index status

# 4. Search the index semantically.
tome query "how do I write a compact circuit"
tome query "deploy a contract" --top-k 5 --json

# Health + maintenance
tome status                                    # ok / degraded / unhealthy, per subsystem
tome models list                               # installed models; --verify rehashes vs pinned SHA-256
tome reindex                                   # rebuild the index for every enabled plugin
```

Organise work into named workspaces and wire Tome into your harnesses:

```sh
# Named workspaces — central storage; a project binds via a pointer.
tome workspace init my-project              # creates ~/.tome/workspaces/my-project/
tome workspace list                         # all workspaces + their counts
tome workspace use my-project               # bind the current project; writes .tome/config.toml + runs sync
tome --workspace my-project plugin enable midnight-expert-tome/compact-expert

# Harness integration — declarative composition.
tome harness                                # list the supported harnesses
tome harness use claude-code                # add claude-code to the project's settings
tome harness list                           # effective list with the source chain
tome harness sync                           # reconcile rules + MCP config + hooks + guardrails + agents

# Run as an MCP server (launched by your harness over stdio).
tome mcp                                    # search_skills + get_skill + get_skill_info + prompts
                                            # diagnostics → ~/.tome/logs/mcp.log (JSON-lines)
```

## Where things live

Every Tome-owned path lives under **`<home>/.tome/`**:

- `<home>/.tome/config.toml` — Tome's global config (strict TOML, 0600 on Unix)
- `<home>/.tome/settings.toml` — global harness composition settings
- `<home>/.tome/index.db` — central SQLite + `sqlite-vec` (one DB; workspaces and catalog enrolments are junction-table-keyed)
- `<home>/.tome/index.lock` — single advisory lockfile
- `<home>/.tome/catalogs/<sha>/` — shared catalog clones (reference-counted)
- `<home>/.tome/models/<name>/` — embedder, reranker, and summariser weights + `manifest.json` with the pinned SHA-256
- `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md}` — per-workspace state
- `<home>/.tome/logs/mcp.log` (+ `mcp.log.1` rotation) — JSON-lines, 10 MiB cap, 0600 on Unix

**Per project** (bound via `tome workspace use`):

- `<project>/.tome/config.toml` — a pointer marker (`workspace = "<name>"`); the central registry is the source of truth.
- `<project>/.tome/RULES.md` — propagated from the workspace's `RULES.md` on every sync.

## Security

Tome makes **mechanical** safety guarantees but cannot vet catalog **content** — only add catalogs you trust. See [`SECURITY.md`](https://github.com/devrelaicom/tome/blob/main/SECURITY.md) for the full trust model and how to report a vulnerability.

## Documentation

- **Project principles:** [`CONSTITUTION.md`](https://github.com/devrelaicom/tome/blob/main/CONSTITUTION.md)
- **Contributor on-ramp:** [`CONTRIBUTING.md`](https://github.com/devrelaicom/tome/blob/main/CONTRIBUTING.md)
- **Security & trust model:** [`SECURITY.md`](https://github.com/devrelaicom/tome/blob/main/SECURITY.md)

## Licence

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](https://github.com/devrelaicom/tome/blob/main/LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](https://github.com/devrelaicom/tome/blob/main/LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Tome by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
