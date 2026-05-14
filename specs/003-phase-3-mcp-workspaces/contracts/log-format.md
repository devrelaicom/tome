# MCP Log Format — Contract

The `tome mcp` server cannot use stderr or stdout for diagnostic logging (stdout is the MCP protocol channel; stderr is reserved for fatal startup errors only — FR-221, FR-222). All diagnostics go to a single file at `${XDG_STATE_HOME}/tome/mcp.log`.

## File format

JSON-lines (one record per line, terminated by `\n`). Schema per line:

```json
{
  "ts": "RFC3339 timestamp",
  "level": "trace|debug|info|warn|error",
  "target": "tome::module::path",
  "msg": "human-readable message",
  …                              // optional structured fields specific to the log event
}
```

Required fields: `ts`, `level`, `target`, `msg`. Every other field is event-specific structured context.

## Rotation policy

- File path: `${XDG_STATE_HOME}/tome/mcp.log`.
- Backup path: `${XDG_STATE_HOME}/tome/mcp.log.1`.
- Rotation trigger: **at server startup only**, if the existing `mcp.log` is `> 10 MiB`. Rotation = atomic rename `mcp.log → mcp.log.1` (overwriting any prior `.1`), then create a fresh `mcp.log`.
- Mid-process rotation: not implemented. A single MCP session writing more than 10 MiB of logs in one go is exceptional; the developer can investigate why.

Total on-disk footprint: bounded at ~20 MiB per machine.

## Filtering

`tracing-subscriber`'s `EnvFilter` reads the `TOME_LOG` or `RUST_LOG` environment variable. Default level: `info`. Examples:

```
TOME_LOG=debug                  # everything at debug or higher
TOME_LOG=tome::mcp=trace        # trace level for the MCP module only
TOME_LOG=tome::mcp::tools=debug,info  # debug for tools, info elsewhere
```

The filter applies to the file appender; stderr (used for fatal startup errors before the file is open) emits unconditionally.

## Event taxonomy

| Event | Level | Target | Required structured fields |
|---|---|---|---|
| MCP startup completed | `info` | `tome::mcp::server` | `scope` (`"global"\|"workspace"`), `workspace` (path or `null`), `embedder`, `reranker_lazy` |
| Pre-flight check failed | `error` | `tome::mcp::preflight` | `check` (e.g., `"db_present"`, `"embedder_checksum"`), `error` (scrubbed) |
| Tool call (`search_skills`) | `info` | `tome::mcp::tools::search_skills` | `query_len`, `top_k`, `filter` (struct: `{catalog?, plugin?}`), `matches`, `elapsed_ms` |
| Tool call (`get_skill`) | `info` | `tome::mcp::tools::get_skill` | `catalog`, `plugin`, `name`, `result` (`"ok"\|<error_code>`), `body_bytes`, `resource_count` |
| Tool error | `error` | `tome::mcp::tools::<tool>` | `error_code`, `error_message` (scrubbed) |
| Graceful shutdown | `info` | `tome::mcp::server` | `signal` (`"SIGINT"\|"SIGTERM"\|"stdin_closed"`), `in_flight` (count) |
| Hard shutdown | `error` | `tome::mcp::server` | `reason` |

## Sample log

```json
{"ts":"2026-05-14T12:34:55.823Z","level":"info","target":"tome::mcp::server","msg":"startup ok","scope":"workspace","workspace":"/home/user/projects/acme-app","embedder":"bge-small-en-v1.5","reranker_lazy":true}
{"ts":"2026-05-14T12:35:02.014Z","level":"info","target":"tome::mcp::tools::search_skills","msg":"call","query_len":52,"top_k":10,"filter":{},"matches":7,"elapsed_ms":214}
{"ts":"2026-05-14T12:35:02.215Z","level":"info","target":"tome::mcp::tools::get_skill","msg":"call","catalog":"acme-catalog","plugin":"writers","name":"blog-post-skeleton","result":"ok","body_bytes":3148,"resource_count":2}
{"ts":"2026-05-14T12:36:11.991Z","level":"error","target":"tome::mcp::tools::get_skill","msg":"call","catalog":"acme-catalog","plugin":"writers","name":"nonexistent","result":"unknown_skill","body_bytes":0,"resource_count":0}
{"ts":"2026-05-14T12:42:18.000Z","level":"info","target":"tome::mcp::server","msg":"graceful shutdown","signal":"SIGINT","in_flight":0}
```

## Scrubbing

Every value placed into the structured-fields layer passes through `git::scrub_credentials::scrub_to_string` before write. Specifically:

- `workspace` paths are scrubbed (the path is the user's; ordinarily inert, but if the path contains a credential-shaped segment like `~/.aws/credentials/something`, scrubbing is conservative).
- `error_message` strings flow through the scrubber (HTTP error chains may contain signed URLs).
- `query_len` is a length, not the query itself — queries are never logged at info level.

At debug or trace level, fields prefixed with `_unsafe_` (e.g., `_unsafe_query`) carry pre-scrub values for local debugging. These never appear at info or higher. The filter `TOME_LOG=tome::mcp::tools=trace` is the only way to surface them, and the developer must understand the trade-off (they're enabling content logging on their own machine).

## Privacy / safety

- The protocol channel never carries log output.
- Stderr never carries log output in the steady state.
- Workspace paths and catalog URLs scrub on write.
- Per-tool-call timing data (`elapsed_ms`) is captured at info level for ops support; no per-tool-call PII.

## Reader-friendly format option

The file is JSON-lines. To read interactively:

```sh
tail -F ~/.local/state/tome/mcp.log | jq 'select(.level=="error")'
tail -F ~/.local/state/tome/mcp.log | jq -c 'select(.target | startswith("tome::mcp::tools"))'
```

`tome doctor` does NOT read the log file — diagnostics there are user-facing observation, not Tome's own state. If a future phase needs Tome to report on log contents, it ships a dedicated `tome mcp log` subcommand.
