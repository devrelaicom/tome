# Tome

A Rust CLI (and eventually an MCP server) that makes Claude Code's plugin ecosystem work across other agentic coding harnesses — Cursor, Codex, Gemini CLI, OpenCode, and friends.

> **Status: pre-release, Phase 4 in flight (post-v0.3.0).** Tome manages **catalogs** (Git-hosted plugin collections), **plugins** (enable them locally, build a semantic skill index, search it), **workspaces** (per-project state in `.tome/`), and ships an **MCP server** so harnesses can query the index over the Model Context Protocol. Phase 4 is the central-architecture refactor — collapsing per-workspace state into a single `<home>/.tome/` root, introducing named workspaces with project-binding pointers, and wiring rules-file + MCP-config integration into five harnesses (Claude Code, Codex CLI, Gemini CLI, Cursor, OpenCode). Constitution **v1.3.0** lands as part of Phase 4: the v1.2.0 §Paths `directories`-crate citation is replaced by the consolidated `<home>/.tome/` layout.

## Install

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
cargo install --path .
```

Requires Rust ≥ 1.93 and a system `git` on the executable path. On first plugin enable, Tome downloads two ONNX models (`bge-small-en-v1.5` + `bge-reranker-base`, ~325 MB total, MIT) into `${XDG_DATA_HOME}/tome/models/`.

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

# Workspaces (Phase 3) — per-project state in `.tome/`
cd ~/projects/my-app
tome workspace init                      # atomically lands ./.tome/
tome workspace info                      # scope + catalogs + plugins + index
tome catalog add ...                     # writes to the workspace, not global
tome --workspace global plugin list      # explicitly target the global workspace

# Diagnostic (Phase 3) — broad doctor with auto-repair
tome doctor                              # models + index + catalogs + drift + harnesses
tome doctor --fix                        # auto-repairs: model re-download, catalog re-clone, schema migrate
tome doctor --verify --json              # re-hash all model artefacts + emit structured report

# MCP server (Phase 3) — Model Context Protocol over stdio
tome mcp                                 # advertises search_skills + get_skill; stdout = MCP protocol
                                         # diagnostics → ${XDG_STATE_HOME}/tome/mcp.log (JSON-lines)
```

## Where things live

**Global scope:**

- **Config** — `${XDG_CONFIG_HOME}/tome/config.toml` (catalog registry, 0600 on Unix)
- **Catalogs** — `${XDG_DATA_HOME}/tome/catalogs/<sha>/` (one git clone per registered catalog; shared across scopes via reference counting)
- **Models** — `${XDG_DATA_HOME}/tome/models/<name>/` (ONNX weights + `manifest.json` with pinned SHA-256)
- **Skill index** — `${XDG_DATA_HOME}/tome/index.db` (SQLite + `sqlite-vec`)
- **Advisory lock** — `${XDG_DATA_HOME}/tome/index.lock` (held during writes; readers do not block)

**Per workspace** (Phase 3):

- **Marker + config + index** — `<workspace>/.tome/{config.toml, index.db, index.lock}` (0700 on Unix)
- **Workspace registry (opt-in)** — `${XDG_STATE_HOME}/tome/workspaces.txt`; touch the file once to start tracking
- **MCP log** — `${XDG_STATE_HOME}/tome/mcp.log` (JSON-lines, 10 MiB rotation cap, 0600 on Unix)

## Documentation

- **What Tome will do in each phase:** [`PRDs/`](./PRDs/)
- **Project principles:** [`CONSTITUTION.md`](./CONSTITUTION.md)
- **Phase 2 specification:** [`specs/002-phase-2-plugins-index/`](./specs/002-phase-2-plugins-index/)
- **Phase 3 specification:** [`specs/003-phase-3-mcp-workspaces/`](./specs/003-phase-3-mcp-workspaces/)
- **Contributor on-ramp:** [`CONTRIBUTING.md`](./CONTRIBUTING.md)

## Licence

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](./LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](./LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Tome by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
