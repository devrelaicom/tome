---
title: Exit codes
sidebar_position: 2
---

# Exit codes

Tome exits `0` on success. Every failure class maps to its own specific non-zero
code — there is no generic "unknown error" arm — so you can branch on the exact
failure in scripts. The `--json` error output also includes a snake-case
`category` for each failure (mostly one per code, though a couple of codes share
a category — e.g. `52` and `73` both report `schema_too_new`). This table is
enforced against the CLI contract in CI.

## The `--json` error envelope

A failed command with `--json` prints one error record to **stderr** (never
stdout):

```json
{
  "error": {
    "category": "index_busy",
    "exit_code": 50,
    "message": "another tome process is updating the index; retry once it has finished\nhint: the advisory lock is held by a live process and self-heals when that process exits — there is no lock file to delete; retry shortly",
    "retryable": true
  }
}
```

The fields are:

- `category` — the snake-case failure class (the middle column of the table
  below).
- `exit_code` — the integer this command exited with.
- `message` — the human-readable message (already credential-scrubbed).
- `retryable` — a boolean, **always present**. `true` for transient or contended
  failures where retrying the same command unchanged could succeed
  (`index_busy`, `harness_clash`, a network/remote provider call, a git fetch);
  `false` for deterministic failures (a malformed manifest, an unknown catalog,
  a strict-mode verdict).
- `remediation` — the coarse `tome` command that fixes this class of failure,
  when a single one exists (e.g. embedder drift → `tome reindex --force`,
  `plugin_not_converted` → `tome plugin convert`). **Omitted entirely** when
  there is no single fix command. It is always a static `tome …` command hint —
  it never contains a path, credential, or other instance-specific value (those
  stay in `message`).

When a failure carries **both** `retryable: true` and a `remediation` (today
only `harness_clash`), the two are sequenced: apply the `remediation` **first**,
*then* retry. Re-running the identical command without applying the fix just
reproduces the same error.

Branch on `retryable` / `remediation` rather than string-matching `message`:

```sh
tome query "…" --json 2>err.json || {
  jq -e '.error.retryable' err.json >/dev/null && echo "will retry"
  fix=$(jq -r '.error.remediation // empty' err.json); [ -n "$fix" ] && echo "run: $fix"
}
```

The same `category` / `retryable` / `remediation` triple is attached to the MCP
tool error `data` payload (alongside its `code`), so an agent driving Tome over
MCP branches on the identical structured data.

