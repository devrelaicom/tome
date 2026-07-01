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
tome workspace current                      # just the bound name, one line (for prompts/scripts)

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
tome models profile                         # show the active profile (small/medium/large) + its models
tome models profile large                   # switch the active profile (prints a reindex notice if needed)
tome models list                            # installed models, the profile(s) that use each, and the active set
tome models list --verify                   # re-hash each artefact against its pinned SHA-256
tome models download                        # fetch the active profile's models up front
tome models download --all                  # fetch every model in every profile
tome models remove <name>
tome models test <summariser|embedding|reranker>  # one real round-trip against the active model
```

`tome models test` exercises the **active** model for a capability — the configured external provider if one is set (see [External model providers](#external-model-providers-byokbyom)), otherwise the bundled local model — and reports success (with latency + the validated result shape) or a precise failure, without writing any state.

`tome models list` annotates each row with the profile(s) that reference it and marks the active set with `*`. `tome models download` defaults to just the active profile's `{embedder, reranker, summariser}`; pass `--all` to fetch every tier's weights.

## Configuration

Every Tome-owned path lives under **`~/.tome/`**:

- `~/.tome/config.toml` — global config (strict TOML, `0600` on Unix); see [Global config reference](#global-config-reference) below
- `~/.tome/index.db` — central SQLite + `sqlite-vec` (one DB; workspaces and catalog enrolments are junction-keyed)
- `~/.tome/index.lock` — single advisory lockfile
- `~/.tome/catalogs/<sha>/` — shared catalog clones (reference-counted)
- `~/.tome/models/<name>/` — embedder, reranker, and summariser weights + a manifest with the pinned SHA-256
- `~/.tome/workspaces/<name>/{settings.toml, RULES.md}` — per-workspace state
- `~/.tome/logs/mcp.log` (+ `mcp.log.1` rotation) — JSON-lines, 10 MiB cap, `0600` on Unix

**Per project** (bound via `tome workspace use`):

- `<project>/.tome/config.toml` — a pointer marker (`workspace = "<name>"`); the central registry is the source of truth.
- `<project>/.tome/RULES.md` — propagated from the workspace's `RULES.md` on every sync.

### Global config reference

`~/.tome/config.toml` is the single file for all global Tome settings. It is created with built-in defaults on first run (mode `0600` on Unix). Every key is optional; unset keys inherit the built-in default listed below.

**Precedence:** CLI flag (per run) > environment variable (where one exists) > `~/.tome/config.toml` > built-in default.

```toml
# ~/.tome/config.toml

[harness]
# Harnesses active at global scope. Re-run `tome harness use <name>` to populate
# this; the `enabled` list is written for you.
enabled = ["claude-code"]
expose_agents_as_personas = false
strip_plugin_agent_privileges = false
# Default --scope for `tome harness use` / `tome harness remove` (project|global).
default_scope = "project"

[query]
# Default result count for `tome query` (CLI) and `search_skills` (MCP).
top_k = 10
# Cross-encoder reranker on/off (applies to both CLI and MCP).
rerank = true
# Minimum score threshold applied when --strict is passed.
strict_min_score = 0.5

[summariser]
# Auto-regenerate workspace summaries on plugin enable/disable/reindex/update.
enabled = true
# Long-summary character cap. Clamped to the range 1500..=8000.
long_max_chars = 2500
# BYOK (optional): name of a [providers.*] entry to summarise with, instead of
# the bundled Qwen. When set, `model` is required. Omit → bundled local model.
# provider = "openai"
# model    = "gpt-4o-mini"

# BYOK/BYOM — external model providers (optional). See "External model
# providers" below. Each [providers.<name>] declares a `kind` and an optional
# `base_url` (defaulted per kind) + optional inline `api_key` (env wins).
# [providers.openai]
# kind     = "openai"            # openai | anthropic | gemini | voyage
# base_url = "https://api.openai.com/v1"   # or e.g. http://localhost:11434/v1 (Ollama)
# api_key  = "sk-..."            # optional; env TOME_OPENAI_API_KEY wins if set

