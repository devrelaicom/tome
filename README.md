# Tome

**Tome is a Rust CLI _and_ MCP server** that makes Claude Code's plugin ecosystem work across other agentic coding harnesses — Cursor, Codex CLI, Gemini CLI, OpenCode, GitHub Copilot, Cline, Zed, and more.

Every coding agent has its own place to put knowledge — rules files, skill directories, MCP config — and none of them read each other's. Tome organises all of it behind a small set of concepts: you register **catalogs** (git repos of plugins), enable the **plugins** you want, and Tome builds a local semantic index of their skills and commands. From there you can search that index from your shell, organise work into named **workspaces**, and wire the whole thing into around sixteen coding harnesses — both as rules-file / MCP-config integration and as a live **MCP server** that lets your agent find and load exactly the skill it needs, mid-task, instead of holding everything in context.

```text
catalog (git repo)
  └─ plugins
       └─ entries (skills · commands · agents · hooks)
                │  tome plugin enable
                ▼
        central index — ~/.tome/  (SQLite + vector search, fully local)
                │
        ┌───────┴───────────────┐
        ▼                       ▼
    tome query              tome mcp
    (you, at a shell)       (your agent, mid-task)

tome harness use <name> writes each harness's native config:
Claude Code · Cursor · Codex · Gemini CLI · OpenCode · Copilot · Cline ·
JetBrains AI · Zed · Kiro · Devin · Junie · Antigravity · Pi · Crush · Goose
```

## What Tome does

- **Catalogs → plugins → index.** `tome catalog add` registers a git-hosted catalog; `tome plugin enable` indexes a plugin's skills and commands into a local SQLite + vector store. `tome query` runs semantic (KNN + reranker) search across everything enabled — entirely on your machine.
- **Named workspaces.** Central storage lives under `~/.tome/workspaces/<name>/`; a project binds to a workspace with a tiny `.tome/config.toml` pointer, so different projects can see different sets of plugins.
- **Integration across ~16 harnesses.** Tome writes each harness's rules file and MCP-config entry, delivers a tiered skill-routing directive at session start (via a session-start hook or a Tome-shipped plugin shim where supported, otherwise the rules file), and propagates per-plugin guardrails, hooks, and agent translations where the harness supports them. See the [harness-support matrix](#harness-support).
- **An MCP server.** `tome mcp` runs a stdio Model Context Protocol server backed by the resolved workspace's index, exposing `search_skills` / `get_skill_info` / `get_skill` plus a `meta` tool, and your enabled plugins' user-invocable commands as MCP prompts.
- **Authoring & conversion.** `tome {catalog,plugin,skill} create` scaffolds a new lint-clean artifact; `… convert` brings a Claude Code marketplace/plugin/skill, a Codex project, or a native `SKILL.md` into the native Tome format; `… lint` validates an artifact for CI.

## Install

