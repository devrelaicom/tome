# Phase 5 — MCP tools

Authoritative contract for the three Phase 5 MCP tool surfaces: updated `search_skills`, new `get_skill_info`, updated `get_skill`. Phase 3's two-tool model becomes three-tool. Per FR-080 et seq.

## Tool list at runtime

When the MCP server starts in a workspace with at least one enabled plugin, the `tools` capability lists exactly three tools:

1. `search_skills` — semantic search, KNN + reranker, cheap top-k.
2. `get_skill_info` — middle-tier metadata + resource enumeration; **NEW in Phase 5**.
3. `get_skill` — full entry body with substitution applied; existing in Phase 3, extended in Phase 5.

A workspace with zero enabled plugins still advertises all three; calls return `entry_not_found` (exit 27) for any catalog/plugin/name.

## `search_skills` — updated

### Input schema

```json
{
  "type": "object",
  "properties": {
    "query":                  { "type": "string" },
    "top_k":                  { "type": "integer", "default": 10 },
    "catalog":                { "type": "string" },
    "plugin":                 { "type": "string" },
    "description_max_chars":  { "type": "integer", "default": 150 }
  },
  "required": ["query"]
}
```

### Behaviour

- `WHERE searchable = 1 AND enabled = 1` filter applied to the candidate set (FR-090). Entries with `disable-model-invocation: true` are excluded.
- Both kinds (`skill` and `command`) returned in the same ranked result set (FR-090).
- KNN + optional reranker logic unchanged from Phase 3.
- The query string is capped at `MAX_QUERY_CHARS` (existing 4096-char cap from Phase 4 US5.a).

### Result element

```json
{
  "catalog": "midnight-expert",
  "plugin":  "compact-dev",
  "name":    "compact-circuits",
  "kind":    "skill",
  "description": "<truncated to description_max_chars, suffix '…' if truncated>",
  "path":    "/abs/path/to/SKILL.md",
  "score":   0.87
}
```

Truncation rule (FR-092): if the description's character count (Unicode scalar values, NOT bytes) exceeds `description_max_chars`, truncate at the boundary AND append the ellipsis character `…` (U+2026). The total post-truncation string is exactly `description_max_chars` + 1 characters (`description_max_chars` content chars + `…`). Untruncated descriptions are returned as-is.

### Error responses

| Trigger | Code | Slug |
|---|---|---|
| `query` exceeds `MAX_QUERY_CHARS` | INVALID_PARAMS | `query_too_long` |
| `top_k` < 1 or > 100 | INVALID_PARAMS | `invalid_top_k` |
| `description_max_chars` < 0 | INVALID_PARAMS | `invalid_description_max_chars` |
| Workspace has no enabled plugins | OK (returns empty array) | — |

## `get_skill_info` — NEW

### Input schema

```json
{
  "type": "object",
  "properties": {
    "catalog": { "type": "string" },
    "plugin":  { "type": "string" },
    "name":    { "type": "string" },
    "kind":    { "type": "string", "enum": ["skill", "command"], "default": "skill" }
  },
  "required": ["catalog", "plugin", "name"]
}
```

### Output (skill-kind)

```json
{
  "catalog": "midnight-expert",
  "plugin": "compact-dev",
  "name": "compact-circuits",
  "kind": "skill",
  "path": "/abs/path/to/SKILL.md",
  "description": "<full, untruncated>",
  "when_to_use": "<full when_to_use frontmatter content or null>",
  "plugin_version": "1.4.0",
  "user_invocable": false,
  "resources": {
    "files": [
      "/abs/path/to/skill/config.json"
    ],
    "directories": {
      "examples": [
        "/abs/path/to/skill/examples/basic.ts",
        "/abs/path/to/skill/examples/advanced.ts"
      ],
      "references": [
        "/abs/path/to/skill/references/api-spec.md",
        "/abs/path/to/skill/references/glossary.md"
      ],
      "scripts": [
        "/abs/path/to/skill/scripts/audit.py",
        "/abs/path/to/skill/scripts/lint.py",
        "/abs/path/to/skill/scripts/build.sh",
        "/abs/path/to/skill/scripts/deploy.sh",
        "/abs/path/to/skill/scripts/test.sh",
        "and 3 more"
      ]
    }
  }
}
```

### Output (command-kind)

Same shape minus the `resources` key entirely (FR-083). Example:

```json
{
  "catalog": "midnight-expert",
  "plugin": "compact-dev",
  "name": "fix-issue",
  "kind": "command",
  "path": "/abs/path/to/commands/fix-issue.md",
  "description": "<full>",
  "when_to_use": null,
  "plugin_version": "1.4.0",
  "user_invocable": true
}
```

### Resource enumeration rules

