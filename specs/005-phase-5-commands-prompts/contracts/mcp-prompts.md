# Phase 5 — MCP prompts capability

Authoritative contract for the new MCP `prompts` capability — Tome's user-invocation surface. Per FR-060–FR-072.

## Capability declaration

At MCP server initialization:

```rust
ServerInfo::new(
    ServerCapabilities {
        tools:   Some(ToolsCapability { list_changed: Some(false) }),
        prompts: Some(PromptsCapability { list_changed: Some(false) }),
        ..Default::default()
    }
)
```

Per NFR-008: `list_changed: false` is the Phase 5 commitment. Workspace switches that change the prompt set require server restart (per-session model).

Capability declared whenever Tome's MCP server starts, regardless of whether the workspace has any user-invocable entries (capability presence is independent of prompt set size).

## Methods

### `prompts/list`

Returns all user-invocable entries from the active workspace's enabled-entries set as MCP prompts.

#### Request

```json
{ "method": "prompts/list", "params": { /* optional cursor for pagination — Phase 5 does NOT paginate */ } }
```

#### Response

```json
{
  "prompts": [
    {
      "name": "midnight_expert__compact_circuits",
      "description": "<entry description, possibly truncated to a fixed cap>",
      "arguments": [
        { "name": "issue", "required": true }
      ]
    },
    {
      "name": "midnight_expert__fix_issue",
      "description": "Fix a GitHub issue",
      "arguments": [
        { "name": "args", "description": "<argument-hint OR default>", "required": false }
      ]
    }
  ]
}
```

#### Behaviour

- DB query: `SELECT * FROM skills WHERE user_invocable = 1 AND enabled = 1` against the active workspace's enrolment.
- For each row, derive the prompt name + argument schema (see below).
- Sort the result by prompt name (alphabetical) for stable output across calls.

#### Description cap (per FR-066)

The `description` field on each prompt in the list is truncated to **300 characters** per FR-066. Rationale for the 300-vs-150 split with `search_skills`:

- `search_skills` results are **agent-consumed** (machine-read by the LLM through the tool-call interface); verbose descriptions waste agent context tokens. FR-092 pins 150 chars there.
- `prompts/list` results are **user-consumed** (rendered in the harness slash menu); slightly longer descriptions help users pick the right command without expanding the entry. 300 chars fits comfortably in typical slash-menu UI without overflowing.

Truncation rule (mirrors FR-092): truncate at the Unicode-scalar-value boundary and append `…` (U+2026). Plugin authors who want a shorter prompt description can set a short `description` explicitly; the cap is an upper bound, not a target.

Claude Code itself truncates `description + when_to_use` at 1,536 chars in its own skill listing — this is a Claude Code-specific UI choice. Tome's 300-char cap is independent of (and stricter than) that.

### `prompts/get`

Returns the rendered body of a named prompt as a single user-role message.

#### Request

```json
{
  "method": "prompts/get",
  "params": {
    "name": "midnight_expert__fix_issue",
    "arguments": {
      "args": "123"
    }
  }
}
```

The `arguments` parameter is an object mapping argument name to string value. For prompts declaring named arguments, keys match the declared names. For prompts with the catch-all `args` argument, key is `"args"`.

#### Response

```json
{
  "messages": [
    {
      "role": "user",
      "content": {
        "type": "text",
        "text": "<rendered body after substitution pipeline>"
      }
    }
  ]
}
```

#### Behaviour

- Resolve `name` to the corresponding `EntryRow` via `PromptRegistry.by_name`.
- Build `SubstitutionContext` for the entry.
- Set `context.args = Some(ArgumentValues::Object { ... })` from the request's `arguments` payload.
- Run `substitution::render(body, &context)`.
- Return rendered body wrapped in the single-message envelope.

The `role` is always `"user"` per the PRD ("slash invocation injects the rendered content as if the user typed it"). Phase 5 does not return assistant-role or system-role messages.

#### Error responses

