# Exit Codes — Phase 4 Additions and Reused Variants

**Spec source**: [spec.md FR-600 through FR-603](../spec.md)

## New variants (codes 13–19, 24)

| Code | Variant | When raised | Auto-fixable by `doctor --fix`? |
|------|---------|-------------|----------------------------------|
| 13 | `WorkspaceNotFound { name }` | `--workspace <name>` whose name has no row in `workspaces`; `TOME_WORKSPACE` env var carrying a missing name; project marker config naming a missing workspace; composition reference `[workspaces.<name>]` to a missing workspace | No — requires explicit developer choice (rebind or recreate) |
| 14 | `WorkspaceAlreadyExists { name }` | `tome workspace add <name>` against an existing name | No |
| 15 | `WorkspaceNameInvalid { name, reason }` | The candidate name violates the FR-347 rule (charset, length, reserved on init, edge cases like leading `-`) | No |
| 16 | `WorkspaceHasBoundProjects { name, count, projects }` | `tome workspace remove <name>` without `--force` when ≥1 project is bound | No — rerun with `--force` |
| 17 | `CompositionError { kind }` | DFS cycle detected; `[workspace]` in workspace/global settings; non-plain-name `!`-prefix; (`UnknownWorkspace` sub-variant maps to code 13 instead — see Reused section) | No |
| 18 | `HarnessNotSupported { name }` | A `harnesses` array entry names a harness not in the supported five | No |
| 19 | `HarnessClash { path, command, first_arg }` | An existing `"tome"` MCP entry is user-owned (`command != "tome"` OR `args[0] != "mcp"`) and `--force` was not passed | No — rerun with `--force` |
| 24 | `SummariserFailure { kind }` | Summariser model missing at use time; model checksum mismatch at use time; `LlamaBackend::init()` failure; model output empty or unparsable | Yes for `ModelMissing` (re-download); No for others |

**Note on code 24 vs the original 20**: The data-model.md and earlier drafts of this contract specced `SummariserFailure` at code 20. Code 20 is already owned by Phase 2's `PluginNotFound` (closed-set rule II — pairwise-unique codes). F3 placed `SummariserFailure` at code 24 (next free slot above the Phase 2 plugin-lifecycle range 20–23). The contract is now reconciled to match the implementation.

## Reused variants

These Phase 4 failure modes do NOT introduce new variants; they map to existing Phase 1/2/3 codes:

| Failure mode | Reused variant | Code | Rationale |
|--------------|----------------|------|-----------|
| Project marker config malformed | `WorkspaceMalformed` | 70 | Same semantic class as Phase 3's marker-malformed case; widened to cover Phase 4's binding-pointer model |
| Workspace rename precondition: bound project dir missing on disk | `WorkspaceMalformed` | 70 | Central registry's recorded binding is malformed relative to the filesystem |
| Per-user state dir unwritable | `Io` | 7 | Standard filesystem permission error |
| Composition reference to non-existent workspace | `WorkspaceNotFound` | 13 | Same semantic class as direct workspace-not-found |
| Bind step succeeds but harness-sync fails | Whichever code from the failing harness-sync surface | varies | Binding remains committed; doctor reports the drift |

## Closed-set invariant

Phase 4 maintains the Phase 1/2/3 closed-error-set principle. No "Other"/"Unknown" variant exists. Every Phase 4 code path emits either:

1. A new variant from the codes 13–19 / 24 table above, OR
2. A reused variant from the table above, OR
3. A Phase 1/2/3 variant (unchanged behaviour)

A structural test (extending `tests/exit_codes.rs`) asserts every variant maps to a documented code; no `0` or `1`-coded variants exist outside `Success` and `Internal`.

## Error message style

- Begin with the failure class in lowercase ("workspace not found", "summariser failure"), then the specific identifier.
- Include the failing identifier as a quoted backtick where possible: `workspace \`my-project\` not found in the central registry`.
- For composition errors, quote the chain of references that triggered the cycle: `composition cycle: project → [workspaces.shared] → [workspaces.shared] (self-reference)`.
- For summariser failures, name the failure sub-class: `summariser failure: model output empty (long summary)`.
- Where the doctor can repair, suggest the exact fix command in the error tail: `... rerun with \`tome doctor --fix\``.