- `files`: top-level files in the entry's directory (parent of `path`), excluding the entry file itself, sorted alphabetically. Capped at 5; over-cap files list the first 5 + sentinel `"and N more"`.
- `directories`: top-level subdirectories of the entry's directory, sorted alphabetically by name. Each value is the array of immediate children (NOT recursed). Children sorted alphabetically by basename, absolute paths returned. Capped at 5 per subdirectory; over-cap entries get the same sentinel.
- The `directories` map's KEY ordering is preserved alphabetically via `BTreeMap` (serialised as JSON object with key order matching `BTreeMap` iteration).
- Sentinel format: literal string `"and {count} more"` where `{count}` is the number of OMITTED entries. NOT `"and {count} more entries"` — match the PRD's documented form.
- Hidden files (starting with `.`) and directories are included if present (no filtering).

### Error responses

| Trigger | Code | Slug |
|---|---|---|
| Entry not found in current workspace's enabled set | INVALID_PARAMS | `entry_not_found` |
| Catalog or plugin name unknown | INVALID_PARAMS | `entry_not_found` |
| `kind` parameter not `"skill"` or `"command"` | INVALID_PARAMS | `invalid_kind` |
| Resource enumeration IO error | INTERNAL_ERROR | `resource_enum_failed` |

Per FR-083, command-kind never enumerates resources — calling for a command always succeeds with `resources` field absent.

## `get_skill` — updated

### Input schema (extended)

```json
{
  "type": "object",
  "properties": {
    "catalog": { "type": "string" },
    "plugin":  { "type": "string" },
    "name":    { "type": "string" },
    "kind":    { "type": "string", "enum": ["skill", "command"], "default": "skill" },
    "args":    {
      "oneOf": [
        { "type": "string" },
        { "type": "object", "additionalProperties": { "type": "string" } }
      ]
    }
  },
  "required": ["catalog", "plugin", "name"]
}
```

### Behaviour

- Resolves the entry by (catalog, plugin, kind, name).
- Reads the entry's source file from disk.
- Runs the substitution pipeline (per substitution-engine.md):
  - Built-in stage: always.
  - Env-passthrough stage: always.
  - Argument stage: only when `args` provided.
  - Append-fallback stage: only when `args` provided AND stage 3 made no replacements.
- Returns the rendered body.

### Output

```json
{
  "content": "<rendered body after substitution>",
  "path": "/abs/path/to/SKILL.md"
}
```

### Error responses

| Trigger | Code | Slug |
|---|---|---|
| Entry not found | INVALID_PARAMS | `entry_not_found` |
| Source file removed from disk (stale row) | INTERNAL_ERROR | `entry_source_missing` |
| Argument schema mismatch (too many args, named arg not declared) | INVALID_PARAMS | `prompt_argument_mismatch` |
| Substitution failure (e.g. data-dir creation IO error) | INTERNAL_ERROR | `substitution_failed` |
| Plugin/workspace data-dir creation failure | INTERNAL_ERROR | `workspace_data_dir_write_failed` |

## Backwards compatibility

| Scenario | Phase 4 behaviour | Phase 5 behaviour |
|---|---|---|
| Caller passes no `kind` to `get_skill` | n/a (no `kind` parameter existed) | Defaults to `skill` |
| Caller passes no `args` | Body returned verbatim | Body returned with built-in + env substitution applied |
| Caller passes empty `args` object `{}` | n/a | Same as no `args` — substitution stage 3 skipped |
| `search_skills` returns description | Full description (no truncation) | Truncated to `description_max_chars` (default 150) |

The `description_max_chars` default is a Phase 5 change — callers who relied on full descriptions can opt out by passing `description_max_chars: 99999` or by calling `get_skill_info` for the full text.

## Schema generation

All tool input/output shapes use `schemars::JsonSchema` derives (existing pattern from Phase 3 / US1). The generated schemas are surfaced through the rmcp `#[tool]` macro at registration time.

## Tests

| Behaviour | Test |
|---|---|
| `search_skills` truncation honours default 150 + `description_max_chars` override | `tests/mcp_search_skills_truncation.rs` |
| `search_skills` excludes `searchable = 0` entries | `tests/mcp_search_skills_truncation.rs::disable_model_invocation_excluded` |
| `search_skills` returns both kinds with kind discriminator | `tests/entry_e2e.rs::search_returns_both_kinds` |
| `get_skill_info` shape for skill (with resources) | `tests/mcp_get_skill_info.rs::skill_info_includes_resources` |
| `get_skill_info` shape for command (no resources) | `tests/mcp_get_skill_info.rs::command_info_omits_resources` |
| Per-directory cap of 5 + sentinel | `tests/mcp_get_skill_info.rs::heavy_directory_capped_with_sentinel` |
| `kind` default selects skill | `tests/mcp_get_skill_info.rs::default_kind_is_skill` |
| `get_skill` with `args` runs substitution | `tests/mcp_prompts.rs::get_skill_with_args_substitutes` |
| `get_skill` without `args` runs built-ins only | `tests/mcp_prompts.rs::get_skill_without_args_still_substitutes_builtins` |
| Byte-stable JSON wire pin (per-tool response shape) | `tests/mcp_search_skills_json_shape.rs`, `tests/mcp_get_skill_info_json_shape.rs` |
