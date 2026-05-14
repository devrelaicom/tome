# Sync Algorithm — Contract

**Spec source**: [spec.md FR-540 through FR-547](../spec.md)

Runs from a project directory. Computes the effective harness list and reconciles the project's actual integration state on disk to match. Filesystem-derived state — no sidecar.

## Inputs

- `project_root: PathBuf` — canonicalised CWD or the result of the project-marker walk.
- `paths: &Paths` — the central root layout.
- `central_db: &Connection` — for resolving the bound workspace's identity and settings.

## Algorithm

```text
sync(project_root, paths, central_db, force):
    1. Read <project_root>/.tome/config.toml → ProjectMarkerConfig. Error 70 if malformed.
    2. Look up bound workspace: SELECT id, name FROM workspaces JOIN workspace_projects
       ON workspace_projects.workspace_id = workspaces.id
       WHERE workspace_projects.project_path = canonical(project_root).
       Error 13 if no row (orphan binding).
    3. Read <root>/workspaces/<name>/settings.toml → WorkspaceSettings.
    4. Read <root>/settings.toml → GlobalSettings.
    5. effective = resolve_effective_list(project_marker, workspace_settings,
                                          global_settings, central_db)
       Errors propagate: 17 (composition), 13 (workspace ref unresolvable), 18 (unsupported).
    6. all_rules_targets = { harness.rules_file_target(project_root) for harness in supported_harnesses }
    7. live_rules_targets = { harness.rules_file_target(project_root) for harness in effective.harnesses }
    8. For each path in all_rules_targets:
         is_live = path in live_rules_targets
         existing_tome_owned = read path; check for Tome block / standalone-file marker
         if is_live and not existing_tome_owned:
           CREATE: write Tome block (with @-include or inline per the harness's block_body_style)
                   OR write standalone Tome-owned file
         if is_live and existing_tome_owned and content_differs:
           UPDATE: rewrite the block / standalone file
         if is_live and existing_tome_owned and content_matches:
           LEAVE-ALONE (no syscall; idempotence guarantee)
         if not is_live and existing_tome_owned:
           REMOVE: delete the block (or standalone file)
    9. For each harness in effective.harnesses:
         mcp_path = harness.mcp_config_path(project_root, home)
         existing = read mcp_path under the lockfile; lenient parse via toml_edit / serde_json:preserve_order
         if existing has key "tome":
           if Tome-owned (command=="tome" && args[0]=="mcp"):
             if args match (workspace name correct, scaffold present): LEAVE-ALONE
             else: UPDATE the entry, preserving env
           else (user-owned):
             if force: rewrite (lossy on the entry; preserves env from the original)
             else: error 19 (HarnessClash); bubble; partial sync (other harnesses still processed)
         else:
           CREATE the entry; preserve every other key Tome did not author
    10. For each supported harness NOT in effective.harnesses:
         mcp_path = harness.mcp_config_path(project_root, home)
         existing = read mcp_path
         if existing has key "tome" and Tome-owned:
           REMOVE the entry
         else:
           LEAVE-ALONE
    11. Emit summary: { rules_changes: [...], mcp_changes: [...], leave_alones: count }.
```

## Idempotence (FR-525)

Re-running `sync` on a project whose state already matches the effective list MUST produce zero filesystem writes:

- Rules-file blocks: compare the existing block byte-for-byte against what the algorithm would write. If equal, no rewrite. The file's `mtime` does not advance.
- Standalone files: same byte-for-byte compare against the full content.
- MCP entries: parse the existing config, compare the relevant entry's `command`/`args` against the expected shape (ignoring `env`). If equal, no rewrite.

The test in `tests/sync_idempotence.rs` runs `sync` twice, asserts the second run's filesystem-change count is zero, and verifies via `stat(2)` that no output file's `mtime` advanced between runs.

## Partial sync semantics

If a single harness's sync step fails (e.g. exit 19 on user-owned MCP entry), other harnesses' steps continue. The command's overall exit code is the first non-zero step's code. The structured output's `changes` array reports successful steps and the `errors` array reports failed ones, so the developer sees both.

## Per-harness scope of writes

Per [paths-and-layout-p4.md](./paths-and-layout-p4.md) and the harness modules:

- Project-scope MCP configs (Claude Code, Cursor, OpenCode): writes inside `<project_root>/`.
- Global MCP configs (Codex, Gemini): writes inside `<home>/`.
- Project-scope rules files: writes inside `<project_root>/`.
- Standalone rules files: writes inside `<project_root>/.cursor/rules/` (Cursor only).

A `tome harness sync` invocation may therefore touch files outside `<project_root>/` (specifically, in `<home>/`). The structured output names every path written for the developer's auditing.

## Concurrency

The sync algorithm has two distinct phases with different concurrency profiles:

**Phase A — DB read (under the lock, brief)**: acquire the central DB's advisory lockfile only long enough to read `workspace_projects`, the bound workspace's `WorkspaceSettings`, the global `GlobalSettings`, and to compute the effective harness list. Release the lock before phase B. Typical duration: a few milliseconds.

**Phase B — Filesystem I/O (NO lock held)**: steps 8, 9, 10 (rules-file writes, MCP config read-modify-write, integration removal) run with the lockfile released. These steps touch harness files OUTSIDE `<home>/.tome/` — a slow filesystem in a project directory MUST NOT block every other Tome command on the machine. The cost of releasing the lock: a concurrent `harness use` could mutate the effective list mid-sync; phase B's filesystem operations are individually atomic (atomic rename on each file) so the worst observable outcome is a slightly-stale effective list applied — corrected by the next sync. This matches the Phase 3 reader/writer model where readers don't block on the lock.

Two parallel `sync` invocations from two terminals: each runs its own phase A → phase B independently. The terminal that writes last wins on any specific file. Re-running `sync` produces zero filesystem writes (the byte-for-byte idempotence guarantee FR-525).

A `sync` and a `harness use` running concurrently: `harness use` acquires the lock for its settings-file write (phase-A-style brief lock); `sync`'s phase B doesn't contend. The two operations interleave benignly.

A `sync` and a `plugin enable` running concurrently: `plugin enable` holds the lock during its DB transaction + summariser invocation (the longer hold). `sync`'s phase A waits behind it. `sync`'s phase B then runs unlocked. This is the dominant contention case in practice; the 30-second summariser timeout (NFR-106) bounds the worst-case wait.

## Multi-harness same-rules-file

When two harnesses in the effective list target the same rules-file path (e.g. `claude-code` and `codex` both targeting `AGENTS.md`): step 8 runs once for that path (the iteration is over the set, deduplicated). Step 9 runs once per harness (each harness has its own MCP config path). Removing one of the two harnesses leaves the rules block intact (the other harness still targets it) but removes the second harness's MCP entry — per FR-483.
