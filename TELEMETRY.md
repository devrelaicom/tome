# Tome Telemetry

Tome collects **anonymous, opt-out** usage telemetry to understand which features
are used and where the tool breaks, so the project can be improved. This document
is the **complete, authoritative** description of what is collected, how it leaves
your machine, and how to turn it off. It is **pinned to the code** by an
integration test (`tests/telemetry/telemetry_md_pin.rs`): the two worked examples
below are compared byte-for-byte against the actual event constructors, so this
document cannot silently drift from what Tome really sends.

**The one-line summary:** Tome sends bucketed counts, closed enum values, and
random UUIDs — never your queries, file paths, project names, or any free-form
text. A second, **opt-out-able-and-off-by-default-for-everyone-else** stream sends
the published name of a plugin **only** when that plugin comes from a small,
hardcoded, in-repo allowlist of catalogs (today: one — Midnight).

To turn it off: `tome telemetry off` (or set `TOME_TELEMETRY=0`). CI environments
are auto-disabled.

---

## 1. Identity model

Tome mints **one random v4 UUID per install**, stored locally at
`~/.tome/telemetry/id` (mode `0600`, owner-only). This UUID:

- is generated from OS randomness (`getrandom`), with the v4 version/variant bits
  set; it encodes **nothing** about you or your machine — it is pure randomness;
- is created race-safely (`O_CREAT|O_EXCL`) on the first run, and reused on every
  subsequent run;
- is the **only** stable identifier that appears on the wire. It is the join key
  that lets the project see, e.g., "this install searched, then invoked a skill"
  as one funnel rather than two unrelated events.

A second UUID, the **session UUID**, is minted fresh in memory on every process
start and is **never persisted**. It groups the events of a single CLI invocation
or a single MCP server session.

**Reset semantics:**

- `tome telemetry reset` regenerates the install UUID (severing all future
  continuity from past events) and clears the local queue.
- `tome telemetry purge` deletes the install UUID entirely, clears the queue, and
  disables telemetry until you re-enable it.

The install UUID's file modification time is the **mint time**, used only to
enforce the first-flush grace period (§4).

---

## 2. The full event catalogue

Every event is a **typed, closed record**. Each field is a closed enum, a bucketed
integer, a UUID, a boolean, a number, a timestamp, or — **only** on the
catalog-attributed stream (§3) — a published artefact name. There is no
free-form-string field anywhere in the anonymous stream.

### Shared envelope (on every event)

| Field | Type | Notes |
|---|---|---|
| `schema_version` | integer | event schema version, starts at `1` |
| `install_uuid` | UUID | stable per install |
| `session_uuid` | UUID | per process |
| `tome_version` | string | Tome's own version (e.g. `0.6.0`) |
| `os` | enum | `macos` \| `linux` (`windows` reserved) |
| `arch` | enum | `x86_64` \| `aarch64` |
| `timestamp` | string | RFC3339, millisecond precision, UTC |
| `event_type` | string | the dotted event name (closed set; see below) |
| `sample_rate` | number | always `1.0` on the anonymous stream; **omitted** on the attributed stream (those events are never sampled) |

`embedder_model_id` and `calling_harness` are **event-specific and nullable** —
when present they are closed values (a pinned model id / a `Harness` enum), never
free text — and are omitted when absent.

### Bucketed values

Counts and durations are never sent as raw numbers. They map to closed buckets:

- **Count buckets** (corpus / workspace / catalog / candidate counts): `0`,
  `1-4`, `5-19`, `20-99`, `100+`.
- **Latency buckets** (search compute time): `<50ms`, `50-200ms`, `200-500ms`,
  `500ms-1s`, `1s+`.
- **Load buckets** (embedder / index startup): `<100ms`, `100-300ms`,
  `300-1000ms`, `1s+`.
