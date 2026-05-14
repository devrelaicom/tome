# MCP Config Integration — Contract

**Spec source**: [spec.md FR-500 through FR-505](../spec.md)

For each harness in the effective list, Tome writes one entry keyed `"tome"` into the harness's MCP configuration file. Read-modify-write preserves every other entry, comment, and key order Tome did not author.

## Entry shape

### JSON (Claude Code, Gemini, Cursor, OpenCode)

Top-level structure varies per harness:

- Claude Code: `{"mcpServers": { ... }}`
- Gemini: `{"mcpServers": { ... }}`
- Cursor: `{"mcpServers": { ... }}`
- OpenCode: `{"mcpServers": { ... }}`

Tome's entry inside `mcpServers`:

```json
{
  "tome": {
    "command": "tome",
    "args": ["mcp", "--workspace", "<workspace-name>"]
  }
}
```

### TOML (Codex)

```toml
[mcp_servers.tome]
command = "tome"
args = ["mcp", "--workspace", "<workspace-name>"]
```

## Ownership marker (FR-501)

An existing entry under key `"tome"` is **Tome-owned** if and only if:

- `command == "tome"`, AND
- `args[0] == "mcp"`.

Any other content under the `"tome"` key is **user-owned** and MUST NOT be overwritten without an explicit `--force` flag. On clash:

- Without `--force`: exit 19 (`HarnessClash`); error message quotes the existing `command` and `args[0]`.
- With `--force`: rewrite the entry; preserve any developer-added `env` field on the rewritten entry per FR-503.

## `env` field preservation

When Tome rewrites a Tome-owned entry (for example, on rebind from workspace A to workspace B, or when the bound workspace name changes), any developer-added `env` field MUST be preserved. The marker comparison ignores `env` entirely; the rewrite touches only `command` and `args`.

Example: developer has hand-added `env = { "MY_FEATURE_FLAG" = "1" }` to the Tome entry. After `tome workspace use <new-workspace>`, the entry's `args` change but `env` is unchanged.

## Read-modify-write discipline (FR-349)

Every read-modify-write of a harness MCP config follows this pattern:

1. **Read**: open and parse with the order-preserving library:
   - JSON: `serde_json::from_str` with the `preserve_order` feature enabled (which makes `serde_json::Value::Object` use `IndexMap` internally).
   - TOML: `toml_edit::Document::from_str`.
2. **Modify**: locate the `mcpServers.tome` (or `mcp_servers.tome`) node. Apply the modification (create / update / remove). Every other key in the document remains untouched.
3. **Write**: serialise back via the same library. The output preserves entry order and (for TOML) comments and whitespace.
4. **Atomic rename**: write to a sibling temp file on the same filesystem; fsync; atomic rename onto the original path.

**Lenient parse** (third-party input per the strictness boundary): unknown keys are NOT rejected. The strictness boundary established in Phase 2 applies only to Tome-owned inputs. Harness MCP configs are owned by the harness; Tome reads what the harness's configuration spec says, modifies its slot, and writes back.

## Missing parent directory

If the MCP config file's parent directory does not exist (e.g. a fresh project where `.claude/` has never been created), `sync` creates it using the same atomic-write discipline. The directory is created with mode 0700 on Unix.

If the MCP config file itself does not exist, `sync` creates it with the minimum scaffold:

- JSON: `{"mcpServers": {"tome": ...}}\n`
- TOML: `[mcp_servers.tome]\ncommand = "tome"\nargs = ["mcp", "--workspace", "<name>"]\n`

## Idempotence (FR-525 corollary)

A `tome harness sync` re-run with no state change MUST NOT rewrite the MCP config file. Comparison logic:

1. Parse the existing file.
2. Compute the expected entry shape (workspace name in `args`).
3. Compare the existing entry's `command` and `args` against the expected shape.
4. If equal, no write — return "leave-alone" for that harness.

The comparison treats `env` as opaque (not compared). The order of keys inside the entry's `args` array must be exactly `["mcp", "--workspace", "<name>"]` — any other order is a "drift" requiring rewrite.

## Remove (FR-504)

When a harness leaves the effective list:

1. Read the existing MCP config file.
2. Check the `mcpServers.tome` (or `mcp_servers.tome`) entry. If Tome-owned: remove the entry; serialise; atomic rename.
3. If user-owned: leave alone.
4. If the entry doesn't exist: leave alone (no syscall).

After removal, if the parent object (`mcpServers` or `mcp_servers`) is empty, leave the empty object in place. Other entries in the file are unaffected.

## Test coverage

- `tests/mcp_config_create.rs` — creating a new entry against an empty config, against an existing config with other entries, against a config with comments (TOML).
- `tests/mcp_config_update.rs` — workspace rebind rewrites `args`, preserves `env`.
- `tests/mcp_config_clash.rs` — user-owned `"tome"` entry causes exit 19; `--force` rewrites.
- `tests/mcp_config_remove.rs` — Tome-owned entry removed cleanly; user-owned entry left alone.
- `tests/mcp_config_preserve_order.rs` — three-entry JSON config: insert Tome in the middle position (alphabetical order would put it between two existing keys); verify order preservation.
