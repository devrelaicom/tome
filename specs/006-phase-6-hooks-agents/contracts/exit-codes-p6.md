# Phase 6 — Exit codes

Authoritative numeric assignments for Phase 6 failure classes. Pin these here; do not re-litigate in implementation. Every Phase 6 `TomeError` variant maps to exactly one of these codes via `From<TomeError> for ExitCode` in `src/error.rs`.

## Scope

Phase 6 claims four new codes for its closed-enum additions, all in one free contiguous run. The PRD's first draft proposed **30–33**; those collide with the existing model-on-disk cluster (`ModelMissing` 30, `ModelCorrupt` 31, `ModelChecksumMismatch` 32, `ModelRegistrationParseError` 33), and **34–37** are also taken (the inference/vector cluster). Per research R-1, Phase 6 reassigns to **43–46** — the same precedent as Phase 4's summariser code (proposed 20 → shipped 24) and the Phase 5 cluster (proposed 21–23 → shipped 25–29).

## Occupied-code set (why 43–46)

Before Phase 6 the closed enum occupies:

```
1–9, 13–37, 40–42, 50–54, 60–61, 70, 73–75
```

The first free contiguous run large enough for four pairwise-unique codes is **43–49**. Phase 6 claims **43–46** and leaves **47–49** free for future use. 38–39 (only two slots) is insufficient; 55–59 / 62–69 sit further from the existing clusters with no advantage. Reusing 30–33 is rejected outright — it would silently change shipped exit-code meanings, violating constitution principle II (NON-NEGOTIABLE: pairwise-unique, frozen-once-shipped exit codes; no `Other`/`Unknown` arm).

## Reassigned slots (contract amendment)

The PRD § "Exit codes (additions)" proposed:

| PRD proposal | Final assignment |
|---|---|
| 30 (malformed `hooks.json`) | 43 |
| 31 (hook settings-file write failed) | 44 |
| 32 (agent frontmatter / translation failed) | 45 |
| 33 (guardrails render/write failed) | 46 |

All references below use the final assignment. After this amendment, Phase 6 occupies a clean contiguous cluster at **43–46**.

## Assignments

Variant names are illustrative (data-model §8); final names land in F1.

| Code | Variant | Class | Trigger | FR |
|---|---|---|---|---|
| 43 | `HookSpecParseError { path }` | Parse | A plugin's `hooks/hooks.json` is malformed or unparsable. Sibling components of the same plugin still reconcile where possible (loud-but-isolated parse failure). | FR-092 |
| 44 | `HookSettingsWriteFailed { path, source }` | I/O | Read, merge, or write failure on the project's local Claude settings file (`.claude/settings.local.json`). The committed `.claude/settings.json` is never written. | FR-002, FR-092 |
| 45 | `AgentTranslationFailed { agent }` | Parse / translation | An agent's frontmatter is malformed YAML, or translation fails for a target harness. The agent is never partially emitted (FR-084 atomicity). | FR-030, FR-092 |
| 46 | `GuardrailsWriteFailed { path }` | I/O / render | A guardrails region render or write fails for a rules file or the Cursor standalone sibling. Partially-written guardrails state is never left between markers (FR-084 atomicity). | FR-011, FR-092 |

## Reused codes

Where a Phase 6 failure mode maps cleanly onto an existing closed-set variant, Phase 6 re-emits it rather than promoting a new code (closed-enum discipline, constitution §II):

| Code | Existing variant | Phase 6 site |
|---|---|---|
| 2 | clap usage error | New `--json` flags / settings deserialisation; agent-emission CLI surface argument errors |
| 7 | `Io` | Generic filesystem reads/writes outside the four dedicated sinks (e.g. reading a plugin's `agents/*.md` source at *index* time — during `harness sync`, agent source/parse failures surface as **45**, and a `GUARDRAILS.md` source read failure as **46** per the B-1 marker/symlink decision); atomic-write IO that is not the local Claude settings file (→ 44) or a guardrails target (→ 46) |
| 13 | `WorkspaceNotFound` | Persona toggle resolved against an unknown startup scope; sync against an unknown workspace (carryover) |
| 70 | `WorkspaceMalformed` | Parse failure on Tome-owned settings (the two new `bool` fields), distinct from third-party plugin inputs which surface as 43/45 |
| 73 | `SchemaVersionTooNew` | Read-only DB open against a future schema (carryover) |
| 74 | `SchemaMigrationFailed` | The marker-only `kind`-domain migration failing (entry-schema-p6.md) |

`NotATerminal` (54) is unchanged; Phase 6 adds no new interactive prompts (FR-080), so it is not newly triggered.

## Code slugs (for MCP `data.code` field)

MCP tool / prompts handlers map these to MCP error envelopes per the Phase 3 pattern (`ErrorData.data.code` = the slug below; numeric JSON-RPC code is generic `INVALID_PARAMS` / `INTERNAL_ERROR`).

| Code | Slug |
|---|---|
| 43 | `hook_spec_parse_error` |
| 44 | `hook_settings_write_failed` |
| 45 | `agent_translation_failed` |
| 46 | `guardrails_write_failed` |

## JSON envelope

CLI commands emit failures in the existing JSON envelope on `--json` (shape unchanged from Phase 5):

```json
{
  "ok": false,
  "exit_code": 45,
  "error": {
    "code": "agent_translation_failed",
    "message": "agent translation failed: midnight-expert/compact-dev/reviewer",
    "data": { "agent": "midnight-expert/compact-dev/reviewer" }
  }
}
```

## Discipline

When implementing, the `From<TomeError> for ExitCode` mapping must:

1. Cover every Phase 6 variant exhaustively (no `_ => ExitCode(1)` fallback for the new variants).
2. Be tested via `tests/exit_codes.rs` (extended with four new assertion entries — one per Phase 6 code).
3. Be reflected in `tests/exit_codes_e2e.rs` (extended with CLI-binary coverage where the code is reachable via the binary — 43 and 45 are triggerable through plugin enable / sync against a malformed fixture; 44 and 46 are IO failures that may stay library-API-only per the established split if they cannot be cheaply forced through the binary).
