# MCP Tools — Schema Contract

Two tools advertised by `tome mcp`. Both honour the constraints in spec FR-103 through FR-108.

## `search_skills`

**Description** (≤ 350 chars; normative shape per FR-108 — exact wording iterated against real-harness behaviour):

> Find the most relevant skills in the local Tome index for a natural-language task description. Call this proactively before approaching any non-trivial task to discover existing skills you can rely on. Returns a ranked list of candidates with on-disk paths; follow up with `get_skill` to load the skill body and resource files.

The description **must not** enumerate any specific catalog, plugin, or skill name (FR-108). The string is checked in `tests/mcp_server.rs` against a list of plugin / skill identifier substrings present in the test fixture; failure means the wording leaks an identifier.

### Input schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["query"],
  "properties": {
    "query":   { "type": "string", "minLength": 1, "description": "Natural-language description of the task." },
    "top_k":   { "type": "integer", "minimum": 1, "maximum": 100, "default": 10, "description": "Maximum results to return after reranking." },
    "catalog": { "type": "string", "description": "Restrict to one catalog by name (must match an enabled catalog in the resolved scope)." },
    "plugin":  { "type": "string", "description": "Restrict to one plugin within `catalog` (requires `catalog`). Format: plugin name only, NOT '<catalog>/<plugin>'." }
  }
}
```

**Derived constraints**:
- `plugin` without `catalog` → tool returns a structured error `{ "code": "plugin_without_catalog", … }`.
- `catalog` naming a catalog absent from the resolved scope → `{ "code": "unknown_catalog", "catalog": "<value>" }`.
- `plugin` naming a plugin absent from the named catalog → `{ "code": "unknown_plugin", "catalog": "...", "plugin": "..." }`.
- Filters apply **before** reranking. Candidate pool is `top_k × 4` post-filter; the reranker reduces to `top_k`.

### Output schema

```json
{
  "type": "object",
  "required": ["matches"],
  "properties": {
    "matches": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["catalog", "plugin", "name", "description", "plugin_version", "path", "score"],
        "properties": {
          "catalog":        { "type": "string" },
          "plugin":         { "type": "string" },
          "name":           { "type": "string" },
          "description":    { "type": "string", "description": "The indexed description (frontmatter `description` or fallback per FR-012)." },
          "plugin_version": { "type": "string" },
          "path":           { "type": "string", "description": "Absolute path to the SKILL.md file." },
          "score":          { "type": "number", "description": "Reranker score by default; embedding similarity if reranker drift forced fallback. The output does NOT distinguish — the score is opaque." }
        }
      }
    }
  }
}
```

### Behavioural contract

1. Embed `query` via the eager-loaded embedder.
2. Lazy-load the reranker on the first call (idempotent — subsequent calls reuse).
3. Apply optional `catalog` / `plugin` filters.
4. KNN over the filtered candidate pool, candidate count = `top_k × 4` (or all available if smaller).
5. Rerank with the cross-encoder.
6. Trim to `top_k`.
7. Return.

**No `--strict` equivalent.** The CLI's `--strict` flag (which exits 40 when no result clears a threshold) has no analog in MCP — the agent decides whether the score is high enough. The tool always returns whatever it found, possibly an empty array.

**No `--no-rerank` equivalent.** The agent gets the production scoring pipeline by default and cannot turn off the reranker. Debugging that surface is the CLI's job.

### Error envelope

`rmcp::Error` with one of these `code` strings:

| `code` | Meaning |
|---|---|
| `plugin_without_catalog` | `plugin` parameter provided but `catalog` omitted. |
| `unknown_catalog` | `catalog` parameter does not match any enabled catalog in the resolved scope. |
| `unknown_plugin` | `plugin` parameter does not match any plugin in the named catalog. |
| `embedder_drift` | Embedder identity in the index doesn't match the running embedder. Server should not reach this — pre-flight catches it — but the tool returns this code defensively. |
| `index_busy` | Read failed because the index was contended past timeout. Rare on read paths. |

---

## `get_skill`

**Description** (≤ 250 chars; normative shape per FR-108):

> Fetch the body of one skill by `(catalog, plugin, name)` — typically a triple returned by a prior `search_skills` call. Returns the skill body with frontmatter stripped, plus the absolute paths of every sibling resource file in the skill's directory.

Same enumeration check applies — no specific identifiers may appear in the description.

### Input schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["catalog", "plugin", "name"],
  "properties": {
    "catalog": { "type": "string", "minLength": 1 },
    "plugin":  { "type": "string", "minLength": 1 },
    "name":    { "type": "string", "minLength": 1, "description": "The skill `name` field as returned by `search_skills`." }
  }
}
```

### Output schema

```json
{
  "type": "object",
  "required": ["content", "path", "resources"],
  "properties": {
    "content":   { "type": "string", "description": "SKILL.md body with YAML frontmatter stripped. Body is otherwise verbatim — no normalisation, no rewrites, no path-relative-to-absolute resolution in code blocks." },
    "path":      { "type": "string", "description": "Absolute path to the SKILL.md file." },
    "resources": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths of every OTHER file in the skill's directory (recursive). The agent may load any of them via its own file-reading tools." }
  }
}
```

### Behavioural contract

1. Resolve `(catalog, plugin, name)` against the resolved scope's index. Only enabled skills are considered (FR-107: an existing-but-disabled skill is `unknown_skill`).
2. Read the SKILL.md file at the index-recorded `path`.
3. Strip the YAML frontmatter delimited by `---` lines using the same parser the enable pipeline uses (`plugin::frontmatter::strip`); preserve the body verbatim including trailing newlines.
4. Walk the SKILL.md's parent directory recursively, collecting every file *except* the SKILL.md itself. Return their absolute paths in lexicographic order.
5. Return.

**No size limit on `content`.** The tool surface is the same shape as a file read; a 1 MB SKILL.md returns 1 MB of content. The agent decides whether to truncate.

**No filter on `resources`.** Hidden files (`.gitkeep`), binary blobs (images, fonts), and large files (training data, models) are all listed. The agent's own file-reading tools decide what to fetch.

### Error envelope

| `code` | Meaning |
|---|---|
| `unknown_catalog` | The named catalog is not enabled in the resolved scope. |
| `unknown_plugin` | The catalog is enabled but does not contain the named plugin. |
| `unknown_skill` | The plugin is enabled but does not have a skill with this name, OR the skill is recorded but its `enabled` flag is 0. |
| `skill_file_missing` | Index says the file should exist at `path`, but the file is not on disk. The index is stale; the developer should `tome reindex <catalog>/<plugin>`. |
| `frontmatter_strip_failed` | The SKILL.md body could not be parsed even leniently. Should never happen for an indexed skill (the index pipeline already accepts FR-013c skips), but the variant exists for completeness. |

---

## Cross-tool invariants

- Neither tool name appears verbatim in either description. The descriptions reference "search" and "get the skill" by behavior, not by tool name.
- Neither tool exposes a `--workspace` / `--global` parameter. The server is fixed to one scope for its lifetime.
- Neither tool writes to the index. Both are pure reads.
- Neither tool prompts the user for anything. The MCP protocol has no prompting affordance available to a Tome-owned tool surface in Phase 3.
- Neither tool runs network I/O. Models are loaded from disk; if a model is missing the server refuses to start, never mid-call.

## Versioning

The shape above is the v1 contract. Any breaking change (renamed field, removed field, changed type) requires advancing the MCP server's advertised version and ships in a future spec.
