# Tome Telemetry

Tome collects **anonymous, opt-out** usage telemetry to understand which features
are used and where the tool breaks, so the project can be improved. This document
is the **complete, authoritative** description of what is collected, how it leaves
your machine, and how to turn it off.

**The one-line summary:** Tome sends closed enum values, raw integers, and random
UUIDs — never your queries, file paths, project names, or any free-form text. A
second stream sends the published name of a plugin **only** when that plugin comes
from a small, hardcoded, in-repo allowlist of catalogs (today: one — Midnight).

To turn it off: `tome telemetry off` (or set `TOME_TELEMETRY=0`). CI environments
are auto-disabled.

---

## 1. Backend and endpoint

Telemetry events are emitted through the **`gauge-telemetry` kernel**, which
manages consent, queue, delivery, and the wire format. The kernel owns the exact
wire contract; see the `gauge-telemetry` crate's `SPEC.md` for the authoritative
format description.

**Endpoint resolution** (highest priority first):

1. `TOME_GAUGE_ENDPOINT` environment variable
2. `[telemetry].endpoint` in `~/.tome/config.toml`
3. Default: `https://gauge-telemetry.fly.dev`

The resolved endpoint is HTTPS-only. Any credentials in the URL are scrubbed
before the value is displayed or logged.

---

## 2. Identity model

Tome mints **one random v4 UUID per install**, stored locally at
`~/.tome/telemetry/id` (mode `0600`, owner-only). This UUID:

- is generated from OS randomness, with the v4 version/variant bits set; it
  encodes **nothing** about you or your machine — it is pure randomness;
- is created race-safely (`O_CREAT|O_EXCL`) on the first run, and reused on
  every subsequent run;
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

---

## 3. The two event tiers

Every event is a **typed, closed record**. Each field is a closed enum, a raw
integer, a UUID, a boolean, or a string drawn from a closed/derived, coarse set
— never an arbitrary free-form string on the anonymous tier.

### Tier 1 — Anonymous (`tome.*`)

Anonymous events carry closed enum values, raw integers (e.g. latency in
milliseconds, result counts, inventory sizes), booleans, and UUIDs. The
low-frequency lifecycle events (`install`, `heartbeat`) additionally carry the
coarse **environment snapshot** of §6 — `os_version` (OS major version + id, e.g.
`darwin:14`), `language` (locale subtag only, e.g. `en`), `shell` (shell name
only, e.g. `zsh`), `cpu_cores`, `ram_gb`, `accel`, and `libc`. These string
fields are derived, low-cardinality values, not arbitrary free text; see §6 for
the exact derivation and what is dropped. No other free-form string appears on
this tier.

**Privacy model for quantities:** counts and durations are shipped as **raw
integers**. The server applies bucket groupings at read time; the client never
pre-buckets values before sending. This is enforced structurally — the typed
event API has no string field for these values, so a raw number that is not in
the schema cannot be constructed.

Events include: `install`, `upgrade`, `heartbeat`, `search`, `entry_info`,
`entry_invoked`, `prompt_invoked`, `catalog_action`, `plugin_action`,
`workspace_action`, `harness_action`, `authoring_action`, `meta_action`,
`model_download`, `cold_start`, `doctor_run`, `reindex`, `summary`, `error`,
and others. Fields such as `calling_harness` and `embedder_model_id` are
optional closed values — never free text.

### Tier 2 — Catalog-attributed (`tome.catalog_*`)

Catalog-attributed events carry the **published name** of a plugin and its
entries, as a `catalog` attribute on the event, alongside the same install UUID.
These events are emitted **only** for a catalog whose canonicalized source URL
matches the compiled-in allowlist (see §4).

Tier-2 fields include: `catalog` (the short allowlist id, e.g. `midnight`),
`plugin_name`, `plugin_version`, `entry_name`, `entry_kind`, and
`calling_harness` where relevant. All attribution is gated by
`allowlist::match_source` at the moment the event is emitted.

---

## 4. The allowlist and canonicalization

The catalog-attributed stream sends plugin and entry names **only** when the
plugin's catalog resolves — at the moment the event is emitted — to a source on
a hardcoded, in-repo allowlist:

```text
ATTRIBUTED_TELEMETRY_CATALOGS = [
    ("midnight", "github.com/devrelaicom/midnight-expert-tome"),
]
```

This list is a compile-time constant. It can only change via a pull request to
Tome — there is **no remote configuration** and no way to widen it without a
reviewed, published code change.

**The source is the gate, never the name.** A locally-authored skill named
identically to a Midnight skill produces **anonymous-only** events, because its
catalog's source URL does not match. Resolution canonicalizes the catalog's
enrolled source URL before comparison:

1. credentials are scrubbed (any `user:password@` is removed);
2. the bytes are UTF-8 decoded (a non-UTF-8 source yields no attribution);
3. the scheme is stripped (`https://`, `git://`, `ssh://`), the SSH
   `git@host:path` form is rewritten to `host/path`, the host is lowercased, and
   a trailing `.git` or `/` is removed.

So `https://github.com/devrelaicom/midnight-expert-tome`,
`git@github.com:devrelaicom/midnight-expert-tome.git`, and
`https://GitHub.com/devrelaicom/midnight-expert-tome/` all canonicalize equal
and all match; a different repository does not.

---

## 5. Transport

**Nothing about delivery ever happens on a foreground path.** When you run a
Tome command or call an MCP tool, the only telemetry work performed is appending
one event line to a local queue — no network, no blocking, no waiting. This is
a hard, tested invariant.