| Trigger | JSON-RPC code | data.code |
|---|---|---|
| Prompt name unknown | METHOD_NOT_FOUND | `prompt_not_found` |
| Argument count exceeds declared count | INVALID_PARAMS | `prompt_argument_mismatch` |
| Named arg in request doesn't match any declared name | INVALID_PARAMS | `prompt_argument_mismatch` |
| Substitution failure | INTERNAL_ERROR | `substitution_failed` |
| Data directory creation failure | INTERNAL_ERROR | `workspace_data_dir_write_failed` |

## Prompt name derivation

### Format

```
<plugin>__<entry-name>
```

Tome contributes ONLY this. The harness prepends its own `mcp__<server>__` per Claude Code's `/mcp__servername__promptname` convention.

### Sanitisation

Lowercase → replace non-`[a-z0-9_-]` with `_` → collapse runs of `_` → truncate per portion.

```rust
fn sanitise(input: &str) -> String {
    let s: String = input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let mut prev = '_';
    s.chars().filter_map(|c| {
        let keep = !(c == '_' && prev == '_');
        prev = c;
        keep.then_some(c)
    }).collect()
}
```

Per-portion length caps:
- Plugin portion: 16 characters (NFR-003).
- Entry-name portion: 32 characters.
- Combined: `<≤16>__<≤32>` ≤ 50 characters before the harness adds its `mcp__tome__` prefix. After harness prefix: ≤ 50 + 11 = 61 characters. Well under MCP's documented prompt-name budget (no published exact limit; comfortable safety margin).

Truncation logged at `debug!` level. No warning needed.

### `prompt_name` override

If the entry's frontmatter sets `prompt_name`, the override replaces BOTH portions of the generated name with a single user-chosen string. The override is sanitised + truncated (combined cap of 48 characters — the budget that would otherwise hold both portions plus the `__` separator).

Example: `prompt_name: review-my-pr` →
- Sanitised: `review-my-pr`
- Truncated (well under 48 chars): `review-my-pr`
- Final Tome contribution: `review-my-pr`
- Harness prepends: `/mcp__tome__review-my-pr`

If `prompt_name` is set, the `__` separator is NOT injected — the override IS the entire Tome contribution.

## Collision handling

When multiple entries (after sanitisation + truncation) derive identical prompt names:

### Detection

- Run name derivation against the full enabled-entry set at MCP startup.
- Build a `HashMap<derived_name, Vec<EntryRow>>`; any bucket with > 1 entries is a collision.

### Resolution

Per FR-062:

1. Sort the colliding entries by:
   1. `indexed_at` ASC.
   2. Lex order on tuple `(catalog, plugin, kind, name)` for ties.
2. First entry gets the **unsuffixed** name.
3. Subsequent entries get `<name>2`, `<name>3`, `<name>4`, etc. — counter starts at **2**, increments by 1.

The counter-suffix increments AT the entry-name portion length cap — but the suffix itself uses 1-2 additional characters beyond the cap. The combined `<plugin>__<entry>N` may briefly exceed the per-portion cap; that's acceptable because collisions are uncommon and the harness-side budget has comfortable margin.

### Diagnostics

Each collision is logged at `warn!` level:

```
collision_resolved derived_name="midnight_expert__fix_issue" entries=[
  { catalog: "midnight-expert", plugin: "compact-dev", kind: "command", name: "fix-issue", indexed_at: "2026-05-26T10:00:00Z", final_name: "midnight_expert__fix_issue" },
  { catalog: "midnight-expert", plugin: "compact-cli-dev", kind: "command", name: "fix-issue", indexed_at: "2026-05-26T10:30:00Z", final_name: "midnight_expert__fix_issue2" }
]
```

The doctor surface (FR-121) reads `PromptRegistry.collisions` to surface this to users.

## `PromptRegistry`

Built at MCP startup; immutable for the session.

```rust
pub struct PromptRegistry {
    pub by_name: HashMap<String, EntryRow>,   // resolved-name → entry
    pub collisions: Vec<CollisionRecord>,     // surfaced via doctor
}
```