Code `10` is special: it is not a failure class but a **health verdict** emitted
by `tome status` and `tome doctor` (see [Health verdicts](#health-verdicts-status--doctor)
below). It is never carried by the `--json` *error* envelope's `category`.

| Code | Category | Meaning |
| --- | --- | --- |
| `0` | — | Success. |
| `1` | `internal` | Internal error. |
| `2` | `usage` | Invalid usage / arguments. |
| `3` | `catalog_not_found` | Catalog not found. |
| `4` | `catalog_already_exists` | Catalog already exists. |
| `5` | `manifest_invalid` | Catalog manifest (`tome-catalog.toml`) invalid. |
| `6` | `git_failed` | A git operation failed. |
| `7` | `io` | I/O error. |
| `8` | `interrupted` | Interrupted (SIGINT / Ctrl-C). |
| `9` | `plugin_data_dir_write_failed` | Failed to write a plugin's data directory. |
| `10` | `health_degraded` | `tome status` / `tome doctor` health verdict: **degraded** (a non-fatal issue — queries still serve). See [Health verdicts](#health-verdicts-status--doctor). |
| `12` | `workspace_not_bound` | No workspace is bound to the current directory (`tome workspace current`). |
| `13` | `workspace_not_found` | Workspace not found. |
| `14` | `workspace_already_exists` | Workspace already exists. |
| `15` | `workspace_name_invalid` | Invalid workspace name. |
| `16` | `workspace_has_bound_projects` | Workspace still has bound projects. |
| `17` | `composition_error` | Workspace composition error. |
| `18` | `harness_not_supported` | Unsupported harness. |
| `19` | `harness_clash` | Harness configuration clash. |
| `20` | `plugin_not_found` | Plugin not found. |
| `21` | `plugin_already_in_state` | Plugin already in the requested state. |
| `22` | `plugin_manifest_parse_error` | Plugin manifest (`tome-plugin.toml`) parse error. |
| `23` | `skill_frontmatter_parse_error` | `SKILL.md` frontmatter parse error. |
| `24` | `summariser_failure` | Summariser failure. |
| `25` | `workspace_data_dir_write_failed` | Failed to write a workspace's data directory. |
| `26` | `prompt_argument_mismatch` | MCP prompt argument mismatch. |
| `27` | `entry_not_found` | Entry not found. |
| `28` | `substitution_failed` | Variable substitution failed. |
| `29` | `invalid_argument_frontmatter` | Invalid argument frontmatter. |
| `30` | `model_missing` | A required model is missing. |
| `31` | `model_corrupt` | A model file is corrupt. |
| `32` | `model_checksum_mismatch` | Model checksum mismatch. |
| `33` | `model_registration_parse_error` | Model registration parse error. |
| `34` | `inference_runtime_init_failure` | Inference runtime failed to initialise. |
| `35` | `vector_extension_init_failure` | Vector extension failed to initialise. |
| `36` | `embedding_generation_failure` | Embedding generation failed. |
| `37` | `reranking_failure` | Reranking failed. |
| `40` | `query_no_results_strict` | `--strict` query returned no results. |
| `41` | `embedder_name_drift` | Embedder name drift (index vs. configured model). |
| `42` | `embedder_version_drift` | Embedder version drift. |
| `43` | `hook_spec_parse_error` | Hook spec parse error. |
| `44` | `hook_settings_write_failed` | Failed to write hook settings. |
| `45` | `agent_translation_failed` | Agent translation failed. |
| `46` | `guardrails_write_failed` | Failed to write the guardrails file. |
| `47` | `reindex_scoped_embedder_change` | A scoped reindex was refused because the embedder changed — run a full `tome reindex`. |
| `50` | `index_busy` | The index is locked by another process. |
| `51` | `index_integrity_check_failure` | Index integrity check failed. |
| `52` | `schema_too_new` | Index schema is newer than this binary supports. |
| `53` | `catalog_has_enabled_plugins` | Catalog still has enabled plugins (use `--force`). |
| `54` | `not_a_terminal` | An interactive command was run without a terminal. |
| `60` | `mcp_startup` | MCP server failed to start. |
| `61` | `mcp_io` | MCP protocol I/O error. |
| `70` | `workspace_malformed` | Workspace data on disk is malformed. |
| `73` | `schema_too_new` | Workspace schema version too new. |
| `74` | `schema_migration` | Schema migration failed. |
| `75` | `doctor_fix_unsafe` | A `doctor --fix` repair was not safe to apply. |
| `80` | `plugin_not_converted` | Plugin not converted: legacy `.claude-plugin/plugin.json` exists but no `tome-plugin.toml`. |
| `81` | `output_exists` | Refusing to overwrite existing output (pass `--force`). |
| `82` | `template_invalid` | Template unusable (missing file, malformed template, render error). |
| `83` | `source_format_unrecognized` | Could not auto-detect source format (pass `--from <harness>`). |
| `84` | `conversion_unsupported_strict` | `convert --strict` hit an unsupported feature. |
| `85` | `validation_found_errors` | `lint` found at least one error. |
| `86` | `validation_strict_warnings` | `lint --strict` found warnings (and no errors). |
| `87` | `meta_skill_not_found` | Unknown bundled meta skill id. |
| `88` | `meta_install_failed` | Failed to install a meta skill. |
| `89` | `no_harness_detected` | No supported harness detected (use `--harness` or install one). |
| `90` | `telemetry_endpoint_unreachable` | Telemetry endpoint unreachable (vestigial — retained for the closed-set contract; not constructed today). |
| `91` | `telemetry_config_invalid` | Telemetry config invalid (vestigial — retained for the closed-set contract; not constructed today). |
| `92` | `telemetry_queue_corrupt` | Telemetry queue corrupt: unparsable lines were dropped (`tome telemetry inspect`). |
| `93` | `provider_config_invalid` | Provider config invalid: an undefined provider reference, a kind illegal for the capability, a `provider` set without a `model`, or a configured external provider with no resolvable credential (`tome models test`). |
| `94` | `provider_request_failed` | A remote provider request failed (auth, rate-limit, timeout, unreachable, malformed response). |
| `95` | `remote_embedding_invalid` | A remote embedding failed content validation (empty / non-finite / wrong dimension). |

## Authoring verdicts

Codes `85` and `86` are *verdicts*, not crashes: `lint` ran to completion and
is reporting what it found — `85` means at least one error, `86` means
warnings-only under `--strict`. Scripts and CI should branch on them (a `0`
means no findings; anything else in this pair is feedback, not a tool
failure). See [Linting](../authoring/lint.md).

## Health verdicts (`status` / `doctor`)

`tome status` and `tome doctor` render a report and then exit with one of three
**health verdicts** — the report always prints first, so these codes never
suppress the diagnosis:

| Verdict | Code | Meaning |
| --- | --- | --- |
| Healthy | `0` | Everything checks out. |
| Degraded | `10` | A non-fatal issue (e.g. the reranker or summariser is missing, or a catalog cache is broken) — queries still serve. |
| Unhealthy | `1` | A fatal issue (broken index, embedder drift, malformed config). |

Both non-zero verdicts fail a plain "fail on any non-zero" gate. The distinct
`10` lets a stricter gate **fail on unhealthy only**:

```sh
tome status; code=$?
if [ "$code" -eq 1 ]; then
  echo "unhealthy — failing the build"; exit 1
elif [ "$code" -eq 10 ]; then
  echo "degraded — warning only"; exit 0
fi
```

Equivalently, gate on the structured field — `status --json | jq -r .overall`
and `doctor --json | jq -r .overall` both yield `ok` / `degraded` / `unhealthy`,
which is the recommended, code-independent gating source.

For `tome doctor --fix`: when the repair runs but un-fixable issues remain, it
exits `75` (`doctor_fix_unsafe`) instead of the health verdict — "the fix did
something, but manual work is still required".
