---
title: Configuration
sidebar_position: 3
---

# Configuration

Tome keeps all of its state in one place. Every Tome-owned path lives under a
single root — `~/.tome/` — so you can inspect, back up, or remove Tome's state
by working with one directory. Project directories carry a small marker so Tome
knows which workspace they belong to.

## `~/.tome/` layout

```
~/.tome/
├── config.toml          # global configuration (includes the [harness] settings layer)
├── index.db             # the central SQLite search index
├── index.lock           # the index advisory lock
├── catalogs/            # cloned catalog content
├── models/              # downloaded embedding + rerank models
├── logs/                # logs
└── workspaces/
    └── <name>/
        ├── settings.toml   # workspace-layer settings
        └── RULES.md        # workspace-layer rules surface
```

There is exactly one central `index.db`, one `index.lock`, and one global
`config.toml`. Per-workspace state lives under `workspaces/<name>/`.

A few details worth knowing:

- **`index.lock`** is an advisory lock guarding index writes. Read-only
  commands — `tome status` in particular — never take it, so they are always
  safe to run while something else is indexing.
- **`catalogs/`** holds shared clones, reference-counted across everything
  that uses them. Adding the same catalog twice doesn't clone it twice, and a
  clone is deleted only when the last reference to it is removed.

## Global `config.toml`

`~/.tome/config.toml` holds global configuration. Tome bootstraps the `~/.tome/`
root on first write. Tome-owned config is parsed strictly — unknown fields are
rejected — so a typo surfaces as an error rather than being silently ignored.

If the file is malformed, ordinary commands fail loudly with an exit-5 parse
error that names the offending key, section, and line. The two read-only
diagnostics — `tome doctor` and `tome status` — are the exception: they keep
running and report the same parse problem as a finding (so the command you would
reach for to *diagnose* a broken config is never itself bricked by it). Fix the
named key; Tome never rewrites your config for you.

## Config keys