Construction:
1. Query enabled-and-user-invocable entries for the active workspace.
2. Derive prompt name for each (applying `prompt_name` override if present).
3. Group by derived name; for any bucket > 1, resolve collisions.
4. Final `by_name` keyed by post-collision names.

Lookup at `prompts/get` time: `O(1)` via `HashMap.get(&name)`.

## Argument schema derivation (FR-070–FR-072)

### Case A: entry declares named arguments

`arguments: [component, from, to]` →

```json
{
  "name": "<derived prompt name>",
  "description": "<truncated>",
  "arguments": [
    { "name": "component", "required": true },
    { "name": "from",      "required": true },
    { "name": "to",        "required": true }
  ]
}
```

All declared arguments are required strings. Order matches declaration order.

### Case B: entry declares no named arguments

```json
{
  "name": "<derived prompt name>",
  "description": "<truncated>",
  "arguments": [
    {
      "name": "args",
      "description": "<argument-hint OR documented generic default>",
      "required": false
    }
  ]
}
```

The catch-all argument is named `args` (literal string per FR-071). Its description:
- If `argument-hint` frontmatter is set: use its value verbatim.
- Else: a documented generic description string — for Phase 5: `"Optional free-form input passed to the entry as a single positional argument."`

`required: false` because:
- Many commands work without args.
- The append-fallback footer ensures supplied args reach the agent even without body references.

## Workspace lifecycle interaction

- `tome plugin enable` / `disable`: changes the set of user-invocable entries for affected workspaces. MCP server must be restarted to pick up changes (NFR-008).
- `tome workspace use <name>`: rebinds the project; MCP server restart picks up the new workspace's prompt set.
- `tome reindex`: re-evaluates content hashes; does NOT change which entries are user-invocable (frontmatter-driven, not content-hash-driven).
- `tome catalog update`: same as reindex — frontmatter changes from upstream may flip `user_invocable`; MCP restart picks them up.

## Backwards compatibility

Phase 4 Tome did NOT advertise the `prompts` capability. Phase 5 Tome advertises it unconditionally. Harnesses that don't support MCP prompts (e.g. Codex per https://code.claude.com — pending upstream support) simply ignore the capability; the agent surface (search + read tools) remains fully functional. SC-006 pins this.

## Tests

| Behaviour | Test |
|---|---|
| `prompts/list` returns all user-invocable entries | `tests/mcp_prompts.rs::list_returns_user_invocable_entries` |
| `prompts/list` excludes entries with `user_invocable = 0` | `tests/mcp_prompts.rs::list_excludes_non_invocable` |
| `prompts/list` includes both kinds when applicable | `tests/mcp_prompts.rs::list_includes_both_kinds` |
| Argument schema derivation: named args | `tests/mcp_prompts.rs::named_args_become_required_string_array` |
| Argument schema derivation: catch-all `args` | `tests/mcp_prompts.rs::no_args_becomes_optional_catchall` |
| `prompts/get` runs substitution layer | `tests/mcp_prompts.rs::get_invokes_substitution` |
| `prompts/get` with structured args | `tests/mcp_prompts.rs::get_with_structured_args` |
| `prompts/get` with single string arg | `tests/mcp_prompts.rs::get_with_single_string_arg` |
| Prompt name sanitisation + truncation | `tests/prompt_naming.rs::*` |
| `prompt_name` override replaces both portions | `tests/prompt_naming.rs::override_replaces_both_portions` |
| Collision counter suffixing | `tests/prompt_collision.rs::counter_suffix_starts_at_2` |
| Collision tie-breaking on lex order | `tests/prompt_collision.rs::ties_break_lex_catalog_plugin_kind_name` |
| Collision logged at warn level | `tests/prompt_collision.rs::collision_logged_warn` |
| Doctor surfaces collisions | `tests/doctor_p5.rs::collisions_appear_in_doctor_report` |
| Byte-stable JSON wire pin (prompts/list response) | `tests/mcp_prompts_list_json_shape.rs` |
| Byte-stable JSON wire pin (prompts/get response) | `tests/mcp_prompts_get_json_shape.rs` |