- **Rank buckets** (an entry's position in a preceding search): `1`, `2`, `3`,
  `4`, `5`, `6-10`, `11+`, `none` (no preceding search this session).
- **Findings buckets** (doctor): `0`, `1-4`, `5+`.

### Closed enums

`Harness`: `claude-code` \| `cursor` \| `codex` \| `opencode` \| `gemini-cli`.
`Surface`: `cli` \| `mcp`. `InstallMethod`: `cargo` \| `brew` \| `curl` \|
`unknown`. `EntryKind`: `skill` \| `command` \| `agent`. `PromptKind`: `command`
\| `persona` \| `builtin`. Plus the per-event action enums
(`Outcome{ok,partial,failed}`, `CatalogAction{added,removed,updated}`,
`PluginAction{enabled,disabled}`, `WorkspaceAction`, `HarnessAction`,
`AuthoringVerb`, `Artifact`, `SourceFormat`, `MetaAction`, `ReindexScope`,
`SourceType`, and the authoring `Outcome{ok,warnings,errors,strict_refused}`).
`error_class` is the closed `ErrorCategory` set (the same slugs Tome uses for its
exit-code categories) — **never** a raw error message.

`model_id` / `skill_id` / `embedder_model_id`, where present, are closed by
construction — they come from Tome's pinned model registry or its own embedded
meta-skill ids, never from user content.

### Anonymous events (`tome.*`) — 18 types

`install` (`install_method`), `upgrade` (`from_version` — Tome's own prior
version), `heartbeat` (bucketed skills/commands/agents/workspaces/catalogs counts
+ a sorted list of detected harnesses; at most once per UTC day), `search`
(`surface`, `latency_bucket`, `candidates_returned`, `reranker_used`, `strict`,
`corpus_size_bucket`, `embedder_model_id?`, `calling_harness?`), `entry_info`
(`rank_bucket`, `calling_harness?`), `entry_invoked` (`entry_kind`, `rank_bucket`,
`calling_harness?`), `prompt_invoked` (`prompt_kind`, `calling_harness?`),
`catalog_action` (`action`, `source_type`), `plugin_action` (`action`),
`workspace_action` (`action`), `harness_action` (`action`, `harness`),
`authoring_action` (`verb`, `artifact`, `source_format`, `outcome`), `meta_action`
(`action`, `outcome`), `model_download` (`model_id`, `outcome`, `error_class?`),
`cold_start` (`embedder_load_bucket`, `index_ready_bucket`, `embedder_model_id?`),
`doctor_run` (`fix`, `findings_bucket`), `reindex` (`scope`, `forced`, `outcome`),
`error` (`error_class`, `surface`, `calling_harness?`).

### Catalog-attributed events (`catalog.<id>.*`) — 6 types

See §3. `plugin_enabled` / `plugin_disabled`
(`plugin_name`, `plugin_version`, `catalog_id`), `plugin_updated`
(`plugin_name`, `from_version`, `to_version`, `catalog_id`), `entry_invoked`
(`entry_name`, `entry_kind`, `plugin_name`, `plugin_version`, `catalog_id`,
`calling_harness?`), `search_result` (`entry_name`, `entry_kind`, `plugin_name`,
`rank` — exact integer, `catalog_id`, `calling_harness?`), `error`
(`plugin_name`, `entry_name?`, `error_class`, `plugin_version`, `catalog_id`).

---

## 3. The allowlist and canonicalization

The catalog-attributed stream sends the **published name** of a plugin (and the
entries you invoke from it) **only** when the plugin's catalog resolves — at the
moment the event is emitted — to a source on a hardcoded, in-repo allowlist:

```text
ATTRIBUTED_TELEMETRY_CATALOGS = [
    ("midnight", "github.com/devrelaicom/midnight-expert-tome"),
]
```

This list is a compile-time constant. It can only change via a pull request to
Tome — there is **no remote configuration** and no way to widen it without a
reviewed, published code change. Removing an entry stops its attribution on the
next release, with no stored state to clear.

**The source is the gate, never the name.** A locally-authored skill named
identically to a Midnight skill produces **anonymous-only** events, because its
catalog's source URL does not match. Resolution canonicalizes the catalog's
enrolled source URL before comparison:

1. credentials are scrubbed (any `user:password@` is removed);
2. the bytes are UTF-8 decoded (a non-UTF-8 source yields no attribution);
3. the scheme is stripped (`https://`, `git://`, `ssh://`), the SSH `git@host:path`
   form is rewritten to `host/path`, the host is lowercased, and a trailing `.git`
   or `/` is removed.

So `https://github.com/devrelaicom/midnight-expert-tome`,
`git@github.com:devrelaicom/midnight-expert-tome.git`, and
`https://GitHub.com/devrelaicom/midnight-expert-tome/` all canonicalize equal and
all match; a different repository does not.

The `catalog_id` short id (e.g. `midnight`) is the **only** source-identifying
value that ever appears on the wire, and only on this stream.

---

## 4. Transport

**Nothing about delivery ever happens on a foreground path.** When you run a Tome
command or call an MCP tool, the only telemetry work performed is appending **one
line** to a local queue file — no network, no blocking, no waiting. This is a
hard, tested invariant.

- **Queue**: `~/.tome/telemetry/queue.jsonl` (mode `0600`), append-only, capped at
  1 MiB. Each event is one JSON line (≤ 4096 bytes) written with a single append.
  Over-cap events are dropped; the oldest are evicted at the next delivery.
- **Endpoint**: a single HTTPS endpoint compiled into the binary. It can be
  overridden for development or self-hosting with `TOME_TELEMETRY_ENDPOINT` (the
  override is HTTPS-only and is credential-scrubbed everywhere it is displayed).
- **Delivery** is best-effort and **lossy by design** — telemetry never retries
  aggressively, never blocks your work, and silently drops events rather than
  inconveniencing you:
  - The CLI, at process exit, may spawn a **detached background process**
    (`tome telemetry flush --quiet`) in a new session, with all I/O redirected to
    `/dev/null`, and exits **without waiting** for it. This child briefly appears
    in `ps` as `tome telemetry flush --quiet` before it finishes — that is the
    flusher, not a hidden daemon; it sends one batch and exits.
  - The MCP server flushes on a background timer (every few minutes, or when the
    queue grows past a threshold), never on a request path.
  - Both go through one synchronous drain: it acquires a non-blocking lock (a
    second flusher simply exits), reads the queue, POSTs batches over HTTPS (5 s
    timeout, no retry, `application/x-ndjson`), and only removes events from the
    queue **after** the server confirms receipt. A crash mid-flush loses nothing
    and never double-removes.
- **Batching**: anonymous and attributed events are sent as separate batches
  (`?stream=anonymous` / `?stream=catalog`), each capped at 100 events / 256 KiB.
- **Grace period**: nothing is ever sent within the **first 10 minutes** after the
  install UUID is minted. This guarantees the first-run disclosure (§7) is
  **seeable on your next CLI run** before any byte leaves the machine.

**Failure modes**: an unreachable or erroring endpoint is a silent no-op for
background delivery (the only command that surfaces an error is the explicit
foreground `tome telemetry flush`, which exits non-zero if the endpoint is
unreachable). A corrupt queue self-heals (unparsable lines are dropped on the
next drain). A backward clock keeps the grace period active (never sends early).

---

## 5. What is never emitted

Tome **never** emits, in any event:

- artefact names from **non-allowlisted** sources (the allowlist short id is the
  only source-identifying value, and only on the attributed stream);
- query strings, search terms, or tool inputs/outputs;
- file paths, file contents, or project / workspace / catalog **names or URLs**;
- environment variable values;
- hostnames, usernames, emails, or IP addresses;
- OS minor / build / kernel versions;
- geographic or locale data;
- raw error messages (only the closed `error_class` category);
- any free-form string that is not explicitly part of the schema above.

This is enforced **structurally** by the typed event API: a field that is not in
the schema cannot be constructed, so it cannot be sent. The only place a published
artefact name appears is the catalog-attributed stream, gated by §3.

---

## 6. The shared-UUID join and what a catalog owner can see

Both streams carry the **same install UUID**. The anonymous stream alone tells the
Tome project nothing about *which* plugins you use — only bucketed shapes of
activity. The attributed stream, for an allowlisted catalog, carries that
catalog's published plugin/entry names alongside the same install UUID.

This means the owner of an **allowlisted** catalog (today: only Midnight), if
given access to the attributed stream, can see the **published names and versions
of their own catalog's plugins/skills that a given install used** — keyed by the
random install UUID (not by anything identifying you). They cannot see your
queries, your other (non-allowlisted) plugins, your files, or who you are. A
non-allowlisted catalog's owner sees nothing — their plugins only ever produce
anonymous events.

Selection attribution (which search result you went on to invoke) is computed
server-side by joining `search_result` and `entry_invoked` on
`(session_uuid, entry_name)`; the client never back-edits queued events.

---

## 7. Opt-out and the `tome telemetry` CLI

Telemetry is **opt-out** and **default-on**, with two exceptions: it is
**auto-disabled in CI** (detected via `CI`, `GITHUB_ACTIONS`, `GITLAB_CI`,
`CIRCLECI`, `BUILDKITE`, `JENKINS_URL`, `TF_BUILD`, `TEAMCITY_VERSION`), and it can
be force-set with the `TOME_TELEMETRY` environment variable.

On the first CLI run with telemetry enabled, Tome prints a one-line disclosure to
**stderr** (once), then proceeds. The disclosure states that anonymous usage plus
named usage of allowlisted-catalog plugins is collected and points to
`tome telemetry --help`. The
guarantee is that this disclosure is **seeable on your next CLI run** (the 10-min
grace period, §4) — not that it was shown before any possible send.

**Precedence:** `TOME_TELEMETRY=1` (force on) > CI auto-disable > `TOME_TELEMETRY=0`
(force off) > the config file (`~/.tome/telemetry/config.toml`) > default-on.

| Command | What it does |
|---|---|
| `tome telemetry status [--json]` | Show enabled state + its source, the install UUID, the queued-event count, the last-flush time + HTTP status, and the (scrubbed) endpoint. Read-only — never mints. |
| `tome telemetry on` | Enable telemetry; mint the install UUID if absent. |
| `tome telemetry off` | Disable telemetry. The install UUID is left intact; `on` resumes it. |
| `tome telemetry inspect [--json]` | Pretty-print the pending queue **without sending**; the queue is byte-identical afterward. Reports any corrupt lines. |
| `tome telemetry flush [--quiet]` | Drain the queue now (foreground). Exits non-zero if the endpoint is unreachable; `--quiet` (the detached child) suppresses output and always exits 0. |
| `tome telemetry reset [--yes]` | Regenerate the install UUID and clear the queue (sever continuity). Prompts unless `--yes`. |
| `tome telemetry purge` | Delete the install UUID, clear the queue, and disable telemetry until re-enabled. |

`tome doctor` additionally reports the telemetry subsystem read-only (enabled
state, UUID file/mode/age, queue depth/oldest/corrupt, last flush, scrubbed
endpoint, and the allowlist). `tome doctor --fix` gains no new telemetry
capability — disabling is a user action, and a corrupt queue self-heals.

---

## 8. Worked examples

These two JSON lines are exactly what Tome emits for a `tome.install` event and a
`catalog.midnight.entry_invoked` event (with fixed example UUIDs / timestamps).
They are compared **byte-for-byte** against the real event constructors by
`tests/telemetry/telemetry_md_pin.rs`, so they cannot drift from the code.

<!-- TELEMETRY_PIN: tome.install -->
```json
{"schema_version":1,"install_uuid":"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f","session_uuid":"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f","tome_version":"0.6.0","os":"macos","arch":"aarch64","timestamp":"2026-06-11T14:11:45.123Z","event_type":"tome.install","sample_rate":1.0,"install_method":"brew"}
```

<!-- TELEMETRY_PIN: catalog.midnight.entry_invoked -->
```json
{"schema_version":1,"install_uuid":"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f","session_uuid":"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f","tome_version":"0.6.0","os":"macos","arch":"aarch64","timestamp":"2026-06-11T14:12:03.456Z","event_type":"catalog.midnight.entry_invoked","entry_name":"midnight-compact-debug","entry_kind":"skill","plugin_name":"midnight-expert","plugin_version":"1.2.0","catalog_id":"midnight","calling_harness":"claude-code"}
```
