# Workspace Resolution — Contract

How every Tome invocation decides which `Scope` (`Global` or `Workspace(path)`) to operate against. This contract is honoured by every Phase 1, Phase 2, and Phase 3 command, including `tome mcp`.

## Resolution priority

In this order, **first match wins**:

| Rank | Source | Wins if |
|---|---|---|
| 1 | `--workspace <path>` CLI flag | The flag is present at the top level. |
| 2 | `--global` CLI flag | The flag is present at the top level. |
| 3 | `TOME_WORKSPACE` env var | Variable is set, non-empty, and resolves to an existing absolute path. |
| 4 | CWD walk | Walk from `current_dir()` toward `/` (or drive root on Windows); first parent directory containing a `.tome/` subdir wins. |
| 5 | Global fallback | No workspace found anywhere above. |

`--workspace` and `--global` are mutually exclusive; supplying both exits `72` (`WorkspaceConflict`). The CLI parser rejects this combination before subcommand dispatch.

## Global flags

```
--workspace <PATH>    use the workspace rooted at <PATH>
--global              use global state, ignoring any workspace context
```

Both are global flags accepted at the top level: `tome --workspace /foo plugin list` and `tome plugin list --workspace /foo` are equivalent. clap's `global = true` is honoured.

## CWD walk algorithm

```rust
let mut here = std::env::current_dir()?;
loop {
    let marker = here.join(".tome");
    match marker.try_exists() {
        Ok(true)  => return Ok(Some(here.canonicalize()?)),
        Ok(false) => { /* keep walking */ }
        Err(e)    => { tracing::debug!(?e, "workspace cwd walk encountered IO error"); break; }
    }
    if !here.pop() { break; }  // reached filesystem root
}
return Ok(None);
```

- Stops at the filesystem root. Does not walk into `/`.
- Non-`NotFound` `io::Error` does NOT propagate as a `TomeError`; the walk falls through to the global fallback and logs at debug level.
- `here.canonicalize()` resolves symlinks so the returned path is stable across re-resolutions.

## Environment variable

`TOME_WORKSPACE=<path>` must point at an existing directory containing a `.tome/` marker subdir. If the path doesn't exist or has no `.tome/`, resolution exits `71` (`WorkspaceNotFound`) rather than falling through — an explicit env var indicates user intent, and silent fall-through would mask configuration bugs.

## Validation

Once resolved:

1. If `Workspace(path)`:
   - `path/.tome/` must exist (verified during resolution).
   - `path/.tome/config.toml` must be readable as a valid `Config`. If unreadable or malformed → exit `70` (`WorkspaceMalformed { path, reason }`).
   - `path/.tome/index.db` may or may not exist. If it exists, it must open successfully (Phase 2 PRAGMA + integrity rules); failure → exit `70` with reason `"index database malformed"`.
2. If `Global`:
   - Normal Phase 1/2 global config behaviour.

## Debug logging

Every command emits one debug-level tracing line at startup recording the resolved scope and source:

```
DEBUG tome::workspace::resolution scope resolved scope=workspace path=/abs/path source=cwd_walk
DEBUG tome::workspace::resolution scope resolved scope=global source=global_fallback
```

This is invaluable when debugging "why is `tome catalog add` writing to the wrong config?"

## Examples

### Inside a workspace, no flags

```sh
$ cd /home/user/projects/acme-app    # contains .tome/
$ tome catalog list
# → resolves Workspace(/home/user/projects/acme-app) via cwd_walk
```

### Inside a workspace, with --global

```sh
$ cd /home/user/projects/acme-app
$ tome --global catalog list
# → resolves Global via global_flag
```

### Explicit workspace flag from elsewhere

```sh
$ pwd
/tmp
$ tome --workspace /home/user/projects/acme-app catalog list
# → resolves Workspace(/home/user/projects/acme-app) via flag
```

### Workspace env var (e.g., shell rc)

```sh
$ export TOME_WORKSPACE=/home/user/projects/acme-app
$ tome catalog list
# → resolves Workspace(/home/user/projects/acme-app) via env
```

### Conflict

```sh
$ tome --workspace /foo --global catalog list
error[72]: workspace conflict: --workspace and --global cannot be combined
```

### Env var points nowhere

```sh
$ TOME_WORKSPACE=/no/such/dir tome catalog list
error[71]: workspace not found: /no/such/dir does not contain a .tome/ marker
```

### Malformed workspace

```sh
$ cd /home/user/projects/broken-app    # contains .tome/ but config.toml is invalid TOML
$ tome catalog list
error[70]: workspace malformed: /home/user/projects/broken-app
  reason: invalid TOML in .tome/config.toml at line 4

Run `tome doctor` for a full diagnosis.
```

## MCP server interaction

`tome mcp` honours the same resolution rules. The MCP startup pre-flight runs resolution; failure modes 70 / 71 / 72 exit with their specific codes (not folded into the generic `McpStartupFailed` 60).

The harness can pin a workspace by either:
- `args: ["mcp", "--workspace", "/abs/path"]`, or
- `env: { "TOME_WORKSPACE": "/abs/path" }`.

The flag form is preferred when the harness already builds the args array; the env form is preferred when the harness lets the user set per-server environment variables in a config UI.

## Performance

Resolution cost is dominated by the CWD walk. Typical project depth ≤ 10; the walk performs ≤ 10 `stat(2)` calls in the worst case. On networked filesystems this could be slower, but the `try_exists` short-circuit on first `Ok(true)` keeps it bounded. No caching across invocations.