- **Delivery model:** zero foreground network. The CLI, at process exit, spawns a
  **detached background process** (`tome telemetry flush --quiet`) with all I/O
  redirected to `/dev/null` and exits without waiting. The MCP server flushes on
  a background `Flusher` thread. Both routes go through the `gauge-telemetry`
  kernel's drain; the kernel never fails the caller (best-effort, lossy by
  design).
- **Grace period:** nothing is ever sent within the **first 10 minutes** after the
  install UUID is minted. This guarantees the first-run disclosure (§6) is
  seeable on your next CLI run before any byte leaves the machine.
- **Failure modes:** an unreachable or erroring endpoint is a silent no-op for
  background delivery. A corrupt queue self-heals (unparsable lines are dropped
  on the next drain). A backward clock keeps the grace period active.

---

## 6. Environment snapshot, and what is never emitted

The low-frequency lifecycle events (`install`, `heartbeat`) carry a small,
coarse **environment snapshot** so we can understand the platforms Tome runs on.
Every field is low-cardinality and best-effort (an undetectable field is simply
omitted):

- `os_version` — the OS **major version and id only**, e.g. `darwin:14`,
  `ubuntu:22`, `windows:11`. The minor/patch/build/kernel numbers are dropped
  before sending — only the major version survives.
- `language` — the **locale subtag only**, derived from `$LANG`, e.g. `en`,
  `de`. The country, encoding, and any `@modifier` are stripped (`en_US.UTF-8`
  → `en`); the no-locale values `C`/`POSIX`/`C.UTF-8` are treated as absent.
- `shell` — the **shell name only**, derived from `$SHELL`, classified to a
  closed set (`bash`, `zsh`, `fish`, `pwsh`, `cmd`, or `other`). The shell's
  path is never sent.
- `cpu_cores` — the CPU core count (an integer).
- `ram_gb` — total physical RAM rounded to whole GB (an integer).
- `accel` — Tome's inference backend (always `cpu`).
- `libc` — on Linux only, `glibc` or `musl` (a compile-time property).

`os_version`, `language`, and `shell` are **derived** values: the full `$LANG`
and `$SHELL` environment-variable values are never sent — only the locale subtag
and the shell name. No other environment-variable value is read or emitted.

With that snapshot disclosed, Tome **never** emits, in any event:

- artefact names from **non-allowlisted** sources;
- query strings, search terms, or tool inputs/outputs;
- file paths, file contents, or project / workspace / catalog **names or URLs**;
- full environment-variable values (only the derived `language`/`shell` above);
- OS minor / patch / build / kernel versions (only the major version + id);
- hostnames, usernames, emails, or IP addresses;
- raw error messages (only the closed `error_class` category);
- any free-form string that is not explicitly part of the schema.

Beyond the disclosed environment snapshot, this is enforced **structurally** by
the typed event API: a field that is not in the schema cannot be constructed, so
it cannot be sent. The only place a published artefact name appears is the
catalog-attributed stream, gated by §4.

---

## 7. Opt-out and the `tome telemetry` CLI

Telemetry is **opt-out** and **default-on**. The opt-out stack (highest priority
first):

1. `GAUGE_TELEMETRY_DISABLE=1` — global kernel kill switch; always wins
2. `TOME_TELEMETRY=0` / `false` / `off` / `no` — Tome-level opt-out. Note:
   `TOME_TELEMETRY` is **opt-out-only**: any value that does not parse as a
   disable is ignored (it does not force-enable over CI auto-disable or the
   config)
3. CI auto-disable — detected via `CI` (any truthy value: set and non-empty,
   not `0`/`false`/`no`/`off`), `GITHUB_ACTIONS`, `GITLAB_CI`, `CIRCLECI`,
   `BUILDKITE`, `TF_BUILD` (same truthy rule), plus the presence markers
   `JENKINS_URL`, `TEAMCITY_VERSION`, `VERCEL`, `NETLIFY`, `TRAVIS`,
   `APPVEYOR`, `DRONE` (set to any non-empty value)
4. `[telemetry].enabled = false` in `~/.tome/config.toml`
5. `tome telemetry off` (writes the config flag)
6. Default: on

On the first CLI run with telemetry enabled, Tome prints a one-line welcome +
quickstart pointer, then the required one-line opt-out disclosure — both to
**stderr**, once — and proceeds (issue #313: the greeting leads, the disclosure
follows). The welcome is human-only (suppressed under `--json`); the disclosure
fires on first run regardless of mode. The guarantee is that the disclosure is
seeable before any byte leaves the machine (the 10-min grace period, §5).

| Command | What it does |
|---|---|
| `tome telemetry status [--json]` | Show enabled state + its source, the install UUID, the queued-event count, the last-flush time, and the (scrubbed) endpoint. Read-only. |
| `tome telemetry on` | Enable telemetry; mint the install UUID if absent. |
| `tome telemetry off` | Disable telemetry. The install UUID is left intact; `on` resumes it. |
| `tome telemetry inspect [--json]` | Pretty-print the pending queue **without sending**; the queue is byte-identical afterward. Reports any corrupt lines (exit 92 if any). |
| `tome telemetry flush [--quiet]` | Drain the queue now (foreground, via the kernel). Always exits 0 — the kernel drain is best-effort. `--quiet` (the detached child) suppresses output. |
| `tome telemetry reset [--yes]` | Regenerate the install UUID and clear the queue (sever continuity). Prompts unless `--yes`. |
| `tome telemetry purge` | Delete the install UUID, clear the queue, and disable telemetry until re-enabled. |

`tome doctor` additionally reports the telemetry subsystem read-only (enabled
state, UUID file/mode/age, queue depth/oldest/corrupt, last flush, scrubbed
endpoint, and the allowlist). `tome doctor --fix` gains no new telemetry
capability — disabling is a user action, and a corrupt queue self-heals.