Tome ships as a single self-contained executable — the semantic index, vector search, and reranker runtime are compiled in. The search models are downloaded on first use and kept under `~/.tome/` (see [Models](#models)).

### From crates.io

The crate is published as `tome-mcp`; the installed binary is named `tome`:

```sh
cargo install tome-mcp
```

### From Homebrew

Prebuilt binaries (produced by the cargo-dist release pipeline for Linux and macOS) are distributed through a Homebrew tap:

```sh
brew install aaronbassett/tap/tome
```

### From source

```sh
git clone https://github.com/devrelaicom/tome.git
cd tome
cargo install --path .
```

### Build prerequisites

Building from source (`cargo install tome-mcp` or `--path .`) needs:

- **Rust ≥ 1.93** (the pinned MSRV; edition 2024) and a system **`git`** on the executable path.
- **A C/C++ toolchain and CMake.** Tome statically links `llama.cpp` (via `llama-cpp-2`) and a vendored `sqlite-vec` extension, both compiled from source by `build.rs`.
- A network connection at build time: the `ort` crate (ONNX Runtime) **downloads the ONNX Runtime shared library during the build**.

The prebuilt Homebrew binaries ship with everything baked in, so they have none of the above build-time requirements.

Confirm the install with `tome --version`, then continue to the [Quick start](#quick-start).

## Quick start

Four commands take you from a fresh install to an agent that loads exactly the skill it needs during a task. The walkthrough uses the public **Midnight Expert** catalog as a concrete example — swap the source for any git-hosted catalog (an `owner/repo` shorthand, a full git URL, or a `file://` path to a local clone).

```sh
# 1. Register a catalog. Tome clones it and registers every plugin inside.
tome catalog add devrelaicom/midnight-expert-tome
tome catalog list                                # confirm it registered (as "midnight-expert")

# 2. Enable a plugin — parses, embeds, and indexes its entries for search.
#    Plugins are addressed as <catalog>/<plugin>.
tome plugin enable midnight-expert/midnight-verify
tome plugin list                                 # enabled plugins + per-plugin index status

# 3. Point a harness at Tome — writes that harness's native config.
tome harness use claude-code                     # one named harness
tome harness use cursor zed copilot              # several at once (variadic)
tome harness use                                 # every auto-detected harness
tome harness use --all                            # every supported harness

# 4. Search the index semantically.
tome query "verify a Compact contract"
tome query "deploy a contract" --top-k 5 --json
```

On a fresh machine, the first `tome plugin enable` prompts to download the pinned inference models (~804 MB total — see [Models](#models)); that step needs a network connection and a little patience. Pass `--yes` to skip the prompt.

Inside a configured harness, the same search runs over the [MCP server](#run-as-an-mcp-server): the agent searches, then loads only the top result instead of holding every indexed entry in context.

## Common usage

### Catalogs and plugins

```sh
tome catalog add <owner/repo | git-url | file://path>   # register a catalog
tome catalog list                                       # registered catalogs
tome catalog show <catalog>                              # every plugin a catalog offers
tome catalog update <catalog>                            # pull + re-resolve
tome catalog remove <catalog> --force                    # cascades plugin disable

tome plugin enable  <catalog>/<plugin>                   # index a plugin's entries
tome plugin disable <catalog>/<plugin>
tome plugin list                                         # enabled/disabled + index status
tome plugin show <catalog>/<plugin>                      # entries grouped by kind
tome plugin                                              # interactive catalog → plugin → action
```

### Search

```sh
tome query "how do I write a compact circuit"
tome query "deploy a contract" --top-k 5 --json
tome query "..." --strict                                # only high-confidence matches
```

### Harness integration

```sh
tome harness                                # list the supported harnesses
tome harness use claude-code                # add one harness to the project's settings
tome harness use cursor zed copilot         # add several (variadic)
tome harness use                            # configure every auto-detected harness
tome harness use --all                      # configure every supported harness
tome harness list                           # effective list with the source chain
tome harness info zed                       # detection + targets; prints the paste-able MCP snippet
tome sync                                    # reconcile rules + MCP config + hooks + guardrails + agents
tome sync --harness cursor --harness zed    # reconcile only the named harnesses (repeatable)
```

See the [harness-support matrix](#harness-support) for every harness Tome configures.

### Run as an MCP server

```sh
tome mcp                                    # stdio MCP server; launched by your harness
                                            # diagnostics → ~/.tome/logs/mcp.log (JSON-lines)
```

The server exposes a search-then-load flow — `search_skills` → `get_skill_info` → `get_skill` — plus a `meta` tool (installs a bundled meta skill into the host harness) and your plugins' user-invocable commands as MCP prompts. You normally don't launch this by hand: `tome harness use` / `tome sync` writes the wiring for you.

### Health and maintenance

```sh
tome status                                 # ok / degraded / unhealthy, per subsystem (read-only)
tome doctor                                 # detailed diagnostics + drift report
tome doctor --fix                           # repair drift in place
tome reindex                                # rebuild the index for every enabled plugin
tome models list                            # installed models; --verify rehashes vs pinned SHA-256
```

## Harness support

Tome configures around sixteen harnesses. For each, where the harness exposes a writable config, Tome registers the **Tome MCP server**; it always delivers the tiered skill-routing directive in the harness's **rules sink**; and, where the harness supports it, it delivers that directive at **session start** — through a session-start command hook, a Tome-shipped TypeScript plugin shim (executed by the harness's own runtime, never by Tome), or, for harnesses with neither, the rules file alone.

`tome harness use <name>` (and `tome sync`) target the **real** harnesses below — named, auto-detected, or selected with `--all`. The **opt-in targets** (`generic`, `generic-op` / `goose`) are reachable by name only; they are never auto-detected and never included in `--all`.

| Harness | MCP config | Rules sink | Session steering | Notes |
|---|---|---|---|---|
| `claude-code` | `.claude/.mcp.json` | `CLAUDE.md` | Session-start hook | Native hooks + agents |
| `codex` | `~/.codex/config.toml` | `AGENTS.md` | Session-start hook | |
| `cursor` | `.cursor/mcp.json` | `.cursor/rules/` | Rules only | Native agents |
| `gemini` | `~/.gemini/settings.json` | `AGENTS.md` / `GEMINI.md` | Session-start hook | `antigravity-cli` aliases here |
| `opencode` | `opencode.json` | `AGENTS.md` | TS plugin shim | |
| `copilot-cli` | `~/.copilot/mcp-config.json` | `.github/copilot-instructions.md` | Session-start hook | GitHub Copilot CLI |
| `copilot` | `.vscode/mcp.json` | `.github/copilot-instructions.md` | Rules only | Copilot in VS Code |
| `devin` | `.devin/config.json` | `AGENTS.md` | Session-start hook | |
| `cline` | `~/.cline/mcp.json` | `.clinerules/tome.md` | TS plugin shim | |
| `junie` | `.junie/mcp/mcp.json` | `.junie/AGENTS.md` | Rules only | EAP; no session hooks |
| `jetbrains-ai` | manual (UI only) | `.aiassistant/rules/tome.md` | Rules only | MCP added via IDE UI |
| `antigravity` | `~/.gemini/config/mcp_config.json` | `.agent/rules/tome.md` | Session-start hook | Hook path live-probe-gated |
| `pi` | manual (adapter) | `AGENTS.md` | TS plugin shim | MCP via the pi-mcp-adapter |
| `crush` | `crush.json` | `CRUSH.md` | Rules only | |
| `zed` | `settings.json` | `.rules` | Rules only | |
| `kiro` | `.kiro/settings/mcp.json` | `.kiro/steering/tome.md` | Rules only | |
| `goose` | `.mcp.json` (Open Plugins) | `AGENTS.md` | Open Plugins hook | Opt-in target (`generic-op`) |
| `generic` | `./mcp.json` | `AGENTS.md` | Rules only | Opt-in catch-all target |

For harnesses with a **manual MCP** step (`jetbrains-ai` is UI-only; `pi` needs the pi-mcp-adapter), `tome harness info <name>` prints the exact paste-able MCP-config snippet. Items marked live-probe-gated are confirmed against a real install before shipping; a harness whose session hook can't be confirmed falls back to rules-only steering. `tome status` and `tome doctor` report each harness's MCP state (`ok` / `manual` / `unverified` / `drift`). Every Tome-written rules sink opens with a self-healing preamble: if the agent can't see the Tome MCP tools, it tells the user to run `tome harness use <name>` (or `tome harness info <name>` for the snippet) and restart.

## Advanced usage

### Named workspaces

Different projects can see different sets of plugins. A workspace is a named, per-project scope held centrally; a project binds to one with a small pointer file.

```sh
tome workspace init my-project              # creates ~/.tome/workspaces/my-project/
tome workspace list                         # all workspaces + their counts
tome workspace use my-project               # bind the current project; writes .tome/config.toml + runs sync
tome workspace info                         # the resolved workspace for the current directory

# Target a workspace explicitly with the global --workspace flag:
tome --workspace my-project plugin enable midnight-expert/midnight-verify
```

### Scoped reindex

```sh
tome reindex                                # everything enabled
tome reindex <catalog>                      # one catalog
tome reindex <catalog>/<plugin> --force     # one plugin, forced rebuild
```

### Model management

```sh
tome models list                            # installed models + sizes
tome models list --verify                   # re-hash each artefact against its pinned SHA-256
tome models download                        # fetch the pinned set up front
tome models remove <name>
```

## Configuration

Every Tome-owned path lives under **`~/.tome/`**:

- `~/.tome/config.toml` — global config (strict TOML, `0600` on Unix)
- `~/.tome/settings.toml` — global harness composition settings
- `~/.tome/index.db` — central SQLite + `sqlite-vec` (one DB; workspaces and catalog enrolments are junction-keyed)
- `~/.tome/index.lock` — single advisory lockfile
- `~/.tome/catalogs/<sha>/` — shared catalog clones (reference-counted)
- `~/.tome/models/<name>/` — embedder, reranker, and summariser weights + a manifest with the pinned SHA-256
- `~/.tome/workspaces/<name>/{settings.toml, RULES.md}` — per-workspace state
- `~/.tome/logs/mcp.log` (+ `mcp.log.1` rotation) — JSON-lines, 10 MiB cap, `0600` on Unix

**Per project** (bound via `tome workspace use`):

- `<project>/.tome/config.toml` — a pointer marker (`workspace = "<name>"`); the central registry is the source of truth.
- `<project>/.tome/RULES.md` — propagated from the workspace's `RULES.md` on every sync.

### Models

Tome downloads three pinned models on demand into `~/.tome/models/`. Each download is verified against a pinned SHA-256; a mismatch aborts the install.

| Model | Role | Format | Approx. size | Licence |
|-------|------|--------|--------------|---------|
| `bge-small-en-v1.5` (Xenova INT8) | Embedder | ONNX | ~34 MB | MIT |
| `bge-reranker-base` (ONNX INT8) | Reranker | ONNX | ~279 MB | MIT |
| `qwen2.5-0.5b-instruct` (Q4_K_M) | Workspace summariser | GGUF | ~491 MB | Apache-2.0 |

The first `tome plugin enable` on a fresh machine prompts to download **all three (~804 MB total)**. The summariser generates workspace summaries; if you decline it at enable time, summary generation is silently skipped until it's present — indexing and search still work with just the embedder + reranker.

### Privacy & telemetry

**Tome collects anonymous, opt-out usage telemetry** to understand which features are used and where the tool breaks. It sends only bucketed counts, closed enum values, and random UUIDs — **never** your queries, file paths, project names, or any free-form text. A second stream sends the *published* name of a plugin only when that plugin comes from a small, hardcoded, in-repo allowlist of catalogs (today: one — Midnight). Nothing is ever sent on a foreground path: a command only appends one line to a local queue, and delivery is a best-effort background flush.

**To turn it off:** `tome telemetry off` (or set `TOME_TELEMETRY=0`). CI environments are auto-disabled. Use `tome telemetry status` to see the state and `tome telemetry inspect` to view the pending queue without sending. [`TELEMETRY.md`](https://github.com/devrelaicom/tome/blob/main/TELEMETRY.md) is the complete, code-pinned description of exactly what is collected.

Apart from telemetry, the only network access is git operations against the catalogs you explicitly register and the one-time model downloads above. The index, embeddings, and summaries are computed and stored locally under `~/.tome/`.

## Authoring & conversion

Tome artifacts are plain files: a plugin is a `tome-plugin.toml` plus `skills/<name>/SKILL.md`, `commands/<name>.md`, and `agents/<name>.md`; a catalog is a `tome-catalog.toml` plus its vendored plugins. Three command groups — each available at the `catalog`, `plugin`, and `skill` level — help you author and migrate them.

```sh
# Scaffold a new, lint-clean artifact from a built-in template.
tome skill create review                         # ./review/  : a plugin "review" wrapping skills/review/
tome skill create review --bare                  # ./review/SKILL.md  (a naked skill)
tome plugin create toolkit --into ./my-catalog   # scaffold + register in my-catalog/tome-catalog.toml

# Convert an existing artifact from another harness into the native Tome format.
tome plugin convert acme/cool-plugin             # owner/repo shorthand (shallow-cloned)
tome skill convert ./path/to/some-skill          # a local native SKILL.md (auto-detected)
tome plugin convert ./cc-plugin --dry-run        # print the plan; write nothing
tome plugin convert ./cc-plugin --strict         # abort if anything can't be represented

# Validate an artifact for CI (errors → exit 85; with --strict, warnings → 86).
tome plugin lint ./toolkit
tome plugin lint ./toolkit --strict --autofix    # apply mechanically-safe fixes
```

`convert` handles Claude Code marketplaces/plugins/skills, Codex projects, and native `SKILL.md` files from Cursor, OpenCode, Cline, and generic Agent Skills. It rewrites harness-isms (`${CLAUDE_PLUGIN_ROOT}` → `${TOME_PLUGIN_DIR}`, legacy `$1..$9` → 0-based) and honestly reports anything Tome cannot represent — monitors, themes, LSP servers, output-styles — as warnings (or, under `--strict`, aborts before writing). The source is never modified; remote sources are fetched into a temp clone that is always cleaned up.

### Meta skills

Tome also ships its own curated, trusted **meta skills** — native `SKILL.md` guides, embedded in the binary, that teach an agent how to use Tome itself. Installing one drops the guide into a harness's skills directory so it persists across sessions, in the harnesses that consume native skills (Claude Code, Cursor, Codex, OpenCode — not Gemini).

```sh
tome meta list                       # bundled meta skills + per-harness install status
tome meta add convert-marketplace    # install into every detected skill-capable harness (project scope)
tome meta add convert-marketplace --global          # install under your home instead
tome meta add convert-marketplace --harness cursor  # target named harness(es) only
tome meta remove convert-marketplace
```

The first bundled skill, **`convert-marketplace`**, walks an agent through converting a Claude Code marketplace into Tome — driving `tome catalog convert` / `tome catalog lint`, applying judgment to the unsupported residue, and reporting back for your confirmation before registering anything.

## Supported platforms

Linux and macOS, on both `x86_64` and `aarch64`. **Windows is untested** — it may build, but no support is offered and CI does not cover it.

## Security

Tome makes **mechanical** safety guarantees but cannot vet catalog **content** — only add catalogs you trust. See [`SECURITY.md`](https://github.com/devrelaicom/tome/blob/main/SECURITY.md) for the full trust model and how to report a vulnerability.

## Documentation

- **Full docs:** [tome-mcp.netlify.app](https://tome-mcp.netlify.app)
- **Project principles:** [`CONSTITUTION.md`](https://github.com/devrelaicom/tome/blob/main/CONSTITUTION.md)
- **Contributor on-ramp:** [`CONTRIBUTING.md`](https://github.com/devrelaicom/tome/blob/main/CONTRIBUTING.md)
- **Security & trust model:** [`SECURITY.md`](https://github.com/devrelaicom/tome/blob/main/SECURITY.md)
- **Telemetry (what's collected, opt-out):** [`TELEMETRY.md`](https://github.com/devrelaicom/tome/blob/main/TELEMETRY.md)

## Licence

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](https://github.com/devrelaicom/tome/blob/main/LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](https://github.com/devrelaicom/tome/blob/main/LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Tome by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.
