# `tome mcp` — Command Contract

```
tome mcp [--workspace <path> | --global]
```

Spawns a long-lived stdio MCP server backed by the resolved scope's index. Designed to be launched by an MCP-compliant harness (Claude Code, Codex, Cursor, Gemini CLI, OpenCode, …) as a child process. Stdin and stdout carry the MCP protocol exclusively. Logs go to a file.

## Invocation example (Claude Code MCP config)

```json
{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp"]
    }
  }
}
```

For a workspace-scoped server, the harness passes `--workspace`:

```json
{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp", "--workspace", "/abs/path/to/project"]
    }
  }
}
```

The harness may also export `TOME_WORKSPACE=<path>` instead of passing the flag. Phase 3 honours both per [workspace-resolution.md](./workspace-resolution.md).

## Behaviour

1. Resolve scope (Phase 3 priority order — see [workspace-resolution.md](./workspace-resolution.md)).
2. Resolve `Paths` against the scope. Open the file log appender at `${XDG_STATE_HOME}/tome/mcp.log`, rotating any pre-existing log > 10 MiB to `.log.1`.
3. Run startup pre-flight (FR-110). In order:
   - Index DB file exists and is openable read-only.
   - `meta.schema_version` matches the running Tome's expected value (no migration in this path — the doctor / `--fix` is the migrator).
   - `meta.embedder_name` and `meta.embedder_version` match the installed `MODEL_REGISTRY` embedder identity.
   - Embedder model files exist and pass SHA-256 verification.
4. Eager-load the embedder model into the MCP server's state. (Reranker is deferred until the first `search_skills` call — FR-109.)
5. Register the two tools (`search_skills`, `get_skill`) on an `rmcp::ServerHandler`.
6. Take over stdin/stdout via `rmcp::transport::io::stdio()`; enter the server's main loop.
7. On SIGINT/SIGTERM, wait up to **5 s** for any in-flight tool call to complete, then exit (FR-112).

## Startup pre-flight failure modes

Each failure exits before binding the stdio transport. The harness sees the child process die immediately; the file log records the diagnosis; stderr also carries a single fatal line so the harness can surface it to the developer without the developer having to find the log file.

| Failure | Exit code | TomeError variant |
|---|---|---|
| Workspace marker malformed | 70 | `WorkspaceMalformed` |
| Workspace not found (`--workspace <path>` and no `.tome/` there) | 71 | `WorkspaceNotFound` |
| `--workspace` + `--global` both passed | 72 | `WorkspaceConflict` |
| Schema-too-new | 73 | `SchemaVersionTooNew` |
| Index DB missing | 30 | (existing) `ModelMissing`-like; surfaced as `McpStartupFailed { reason: "index_missing" }` (60) when no specific Phase 2 code applies |
| Embedder identity mismatch (drift) | 41 | (Phase 2 existing) |
| Embedder file missing | 30 | (Phase 2 existing) |
| Embedder checksum mismatch | 32 | (Phase 2 existing) |
| Index integrity check fails | 35 | (Phase 2 existing) |
| Other unexpected pre-flight error | 60 | `McpStartupFailed { reason }` |

## Output channels

| Channel | Carries |
|---|---|
| stdout | MCP protocol messages only. Phase 3 invariant FR-221: no human text, no JSON envelopes, no progress glyphs ever appear here. |
| stderr | Fatal startup errors only (one line, with the TomeError category and exit code). Silent in the steady state. FR-222. |
| `${XDG_STATE_HOME}/tome/mcp.log` | JSON-lines structured log (see [log-format.md](./log-format.md)). Levels filtered by `TOME_LOG` / `RUST_LOG` (default `info`). |

## Tools advertised

Exactly two — see [mcp-tools.md](./mcp-tools.md):
- `search_skills`
- `get_skill`

No prompts, no resources, no other tools, no notifications beyond what `rmcp` requires for the protocol handshake.

## Process model

- One server per agent session. The harness launches; the harness terminates.
- The server is fixed to the workspace it resolved at startup. Switching workspaces = restart.
- The server reads the index DB read-only via `OpenFlags::SQLITE_OPEN_READ_ONLY` (Phase 10 deferral now folded in per research R-13). It never takes the advisory lockfile.
- Concurrent CLI writers on the same workspace do not block the server. WAL semantics provide a consistent read view; a writer mid-transaction is not visible to the server until commit.

## Concurrency invariants

- Multiple `tome mcp` processes for the same workspace: allowed. Each opens a read-only handle. All see the same WAL state.
- Multiple `tome mcp` processes for different workspaces: allowed. Each is isolated to its workspace.
- A CLI writer (`tome plugin enable`, `tome reindex`, etc.) for the same workspace as a running MCP server: the writer acquires the workspace's advisory lock; the server does not contend. The next `search_skills` call after the writer commits sees the new state.

## Signal handling

`tokio::signal::ctrl_c()` and `tokio::signal::unix::signal(SIGTERM)` both trigger graceful shutdown:

1. Stop accepting new MCP requests.
2. Wait for any in-flight tool call to complete, with a 5 s timeout.
3. Drop the `tokio` runtime.
4. Exit 8 (`TomeError::Interrupted`, the Phase 1 contract for user-initiated termination — applies in MCP context too).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Never returned by `tome mcp` in normal operation (the server runs until signalled or stdin closes). |
| 8 | SIGINT / SIGTERM received; clean shutdown. |
| 60 | `McpStartupFailed` — composite pre-flight failure not covered by a more specific code. |
| 61 | `McpProtocolIo` — stdio transport error mid-session (e.g., harness closed stdin abruptly). |
| Phase 2 codes (30 / 32 / 35 / 41) | Pre-flight failure matching a specific Phase 2 case. The server exits with the specific code rather than 60. |
| 70-73 | Workspace and schema-version failures from §"Startup pre-flight failure modes". |

## Notes

- The protocol channel is sacred. Any code path that writes to stdout outside of `rmcp`'s transport adapter is a bug. A `tests/mcp_server.rs` test captures stdout for the full lifetime of a server invocation and asserts every line parses as a valid MCP message.
- Logging in this command path uses a `tracing` subscriber with a JSON layer pointed at the file appender. The CLI subscriber (stderr-based, used by every other Tome command) is *not* installed.
- The CLI dispatcher recognises `tome mcp` as a special command that intentionally does not honour `--json` at the top level (the protocol IS the structured output).