`config.toml` holds a curated set of scalar knobs, grouped into sections. Every
key is optional — an absent key takes its built-in default — so you only write
the ones you want to change. Env vars override the file at each consumer (see
[Environment variables](#environment-variables)); the file is the persistent
middle layer.

| Section | Key | Effect |
| --- | --- | --- |
| `[query]` | `top_k` | Number of results to return. |
| | `rerank` | Whether to run the reranker over the KNN hits. |
| | `strict_min_score` | Minimum score under `--strict` before a result is dropped. |
| `[summariser]` | `enabled` | Turn skill summarisation on or off. |
| | `long_max_chars` | Upper bound on a long summary (clamped `1500..=8000`). |
| | `provider` / `model` | Point summarisation at an external [provider](#model-providers-byokbyom). |
| `[logging]` | `level` | Log verbosity (`off`, `error`, `warn`, `info`, `debug`, `trace`). |
| `[output]` | `color` | Colour mode (`auto`, `always`, `never`). |
| | `progress` | Whether to render progress bars. |
| `[workspace]` | `default` | Global default workspace, overriding a project marker. |
| `[mcp]` | `description_max_chars` | Cap on the entry description length the MCP server returns. |
| `[models]` | `profile` | Active model tier (`small`, `medium`, `large`). |
| `[doctor]` | `verify_by_default` | Run `doctor`'s deeper checks without passing `--verify`. |
| `[harness]` | `default_scope` | Default target for `tome harness use`/`remove` (`project` or `global`). |
| `[hooks]` | `translate_plugin_hooks` | Whether to translate plugin hooks into native harness hooks (default on). |
| `[telemetry]` | `enabled` | Opt out of anonymous telemetry (`false`). |
| | `endpoint` | Override the telemetry collector endpoint. |

`tome config show` prints every one of these knobs with its effective value and
where that value came from — `(default)`, `(config)`, or `(env)`.
`tome config validate` runs the strict parse and either confirms the file is
valid or prints the same key-naming exit-5 error the loud path emits. Both are
read-only. See [`tome config`](./commands.md#tome-config).

The provider-wiring fields (`[providers]` and the capability
`provider`/`model` references) are covered separately below; `config show` omits
them because a provider entry can carry an inline credential.

## Settings layers

Harness composition is resolved from layered settings:

- **Global** — the `[harness]` section of `~/.tome/config.toml`
- **Workspace** — `~/.tome/workspaces/<name>/settings.toml`

The workspace layer composes over the global layer to produce the configuration
written to each [harness](../using-tome/harnesses.md). Edits are made
surgically, preserving comments and key order — Tome never rewrites a settings
file wholesale.

## Project markers

A project bound to a workspace carries a marker directory at the project root:

```
<project>/.tome/
├── config.toml          # which workspace this project maps to
└── RULES.md             # a copy of the workspace-layer rules
```

This is how working inside a project activates the right
[workspace](../using-tome/workspaces.md) composition automatically.

A global `[workspace] default` takes priority over a project marker. If you set
one while a project marker is present, the default wins and the project binding
goes inactive — so Tome prints a one-line `note:` on stderr saying the
per-project sync is inactive. To restore the per-project binding, unset
`[workspace] default` or run `tome workspace use` in the project.

## Models

Downloaded models live under `~/.tome/models/`, each with its own
`manifest.toml`. (Older installs that still have a `manifest.json` are migrated
in place by `tome doctor --fix` — no re-download.) Manage models with
`tome models {download,list,remove}` against a pinned registry.

The active tier is the `[models] profile` (`small`, `medium`, or `large`).
`tome models download --profile <tier>` pre-fetches a specific tier's weights
without switching the active profile, so you can pull `medium` before committing
to it. `tome models test <capability>` runs one real round-trip against the
active `summariser`, `embedding`, or `reranker` — the configured remote provider
if you set one, else the bundled local model — and `--verify` additionally
rehashes the bundled artefact against its pinned SHA-256. See the
[Commands reference](./commands.md#tome-models).

## Model providers (BYOK/BYOM)

Each model capability can point at an external provider instead of the bundled
local model. Bundled-local is the default: when nothing is configured, Tome runs
its own `bge-small` embedder, `bge-reranker`, and Qwen summariser, and makes no
external call.

You declare providers once in a registry, then reference them per capability.

```toml
[providers.openai]
kind = "openai"
# base_url is optional; it defaults per kind
# api_key is optional inline; the env var below is preferred

[providers.voyage]
kind = "voyage"

[summariser]
provider = "openai"
model = "gpt-4o-mini"

[embedding]
provider = "openai"
model = "text-embedding-3-small"
dimensions = 1536

[reranker]
provider = "voyage"
model = "rerank-2"
```

A `[providers.<name>]` entry has three fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `kind` | yes | `openai`, `anthropic`, `gemini`, or `voyage`. Fixes the wire shape and the default `base_url`. |
| `base_url` | no | Override the endpoint (for an OpenAI-compatible local server, an enterprise gateway, a proxy). A trailing `/` is trimmed. |
| `api_key` | no | An inline credential. The derived env var below is preferred; see credential resolution. |

Each capability references a provider by name and requires a `model`. The
allowed provider kinds differ per capability:

| Capability | Section | Allowed kinds |
| --- | --- | --- |
| Summarisation | `[summariser]` | `openai`, `anthropic`, `gemini` |
| Embedding | `[embedding]` | `openai`, `voyage` |
| Reranking | `[reranker]` | `voyage` |

Setting `provider` requires `model`. A `provider` referencing a name that isn't
in the registry, a kind that's illegal for the capability, or a `provider`
without a `model` is a resolve-time config error (exit `93`). The `[embedding]`
section also takes `dimensions` — when set, it's the authoritative expected
vector length, and a remote embedding of a different length is rejected (exit
`95`) rather than indexed.

### Credential resolution

A provider credential resolves from one of two sources, in order:

1. **The derived env var `TOME_<NAME>_API_KEY`** — the registry name uppercased,
   with every non-alphanumeric character replaced by `_`. Provider `openai`
   reads `TOME_OPENAI_API_KEY`; provider `my-prov.2` reads
   `TOME_MY_PROV_2_API_KEY`. An empty value is ignored.
2. **The inline `api_key`** in the `[providers.<name>]` entry, used only when the
   env var is unset or empty.

Tome deliberately never reads a conventional vendor variable like
`OPENAI_API_KEY` — only the derived `TOME_<NAME>_API_KEY`. If two registry names
derive the same env var, Tome warns that the override is ambiguous but still
resolves.

A configured external provider with no resolvable credential is a config error,
not a request failure: `tome models test <capability>` and `tome doctor` surface
it at resolve time (exit `93`), and the message names the exact env var to set
rather than failing with a `401` deep in a run. A provider that legitimately
needs no auth (a local OpenAI-compatible server) is fine — leave both sources
unset and Tome sends no credential.

## Environment variables

Environment variables override config keys and tune runtime behaviour. The
provider and telemetry variables are specific to this page; the general CLI
variables are documented in full under
[Global behaviour](./commands.md#global-behaviour).

| Variable | Effect |
| --- | --- |
| `TOME_<NAME>_API_KEY` | Per-provider BYOK credential; wins over an inline `api_key`. See credential resolution above. |
| `TOME_PROVIDER_MAX_RETRIES` | Retries beyond the first attempt for a remote provider request. Default `2` (3 total attempts). An unset, empty, or non-numeric value falls back to the default; a valid number is clamped to `0..=10`, so a value above `10` becomes `10`. |
| `TOME_PROVIDER_TIMEOUT_SECS` | Per-request timeout in whole seconds. Default `30`; a missing or unparsable value uses the default. |
| `TOME_GAUGE_ENDPOINT` | Override the telemetry collector endpoint (must be `https` or a loopback address). Wins over `[telemetry] endpoint`. |
| `TOME_TELEMETRY` | `1` forces telemetry on (beating the CI auto-disable); `0` forces it off. |

The general CLI variables — `TOME_LOG` / `RUST_LOG`, `TOME_WORKSPACE`,
`TOME_JSON`, `TOME_NO_COLOR` / `NO_COLOR`, `TOME_NONINTERACTIVE`, and
`TOME_MCP_LOG` — are covered in the
[Commands reference](./commands.md#global-behaviour).