# [embedding]                    # omit → bundled bge per [models] profile
# provider   = "openai"          # allowed kinds: openai, voyage
# model      = "text-embedding-3-small"
# dimensions = 1536              # optional; authoritative expected dimension

# [reranker]                     # omit → bundled reranker per profile
# provider = "voyage"            # voyage only (v1)
# model    = "rerank-2"

[telemetry]
# Set to false or run `tome telemetry off` to opt out.
# TOME_TELEMETRY=0 and CI environments override this.
enabled = true

[logging]
# Log level for the tracing subscriber (stderr).
# Overridden by TOME_LOG / RUST_LOG env vars and the -v / -vv CLI flags.
# Values: off | error | warn | info | debug | trace
level = "warn"

[output]
# Colour output mode. Overridden by the NO_COLOR env var and --no-color flag.
# Values: auto | always | never
color = "auto"
# Show spinners and progress bars even on a TTY. Set to false to suppress them.
progress = true

[workspace]
# Workspace used when no --workspace flag, TOME_WORKSPACE env var, or project
# marker is found.
default = "global"

[mcp]
# Maximum characters for the description field returned by search_skills.
description_max_chars = 150

[models]
# Bootstrap profile for a FRESH index (small | medium | large).
# The value stored in index.db wins once the index has been created.
profile = "medium"

