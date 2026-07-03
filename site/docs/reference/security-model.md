---
title: Security model
sidebar_position: 4
---

# Security model

When you add a catalog and enable its plugins, you allow **third-party
instructions to be executed inside your coding agent** — an agent that has
access to your shell, your filesystem, and possibly your credentials. Tome
cannot remove that risk. Instead, it splits the problem into two boundaries
and defends one of them.

## Two boundaries

Tome treats catalog content as untrusted **input** and defends the
*mechanical* boundary — parsing it, indexing it, and writing harness config
without letting that content affect anything outside its intended targets. The
*semantic* boundary — whether the instructions themselves are something your
agent should follow — is a judgment only you can make.

```
        catalog content (untrusted)
                   │
                   ▼
 ┌─────────────────────────────────────┐
 │  mechanical boundary — Tome's job   │  bounded reads · path validation
 │  parse, index, write harness config │  symlink refusal · scrubbed errors
 └─────────────────┬───────────────────┘
                   │  delivered faithfully
                   ▼
 ┌─────────────────────────────────────┐
 │  semantic boundary — your job       │  "should my agent follow
 │  choose what to enable              │   these instructions?"
 └─────────────────┬───────────────────┘
                   ▼
         your agent executes it
```

## Adding a catalog is a trust decision

**Adding a catalog means trusting it — the same trust decision you make when
you run `npm install` or run a script someone sent you.** Tome defends the
mechanics described below, but it cannot — and does not claim to — review the
*content* a catalog ships. Only add catalogs you trust, and treat "add this
catalog" in a README with the same caution as "pipe this script to `sh`".

## What Tome defends — the mechanical boundary

Each of these defences is implemented in the CLI today:

- **Symlink-refusing writes into harness config.** Before Tome writes into a
  harness's config tree, it refuses symlinked path components. A plugin can't
  use a symlink to redirect Tome's write to another location on your machine.
- **Path validation on third-party names.** Plugin- and entry-supplied names
  that become file names are validated as single safe path segments — no
  `../`, no absolute paths, no escaping the directory the write was aimed at.
- **Bounded reads of third-party files.** Manifests, frontmatter, and skill
  bodies are read under size caps, so a hostile or simply broken file can't
  consume unbounded memory.
- **Managed-marker collision refusal.** Where third-party prose is copied
  verbatim into a marker-delimited region of a file Tome manages, content that
  contains Tome's own markers is refused outright — it can't escape its region
  or corrupt the surrounding file.
- **Credential scrubbing in errors.** Tokens embedded in git remotes and
  download URLs are scrubbed before any error message or log line is emitted.
  The same scrubber runs over config parse diagnostics, so a syntax error on an
  inline `api_key` line never echoes the key.
- **Index and models are local.** The index, the embeddings, and the reranker
  all run on your machine, and a search never leaves the box.

The result: a hostile catalog cannot escape Tome's target directories,
corrupt the files Tome manages, or leak your credentials through error output.

## What Tome sends off your machine

Two paths send data to a remote server. Both are legible and controllable.

**Anonymous telemetry, opt-out.** By default Tome sends anonymous usage
telemetry to a pinned collector endpoint. Every event field is a bucketed
integer, a closed enum, a boolean, or a per-install UUID — no free-form strings,
no paths, no catalog content, no query text. Telemetry auto-disables under CI
and never blocks the foreground: a command only appends to a local queue, and a
detached background flusher drains it best-effort. Disable it with
`tome telemetry off`, force it either way with `TOME_TELEMETRY=0` / `1`, and
inspect exactly what is queued with `tome telemetry inspect` (it prints the
pending events and sends nothing). See [`tome telemetry`](./commands.md#tome-telemetry).

**BYOK/BYOM model providers, off by default.** Tome runs bundled local models
unless you configure an external provider. When a `[providers.<name>]` entry is
wired to a capability, that capability's requests — the text to summarise,
embed, or rerank — go to the provider endpoint you named, authenticated with the
credential you supplied. Nothing external is called until you configure it. See
[Model providers](./config.md#model-providers-byokbyom).

:::note
These refusals report exactly what happened. A refused write to a dedicated
sink exits with the code for its target—`44` (hooks), `45` (agents), `46`
(guardrails)—rather than a generic I/O error, so scripts can tell "blocked a
suspicious write" apart from "disk problem". The rules-file sink (`CLAUDE.md` /
`AGENTS.md`) and the MCP-config sink refuse the same way but surface it as exit
`7` (I/O), the code those sinks already used. See
[Exit codes](./exit-codes.md).
:::

## What Tome does not defend — the semantic boundary

A catalog's skills, commands, and agents are **instructions your AI agent
executes**. Tome delivers them faithfully; it cannot judge whether they are
benign. A hostile `SKILL.md` can tell your agent to exfiltrate secrets or run
destructive commands — and it won't be written as code. It will be written as
helpful prose.

The defence here is the same one you already use for dependencies: provenance.
Enable plugins from authors you trust, and read a plugin's content before you
enable it.

## A note for catalog authors

If you publish a catalog, you are asking users for exactly this trust. Make
it easy to grant: keep your source public, describe what each plugin does, and
don't ship anything you wouldn't want quoted in a security incident report.
See [Distributing your catalog](../authoring/distributing.md).

## Reporting a vulnerability

See `SECURITY.md` in the Tome repository.
