# Tome

A Rust CLI (and eventually an MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses — Cursor, Codex, Gemini CLI, OpenCode, and friends.

> **Status: pre-release, Phase 5 shipped (v0.5.0).** Tome manages **catalogs** (Git-hosted plugin collections), **plugins** (enable them locally, build a semantic skill index, search it), **named workspaces** (central storage under `<home>/.tome/workspaces/<name>/`, projects bind via `.tome/config.toml` pointers), **rules-file + MCP-config integration into five harnesses** (Claude Code, Codex CLI, Cursor, Gemini CLI, OpenCode), and ships an **MCP server** so harnesses can query the index over the Model Context Protocol. Phase 5 added **commands as first-class entries** alongside skills (`commands/<name>.md`), exposed **user-invocable entries as MCP prompts** (host-side slash commands), shipped a hand-rolled **variable substitution layer** (12 built-ins + env passthrough + Claude Code-compatible argument substitution), added a **middle-tier `get_skill_info` MCP discovery tool**, extended `tome doctor` with Phase 5 surfaces (prompts report, orphan data dirs, entry counts by kind, `pending_re_embedding`), and now embeds `when_to_use` frontmatter for semantic search.

## Install

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
cargo install --path .
```

Requires Rust ≥ 1.93 and a system `git` on the executable path. On first plugin enable, Tome downloads two ONNX models (`bge-small-en-v1.5` + `bge-reranker-base`, ~325 MB total, MIT). On first workspace summariser invocation, Tome downloads the `qwen2.5-0.5b-instruct` GGUF (~400 MB, MIT). All models live in `<home>/.tome/models/`.

## Quick example

```sh
# Catalogs (Phase 1)
tome catalog add midnight/midnight-experts
tome catalog list
tome catalog update

# Plugins (Phase 2)
tome plugin enable midnight-experts/compact-expert
tome plugin list
tome plugin show midnight-experts/compact-expert

# Or browse interactively
tome plugin              # catalog → plugin → action

# Semantic search across enabled plugins
tome query "how do I write a compact circuit"
tome query "deploy a contract" --top-k 5 --json

# Health check and maintenance
tome status                              # ok / degraded / unhealthy + per-subsystem detail
tome models list                         # what's installed; --verify rehashes against pinned SHA-256
tome reindex midnight-experts            # rebuild the index for one catalog (or omit scope for all)

# Named workspaces (Phase 4) — central storage; projects bind via pointer
tome workspace init my-project                 # creates ~/.tome/workspaces/my-project/
tome workspace list                            # all workspaces + their counts
tome workspace use my-project                  # bind the CWD project to the workspace
                                               # writes .tome/config.toml marker; runs harness sync
tome workspace regen-summary my-project        # runs the bundled local summariser
tome --workspace my-project plugin enable ...  # explicitly target the workspace

# Harness integration (Phase 4) — declarative composition
tome harness                                   # list shipped harness modules
tome harness use claude-code --scope workspace # add claude-code to the workspace settings
tome harness list                              # effective list with source-chain
tome harness sync                              # re-run integration sweep for bound project

# Diagnostic (Phase 4) — broad doctor with auto-repair
tome doctor                                    # models + index + catalogs + drift + harnesses
tome doctor --fix                              # auto-repairs each Phase 4 subsystem
tome doctor --fix --force                      # also overrides user-owned MCP entries
tome doctor --verify --json                    # re-hash all model artefacts + structured report

# MCP server — Model Context Protocol over stdio
tome mcp                                       # advertises search_skills + get_skill
                                               # diagnostics → ~/.tome/logs/mcp.log (JSON-lines)
```

## Where things live

Every Tome-owned path lives under **`<home>/.tome/`** (constitution v1.3.0 §Paths):

- `<home>/.tome/config.toml` — Tome's global config (strict, 0600 on Unix)
- `<home>/.tome/settings.toml` — global harness composition settings
- `<home>/.tome/index.db` — central SQLite + `sqlite-vec` (single DB; workspaces and catalog enrolments are junction-table-keyed)
- `<home>/.tome/index.lock` — single advisory lockfile
- `<home>/.tome/catalogs/<sha>/` — shared catalog clones (reference-counted via the `workspace_catalogs` table)
- `<home>/.tome/models/<name>/` — embedder, reranker, and summariser GGUF weights + `manifest.json` with pinned SHA-256
- `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md}` — per-workspace state (strict TOML; the local summariser writes `RULES.md` + the `[summaries]` cache)
- `<home>/.tome/logs/mcp.log` (+ `mcp.log.1` rotation) — JSON-lines, 10 MiB cap, 0600 on Unix

**Per project** (bound via `tome workspace use`):

- `<project>/.tome/config.toml` — pointer marker: `workspace = "<name>"` only. No state; the source of truth is the central registry.
- `<project>/.tome/RULES.md` — propagated from the workspace's RULES.md on every sync.

## Documentation

- **What Tome will do in each phase:** [`PRDs/`](./PRDs/)
- **Project principles:** [`CONSTITUTION.md`](./CONSTITUTION.md)
- **Phase 2 specification:** [`specs/002-phase-2-plugins-index/`](./specs/002-phase-2-plugins-index/)
- **Phase 3 specification:** [`specs/003-phase-3-mcp-workspaces/`](./specs/003-phase-3-mcp-workspaces/)
- **Phase 4 specification:** [`specs/004-phase-4-refactor-harnesses/`](./specs/004-phase-4-refactor-harnesses/)
- **Contributor on-ramp:** [`CONTRIBUTING.md`](./CONTRIBUTING.md)

## Licence

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](./LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Tome by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