[doctor]
# Make `tome doctor` behave as if --verify was always passed.
verify_by_default = false
```

### Migration notes (pre-release breaking changes)

The following changes land in the current unreleased version on `main`. They affect any existing Tome install that used the old separate config files.

**`~/.tome/settings.toml` is no longer read.**
Global harness settings (`enabled`, `expose_agents_as_personas`, etc.) have moved into `~/.tome/config.toml [harness]`. Tome will not migrate the old file automatically.

- Re-run `tome harness use <name>` for each harness you had configured globally; this writes the `[harness] enabled` list for you.
- Delete `~/.tome/settings.toml` for tidiness once you have re-declared your harnesses.

**`~/.tome/telemetry/config.toml` is no longer read.**
The telemetry opt-out now lives in `~/.tome/config.toml [telemetry] enabled = false`. Tome will not carry over the old opt-out automatically.

- If you had previously run `tome telemetry off`, re-run it (or add `enabled = false` under `[telemetry]` in `~/.tome/config.toml`).
- Delete `~/.tome/telemetry/config.toml` for tidiness.

**`config.toml [catalogs]` table is dropped.**
The `[catalogs]` table in the global config was dead (the SQLite DB is the authoritative registry). A stale `[catalogs]` table is tolerated and silently ignored on read; it will be dropped the next time the file is written.

**A malformed `~/.tome/config.toml` now surfaces as exit 5.**
Foreground commands that read the global config fail loudly with exit 5 if the file is unparsable. Best-effort paths (telemetry, logging, colour/progress, the post-commit summariser trigger) degrade gracefully instead of aborting.

**New `--no-color` global flag.**
`tome --no-color <command>` suppresses colour output regardless of TTY state. The `NO_COLOR` environment variable and `[output] color = "never"` in config also suppress colour.

### External model providers (BYOK/BYOM)

Each of Tome's three model capabilities — **summarisation**, **embedding**, and **reranking** — can be pointed at an external provider instead of the bundled local model. Leave a capability unconfigured to keep today's bundled behaviour exactly. Configure a provider in `~/.tome/config.toml`:

1. Declare a provider in the `[providers.<name>]` registry: a `kind` (`openai` | `anthropic` | `gemini` | `voyage`), an optional `base_url` (defaulted per kind; set it explicitly for a local OpenAI-compatible server such as Ollama or LM Studio), and an optional inline `api_key`.
2. Point a capability at it: `provider` + `model` on `[summariser]`, or the `[embedding]` / `[reranker]` sections.

| Capability | Allowed provider kinds | Apply the switch with |
|---|---|---|
| summarisation | `openai`, `anthropic`, `gemini` | `tome workspace regen-summary` |
| embedding | `openai`, `voyage` | `tome reindex` (a drift guard requires it) |
| reranking | `voyage` | takes effect on the next `tome query` |

**Credentials** resolve in this order: the environment variable `TOME_<NAME>_API_KEY` (where `<NAME>` is the registry name uppercased with non-alphanumerics → `_`) → the inline `api_key` → none (valid for a local no-auth server). Tome never reads a generic `OPENAI_API_KEY`; credentials are never written to logs or error output.

**Verify before a (paid) reindex:**

```sh
tome models test embedding                  # one real round-trip against the active model
tome models test summariser                 # — remote if configured, else bundled local
tome models test reranker
tome doctor                                  # provider report: kind + credential resolvable?
tome doctor --verify                         # + one lightweight reachability call per provider
```

Switching the embedding model invalidates the index: `tome query` and the embedding-writing commands (`tome plugin enable`, `tome catalog update`) fail with a clear "run `tome reindex`" error until you reindex — Tome never auto-incurs a (possibly paid) reindex. A malformed remote embedding (empty, non-finite, zero-norm, or wrong-dimension) is rejected fail-closed and never written to the index. No streaming, no batch embedding, and no per-workspace overrides in v1 (embedding is global — there is one shared index).

### Models

Tome downloads pinned models on demand into `~/.tome/models/`. Each download is verified against a pinned SHA-256; a mismatch aborts the install.

#### Profiles

A **profile** (`small`, `medium`, `large`) selects which embedder + reranker Tome uses. Larger profiles trade disk and CPU for retrieval quality. The summariser is the same across every profile. The default for a fresh install is **`medium`**.

| Profile | Embedder | Embedding dim | Reranker | Models to download |
|---------|----------|---------------|----------|--------------------|
| `small` | `bge-small-en-v1.5` (~34 MB) | 384 | `bge-reranker-base` (~279 MB) | ~804 MB |
| `medium` *(default)* | `bge-base-en-v1.5` (~110 MB) | 768 | `bge-reranker-large` (~563 MB) | ~1.16 GB |
| `large` | `bge-large-en-v1.5` (~337 MB) | 1024 | `bge-reranker-v2-m3` (~571 MB) | ~1.40 GB |

Every embedder and reranker is a single-file quantized BGE model under the **MIT** licence; the shared summariser `qwen2.5-0.5b-instruct` (Q4_K_M GGUF, ~491 MB) is **Apache-2.0**. The "models to download" column includes the summariser.

```sh
tome models profile                         # show the active profile + its embedder/reranker/state
tome models profile large                   # switch to the large profile
```

#### Switching the embedder requires a reindex, never a migration

Each profile's embedder produces vectors of a different dimension (384 / 768 / 1024). When you switch to a profile whose embedder differs from the one your index was built with, `tome models profile <tier>` prints a notice and **does not** migrate or auto-rebuild your stored vectors:

```
! embedder changed (dim 768→1024); run `tome reindex` to re-embed the index
```

Run `tome reindex` to re-embed every enabled skill with the new embedder. Tome never attempts to "convert" existing vectors between dimensions — re-embedding from the source skills is the only correct path, and it is the one Tome takes. Until you reindex, the stored vectors still match the previous embedder; the drift is reported by `tome status` / `tome doctor` and blocks partial re-embeds (`plugin enable`, `catalog update`) so a half-migrated index can never occur. Switching the reranker only (or moving between profiles that share an embedder) needs no reindex; if the new reranker isn't downloaded yet, the switch hints `tome models download`.

#### Existing installs

An index created before profiles existed was built with `bge-small-en-v1.5`, so it is **auto-mapped to the `small` profile** on first open — no reindex, no re-download, seamless. Its stored 384-d vectors already match the small embedder, so nothing changes until you explicitly switch profiles.

#### Downloading

The first `tome plugin enable` on a fresh machine prompts to download the active profile's set. `tome models download` fetches the active profile's models; `tome models download --all` fetches every model in every profile. The summariser generates workspace summaries; if you decline it at enable time, summary generation is silently skipped until it's present — indexing and search still work with just the embedder + reranker.

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
