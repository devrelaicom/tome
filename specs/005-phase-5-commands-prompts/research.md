# Phase 5 — Research

Outline of design decisions resolved during Phase 0. Each R-decision states what was chosen, why, and what was considered. Where a decision was already pinned by the source PRD, the entry records that and pins anything the PRD left to the contract.

The structure mirrors Phase 3 / Phase 4 research.md format. R-decisions are numbered for cross-reference from data-model.md, contracts/, and tasks.md.

---

## R-1: Substitution engine — hand-rolled, no templating library

**Decision**: Hand-rolled `src/substitution/` module. Three regex sets (one per substitution stage; the append-fallback is not a regex pass), no templating-engine dependency.

**Rationale**: The source PRD's Non-goals explicitly excludes "a full templating engine (Tera, MiniJinja, etc.) for entry content — hand-rolled substitution covers Phase 5 needs without the dependency or syntactic ceremony." The substitution surface is small and well-bounded: 12 built-in variables in one namespace, an env-passthrough namespace gated on a literal prefix, four argument patterns, one append fallback. A templating engine would bring conditionals, loops, filters, partials — none of which Phase 5 needs and all of which would create a learning surface for plugin authors that Tome cannot constrain.

Hand-rolled also matches the constitution's KISS / YAGNI principle and the modular-by-boundary discipline: the substitution layer's public surface is a single function `render(body, context) -> Result<String, SubstitutionError>` plus the `SubstitutionContext` builder type.

**Alternatives considered**:
- **Tera**: full templating engine; ~250 KB binary cost; brings Jinja2 semantics that exceed Phase 5's scope. Rejected.
- **MiniJinja**: smaller (~100 KB); same conceptual mismatch. Rejected.
- **handlebars-rust**: mustache-style; same mismatch. Rejected.
- **`subst` crate**: shell-style variable substitution, no defaults, no namespacing — too narrow. Rejected.

Phase 5 stays with `regex` (already a transitive dep via `catalog::git::scrub_credentials` since Phase 1; promoted to direct in Phase 5).

---

## R-2: Regex strategy — one compiled set per substitution stage, `once_cell`-cached

**Decision**: Each of the three regex-driven stages (built-ins, env passthrough, argument substitution) compiles a `regex::Regex` once at first call into a `std::sync::OnceLock<Regex>`. The append-fallback is not regex-based — it is a structural check ("did any prior stage modify the body?") followed by a string append. Stage ordering per FR-050 is enforced by the `render()` function's control flow, not by regex priority.

**Rationale**: Compiling regex on every call would be a measurable cost on a 100 KB body with thousands of MCP `prompts/get` calls per session. `OnceLock` matches the existing Phase 3 / Phase 4 pattern (e.g. `summarise::backend()` uses `OnceLock` for the llama backend). The PRD's per-stage scan invariant (FR-051) maps naturally to "one regex pass per stage, scan-and-replace each in place". The regex patterns:

- **Built-ins**: `\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}` matched against an explicit whitelist of 12 names; unknown matches in the Tome namespace pass through (FR-023).
- **Env passthrough**: `\$\{TOME_ENV_([A-Z0-9_]+)(?::-(.*?))?\}` per FR-030.
- **Arguments**: `\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)` — alternation handles `$ARGUMENTS[N]` / `$ARGUMENTS` / `$N` / `$name` in one scan per FR-040.

The regex layer is the only `regex` crate consumer for substitution; ownership stays in `src/substitution/regex.rs` to keep the consumer pattern visible.

**Alternatives considered**:
- **One combined regex with named groups**: harder to maintain (one large regex string), harder to reason about (interactions between stages); FR-051 explicitly says each stage scans at most once. Rejected.
- **`regex-automata` directly**: lower-level; gives finer control over scan-and-replace; not justified by Phase 5's surface. Rejected.

---

## R-3: Schema migration v2 → v3 — additive ALTER TABLE + backfill defaults + widened unique constraint

**Decision**: Phase 5's migration is the second registered migration in the framework (Phase 4 shipped the first as the v1→v2 structural-only migration). It runs forward-only via `index::migrations::apply_pending` under the advisory lock, in-process on first open of an older database.

DDL shape:
```sql
ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN when_to_use TEXT;
DROP INDEX IF EXISTS skills_unique;
CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);
```

Backfill values per FR-111a:
- Existing rows: `kind = 'skill'`, `searchable = 1`, `user_invocable = 0`, `when_to_use = NULL`.
- Identity preserved: existing `(catalog, plugin, name)` rows become `(catalog, plugin, 'skill', name)` under the widened index, equivalent identity.
- No re-embedding triggered by the migration itself; the next reindex re-evaluates content hashes (including the new `when_to_use` field's contribution to `embedding_text`) and re-embeds only changed rows.

**Rationale**: Mirrors Phase 4's first migration (additive ALTER TABLE + structural index rewrite) and uses the same `MIGRATIONS_OVERRIDE` test injection seam established in Phase 3 / US5. The `ALTER TABLE … DROP INDEX … CREATE UNIQUE INDEX` sequence is wrapped in a single transaction by the migration framework. SQLite's `ALTER TABLE ADD COLUMN` is fast and well-supported.

**Alternatives considered**:
- **Separate `commands` table**: rejected by PRD §Resolved decisions ("Single `skills` table with `kind` discriminator"). The kind discriminator + widened unique constraint is the simpler and contract-pinned shape.
- **Rename `skills` → `entries`**: would touch every existing query + migration test fixture; cost-benefit is negative. Defer to a follow-up if a meaningful documentation gain emerges. Document in concerns.
- **NOT NULL on `when_to_use`**: rejected because existing rows have no `when_to_use` value and backfill cannot synthesise one.

---

## R-4: Prompt name format — `<plugin>__<entry>` with per-portion sanitisation, length caps, and counter-suffix collision handling

**Decision**: Generated prompt names follow the PRD §Prompt naming algorithm:

1. Plugin name: lowercase, replace any non-`[a-z0-9_-]` with `_`, collapse runs of `_`, truncate to 16 chars.
2. Entry name: same sanitisation, truncate to 32 chars.
3. Concatenate with `__` separator: `<plugin>__<entry>`.
4. If `prompt_name` frontmatter is set, replace BOTH portions with the single override (sanitised + truncated similarly). The `__` separator is retained only if the override contains its own; otherwise the override is one piece.
5. The harness prepends its own `mcp__<server>__` prefix per Claude Code's documented format `/mcp__servername__promptname`. Tome MUST NOT prepend this; it is the harness's responsibility.

Collision resolution per FR-062:
- Detected at `prompts/list` enumeration time, not at index time.
- Two entries deriving the same generated name: order by recorded `indexed_at` ascending, then break ties on the lexicographic order of `(catalog, plugin, kind, name)`.
- The earliest entry gets the unsuffixed name; subsequent entries get `<name>2`, `<name>3`, etc. starting at counter 2.
- Each collision is logged at `warn!` level with original entry identities, derived name, and final suffixed name.

**Rationale**: The format `<plugin>__<entry>` is pinned by the source PRD. The Claude Code MCP docs confirm the harness-prepended prefix is `mcp__<server>__` (see https://code.claude.com/docs/en/mcp.md §"Use MCP prompts as commands"). The per-portion length caps (16 + 32) leave headroom under MCP's overall prompt-name budget after the harness adds its `mcp__tome__` prefix.

Counter starting at 2 (`foo`, `foo2`, `foo3`) matches the Phase 4 contract pattern used in workspace name conflicts. Tie-breaking on `(catalog, plugin, kind, name)` lexicographic order is deterministic for batch-inserted entries sharing a timestamp (FR-062).

**Alternatives considered**:
- **Hash-based names**: opaque to plugin authors and users; rejected for UX.
- **`<catalog>__<plugin>__<entry>`**: too long under MCP's name budget; the catalog identity is uninteresting to slash-menu users (they typically have one catalog or treat it as transparent). Rejected.
- **Counter starting at 1**: less natural than starting the first conflict at suffix `2`. Rejected.

---

## R-5: MCP prompts capability declaration — rmcp `prompts` capability with `listChanged: false`

**Decision**: The MCP server declares the `prompts` capability during initialization with `listChanged: false` per NFR-008. Implementation via rmcp's `#[tool_router]` macro pattern carries over from Phase 3; the prompts handlers register similarly.

```rust
// Conceptual shape — actual API verified during implementation
ServerInfo::new(
  ServerCapabilities {
    tools: Some(ToolsCapability { list_changed: Some(false) }),
    prompts: Some(PromptsCapability { list_changed: Some(false) }),
    ..Default::default()
  }
)
```

`prompts/list` and `prompts/get` request handlers register through the rmcp `#[prompt_router]` (or equivalent) macro, mirroring the `#[tool_router]` pattern. Each handler runs the sync work inside `spawn_blocking` (Phase 3 pattern; rusqlite + substitution layer are sync).

**Rationale**: The PRD §MCP server changes pins `listChanged: false` because:
1. The prompts surface only changes on plugin enable/disable or workspace switch, both of which happen outside an MCP session.
2. MCP `notifications/prompts/list_changed` would require the server to push notifications; rmcp supports this via its `RoleServer` API but introducing it for a use case that doesn't trigger within-session is over-engineering.
3. Workspace switches require server restart anyway under Tome's current per-session model.

The Claude Code MCP docs confirm `list_changed` notifications are supported by the harness; Tome's choice not to emit them is deliberate.

**Alternatives considered**:
- **`listChanged: true` with notifications on plugin enable/disable**: would require coordinating between the server process and CLI invocations; the MCP server is a separate process from the `tome plugin enable` invocation. Cross-process notification is not in scope. Rejected.
- **Per-request DB re-scan**: would let the server respond to prompt list changes without notifications, but rmcp's prompts capability still requires `listChanged` declared one way or the other. Rejected.

---

## R-6: `commands/*.md` walk — flat (non-recursive) markdown file scan at `<plugin>/commands/`

**Decision**: At plugin enable time, `src/plugin/components.rs` walks `<plugin>/commands/*.md` non-recursively (flat directory listing, filtered to `*.md` files). This matches Claude Code's commands directory layout per the Claude Code skills docs ("`.claude/commands/deploy.md` → `/deploy`").

The existing `<plugin>/skills/*/SKILL.md` walk (which IS recursive into per-skill subdirectories looking for `SKILL.md`) is unchanged. Skills are directory-rooted; commands are flat-file. This shape is canonical per Claude Code's skills/commands distinction.

Each command file becomes one entry row of kind `command`. Each `SKILL.md` becomes one entry row of kind `skill`.

**Plugin-root SKILL.md is explicitly out of scope.** Claude Code's skills documentation describes a layout where a plugin's root directory itself contains a `SKILL.md` (treated as a single skill named after the plugin via the frontmatter `name` field, with the plugin directory name as fallback). Tome has never supported this layout — neither Phase 4 nor any earlier phase walks the plugin root for `SKILL.md`. Phase 5 deliberately does NOT add support: the cross-harness value is low (most plugins use the standard `skills/<name>/SKILL.md` layout), and the naming collision surface against `commands/` would need its own resolution. Plugin authors targeting Tome should put their entries under `skills/` or `commands/` subdirectories. A future phase MAY add plugin-root SKILL.md support if user demand emerges; for now it is documented as a permanent non-goal alongside subagent translation.

**Rationale**: The PRD §Indexing pipeline updates pins both walk shapes: "`plugin/skills/*/SKILL.md` — kind `'skill'`; `plugin/commands/*.md` — kind `'command'`". This matches what Claude Code itself does. Non-recursive `commands/` walk keeps the canonical shape simple; plugin authors who want hierarchical commands can nest in subdirectories with `.md` extensions only at the top level. Phase 5 does not impose a directory convention beyond what Claude Code itself does.

**Alternatives considered**:
- **Recursive `commands/**/*.md` walk**: would index commands in subdirectories. Claude Code's loader is also flat-only (file under `.claude/commands/` only). Mirroring that is correct. Rejected.
- **Skill-style `commands/<name>/COMMAND.md`**: not Claude Code's convention. Rejected.

---

## R-7: Frontmatter parsing — extend existing lenient `serde_yaml` parser with widened field set

**Decision**: `src/plugin/frontmatter.rs` gains support for `disable-model-invocation` (bool, default false), `user-invocable` (bool, default depends on kind — `false` for skill, `true` for command), `arguments` (string or `Vec<String>` — both forms accepted), `argument-hint` (string), `prompt_name` (string), `when_to_use` (string). All fields remain optional.

The struct is parsed leniently per the existing third-party boundary — unknown fields are tolerated, recognised fields with malformed values fail loudly. Boolean fields accept YAML truthy/falsy per `serde_yaml`'s defaults. The `arguments` field uses a custom deserializer that accepts both a space-separated string AND a YAML list, matching Claude Code's documented behavior ("Accepts a space-separated string or a YAML list").

Default value computation depends on entry kind:
- For skills: `user_invocable` defaults to `false` (Tome convention; differs from Claude Code's default of `true`).
- For commands: `user_invocable` defaults to `true`.
- For both: `searchable` defaults to `true`, overridable to `false` via `disable-model-invocation`.

**Rationale**: The widened field set is pinned by the PRD §Frontmatter spec and confirmed by the Claude Code skills docs (verified against https://code.claude.com/docs/en/skills.md §"Frontmatter reference"). The skill default for `user_invocable` differs from Claude Code's default — Tome deliberately defaults skills to NOT slash-invocable to avoid 100s of plugin skills cluttering harness slash menus. Plugin authors who want a skill in the slash menu set `user-invocable: true` per FR-012.

**Alternatives considered**:
- **Match Claude Code's default exactly** (skills `user_invocable: true`): would cause every Tome user with plugins enabled to suddenly see 100s of slash-menu entries. Tome's MCP-prompts surface is multi-plugin aggregated; Claude Code's native skills surface is single-plugin scoped. Defaults appropriately differ. Rejected.
- **`arguments` field accepting only YAML list**: would break Claude Code-authored plugins that use the space-separated string form. Rejected per the PRD's "no per-plugin rewrites" goal.

---

## R-8: Substitution call sites — read tool + prompts/get; explicit non-callers — search_skills, get_skill_info

**Decision**: Per FR-046, exactly two MCP surfaces invoke the substitution layer:
- `get_skill` (read tool): built-in and env-passthrough stages always run; argument-substitution + append-fallback only when caller passed args.
- `prompts/get`: built-in and env-passthrough always run; argument-substitution + append-fallback run when caller passed args (which is the typical case for prompts).

`search_skills` and `get_skill_info` MUST NOT invoke the substitution layer. They return metadata (untransformed descriptions, when_to_use text, resource enumerations); they never return body content.

Substitution layer is invoked sync inside `spawn_blocking` per the Phase 3 US1 pattern (NFR-010).

**Rationale**: This split makes the layer's cost predictable. Search and middle-tier discovery are cheap by design; only the surfaces that actually return body content pay the substitution cost. FR-046 codifies the rule.

---

## R-9: Persistent data directory layout — central tree, sanitised path components, lazy creation, idempotent under concurrent retrieval

**Decision**: Per FR-021:
- `${TOME_PLUGIN_DATA}` → `<home>/.tome/plugin-data/<catalog-sanitised>/<plugin-sanitised>/`
- `${TOME_WORKSPACE_DATA}` → `<home>/.tome/workspaces/<workspace-name>/plugin-data/<catalog-sanitised>/<plugin-sanitised>/`

Where `<home>/.tome/` is the Phase 4 central state root (constitution v1.3.0 §Paths).

Sanitisation per FR-024: catalog and plugin name path components have any character illegal under any supported OS (`/ \ : * ? " < > |` and ASCII control) replaced with `_`. Readable characters are preserved. The unsanitised values are still returned by `${TOME_CATALOG_NAME}` and `${TOME_PLUGIN_NAME}` substitutions.

Creation per FR-021 + NFR-012:
- Lazy: `create_dir_all(path)` on first reference within a substitution pass.
- Idempotent: `create_dir_all` is idempotent by definition.
- Concurrent-safe: `create_dir_all` is safe under concurrent calls (the kernel-level `mkdir` is atomic; either succeeds or returns `EEXIST` which `create_dir_all` treats as success).
- Failure path: per Edge Case "directory creation failure", emit a dedicated error code (PRD §Exit codes 25 — "Workspace data directory write failed"); fail the retrieval rather than partial substitution.

Workspace rename relocation (FR-025): `tome workspace rename` (Phase 4 US2) must detect existence of `<home>/.tome/workspaces/<old-name>/plugin-data/` and `std::fs::rename` it to `<home>/.tome/workspaces/<new-name>/plugin-data/` inside the same transaction that updates the workspace row. If the source doesn't exist (workspace was never invoked with a substitution-bearing entry), the rename is a no-op. Failure to rename surfaces exit 25.

**Rationale**: Anchoring the data directories under `<home>/.tome/` matches the Phase 4 v1.3.0 constitutional consolidation. The naming pattern `<home>/.tome/plugin-data/<catalog>/<plugin>/` mirrors Claude Code's `${CLAUDE_PLUGIN_DATA}` semantics (per the Claude Code MCP docs §"Plugin-provided MCP servers") — a per-plugin persistent state area that survives plugin updates. The two-tier (per-plugin + per-workspace-per-plugin) split per FR-021 lets plugin authors distinguish global state (shared across the user's projects) from workspace-scoped state (one user, multiple unrelated project clusters).

**Alternatives considered**:
- **Per-project data dirs** (under each bound project's `.tome/`): rejected because it conflates plugin state with project state; project markers are thin pointers per Phase 4 design.
- **Hashed cache-style dirs** (sha256 of catalog URL + plugin name): rejected because plugin authors need readable paths to write to.
- **No FR-025 rename**: leave a stranded `<old-name>` directory on rename; rejected because subsequent `${TOME_WORKSPACE_DATA}` resolutions would silently lose user data.

---

## R-10: Argument coercion — single string vs structured object, named-argument zipping rules

**Decision**: Per FR-041–FR-043 and the Claude Code skills docs §"Pass arguments to skills":

- **Caller passes a single string** (e.g. `args: "foo bar baz"`):
  - If entry's frontmatter declares named arguments: split the string on whitespace using shell-style quoting (per Claude Code's documented behavior: `"hello world" second` → `["hello world", "second"]`); zip with declared names in order. Extra tokens beyond declared names are kept as positional values for `$ARGUMENTS[N]` / `$N`.
  - If entry declares no named arguments: treat as a single positional value (no whitespace splitting); `$ARGUMENTS` resolves to the whole string per Claude Code's documented "whole string" behavior.
- **Caller passes a structured object** (e.g. `{component: "X", from: "Y", to: "Z"}`):
  - Named-substitution values come from the object directly.
  - Positional values for `$ARGUMENTS[N]` / `$N` derived from object values in declaration order.
  - `$ARGUMENTS` resolves to the joined positional values with single-space separator.
- **Caller passes nothing**:
  - Argument substitution stage and append-fallback both skipped.
  - All argument references in body resolve to empty strings (FR-040 — "empty string if out of range/not provided" per PRD §Argument substitution).

**Shell-style quoting** (per Claude Code docs): tokens may be wrapped in single or double quotes to preserve whitespace inside a token. Quoting respect for nested quotes is not in Phase 5 scope (Claude Code's own behavior is also limited).

**Rationale**: Matches Claude Code's documented behavior verbatim; ensures argument-bearing commands authored against Claude Code work on Tome without modification. The named-vs-positional disambiguation by entry's frontmatter declaration is explicit per FR-041/042.

**Alternatives considered**:
- **Always whitespace-split**: would break the "whole-string" `$ARGUMENTS` case (entries that intentionally accept a multi-word freeform string). Rejected.
- **JSON-only object input**: rejected because MCP's `prompts/get` arguments are string-typed by spec; structured args come from harness UI mapping.

---

## R-11: get_skill_info tool — third agent-callable tool sitting between search_skills and get_skill

**Decision**: New `src/mcp/tools/get_skill_info.rs` registers a third tool via the rmcp `#[tool]` pattern. Input/output shapes per FR-080–FR-085:

Input:
```jsonschema
{
  "catalog": "string (required)",
  "plugin":  "string (required)",
  "name":    "string (required)",
  "kind":    "string (optional, default 'skill')"
}
```

Output (skill-kind):
```json
{
  "catalog": "...",
  "plugin": "...",
  "name": "...",
  "kind": "skill",
  "path": "/abs/path/to/SKILL.md",
  "description": "<full>",
  "when_to_use": "<full or null>",
  "plugin_version": "...",
  "user_invocable": false,
  "resources": {
    "files": ["..."],
    "directories": {
      "scripts": ["..."],
      "references": ["..."]
    }
  }
}
```

Output (command-kind): same shape minus `resources` (which is omitted entirely per FR-083).

Resource enumeration per FR-081/082:
- `files`: top-level files in entry's directory, alphabetical, excluding the entry file itself.
- `directories`: keyed by subdirectory name (alphabetical), values are one-level child paths (alphabetical).
- Per-directory cap: 5 children; over-cap directories list the first 5 + sentinel string `"and N more"` where N is the count of omitted children.
- Top-level `files` array is also capped at 5 with the same sentinel.

The tool does NOT invoke the substitution layer (per R-8).

**Rationale**: Schema and shape pinned by PRD §`get_skill_info` — new. The per-directory cap of 5 is pinned (FR-082) so SC-004's order-of-magnitude payload-size claim holds. Alphabetical sort order is the deterministic default (no per-author ordering preference).

**Alternatives considered**:
- **Cap of 10**: more permissive; rejected because SC-004's order-of-magnitude claim weakens. Authors with > 5 of one kind of resource clearly have heavy supporting material — the agent can use Glob/Grep tools to enumerate further.
- **Recursive enumeration**: rejected per FR-081 ("one-level enumeration"). Heavy supporting trees would explode the payload size.

---

## R-12: Indexing `when_to_use` — embedding text composer includes it when set

**Decision**: Per FR-005 and PRD §Indexing pipeline updates §Embedding text composition:

```text
{name}

{description}

When to use: {when_to_use}
```

Where the "When to use:" prefix + blank-line separator appears only when `when_to_use` is non-empty. The embedding model and dimensionality stay at the existing bge-small-en-v1.5 INT8 ONNX (384-dim) — no model change.

Existing rows pre-migration (FR-111a) have `when_to_use = NULL`; their `embedding_text` follows the existing `{name}\n\n{description}` shape until their next reindex re-evaluates. After reindex, rows whose frontmatter now has `when_to_use` will recompose `embedding_text` and re-embed if the content hash changed.

**Rationale**: Improves retrieval quality by indexing the disambiguation hint plugin authors are already writing for the agent. The PRD specifies the exact composition format. The Claude Code skills docs confirm `when_to_use` is a recognised optional field whose use is "additional context for when Claude should invoke the skill, such as trigger phrases or example requests" — perfect for embedding retrieval.

Content-hash invalidation: the `embedding_text` change is captured by the existing content-hash diffing in `index::skills::enable_plugin_atomic`. Rows whose `when_to_use` value differs from the indexed version (or whose `when_to_use` newly appears) will re-embed on the next reindex.

**Alternatives considered**:
- **Always embed `when_to_use` separately as a second vector**: doubles storage and query cost; rejected.
- **Weight `when_to_use` higher in the composition**: no clear gain without ablation; rejected for KISS.

---

## R-13: ARGUMENTS append fallback — detection by "did argument substitution modify the body?" sentinel

**Decision**: The append fallback runs in stage 4 of the substitution pipeline (FR-050). It is NOT a regex pass; it is a structural check on whether the argument-substitution stage produced any replacements.

Algorithm:
1. Argument substitution stage records whether it performed any replacements (boolean flag).
2. If the caller supplied arguments AND the argument-substitution stage produced no replacements:
   - Append a documented footer to the body: `\n\nARGUMENTS: <values>`
   - Where `<values>` is the same string the caller passed (single string) or the joined positional values (structured object case, joined with single space).

Per FR-044 ("if args are provided AND the body has no substitution references … append `ARGUMENTS: <value>`").

The detection is done by the argument-substitution stage itself reporting `replacements_performed: bool`; the append-fallback stage consults this flag rather than re-scanning the body.

**Rationale**: Avoids re-scanning the body for the existence of `$ARGUMENTS` / `$N` / `$<name>` patterns. The argument-substitution stage already scans the body once; tracking whether it made any replacements is free during the scan.

**Alternatives considered**:
- **Re-scan the body for argument patterns to decide**: redundant work; FR-051 says each stage scans at most once.
- **Always append the footer when args are passed**: would duplicate arguments visible both inline (via `$ARGUMENTS`) and in the footer. Rejected per PRD §Argument substitution: "if args are provided AND the body has no substitution references".

---

## R-14: rmcp prompts API verification — confirm capability and handler shape match Phase 5's requirements

**Decision**: Use rmcp's `#[prompt_router]` macro (or equivalent — verify exact name during implementation) following the `#[tool_router]` pattern. The prompts capability is declared via `ServerCapabilities { prompts: Some(PromptsCapability { list_changed: Some(false) }), ... }`.

Per the Phase 3 US1 retro learning: rmcp's macros have some surprises:
- `#[tool(description = "...")]` accepts only string literals, not `const &str`; the pattern was to use `///` doc comments. Apply the same pattern to prompt handlers.
- `#[tool_router(vis = "pub")]` allows test introspection; apply the same `vis = "pub"` to `#[prompt_router]` for test coverage of `prompts/list` enumeration.
- `ServerInfo` / `Implementation` are `#[non_exhaustive]`; construct via builder methods.

Handler shape:
```rust
// Conceptual — exact API verified during implementation
#[prompt_router(vis = "pub")]
impl Server {
    /// List all user-invocable entries for the active workspace as MCP prompts.
    async fn prompts_list(&self, ctx: ListPromptsContext) -> Result<ListPromptsResult, ErrorData> { ... }

    /// Render a named prompt with optional arguments through the substitution layer.
    async fn prompts_get(&self, ctx: GetPromptContext) -> Result<GetPromptResult, ErrorData> { ... }
}
```

Each handler dispatches its sync work (DB lookup + substitution) inside `tokio::task::spawn_blocking`.

**Rationale**: rmcp is the existing MCP server library; no alternative was considered. Confirming the macro names and capability shape during implementation (not at planning) is appropriate because the rmcp API has been evolving and a Phase 5 work session will pin to a specific rmcp version. The current Phase 4 version (rmcp 1.x with `transport-io` + `schemars` features) is the baseline.

**Action at implementation**: First task of US1 (commands as prompts) is a 1-hour exploratory pass against the current rmcp version to verify the prompts API shape; record findings in a tasks.md note.

---

## R-15: Phase 4 deferred items — disposition for Phase 5

Phase 4's polish closed at v0.4.0 with ~30 items deferred per `specs/004-phase-4-refactor-harnesses/review/disposition.md` § "Deferred to follow-up issue". For each, the Phase 5 disposition:

| Phase 4 deferred item | Phase 5 disposition |
|---|---|
| C-M2 (Paths canonicalisation) | Not Phase 5 scope. Defer to later. |
| C-M4-7/M10-11 (contract doc-only) | Not Phase 5 scope. |
| R-M1 (symlink-refusal helper consolidation) | Not Phase 5 scope. |
| R-M2 (settings parser helper consolidation) | Not Phase 5 scope. |
| R-M6 (workspace-projects helper consolidation) | Not Phase 5 scope. |
| R-M9/M10/M11 (cosmetic) | Not Phase 5 scope. |
| S-M3 (prompt-injection design) | Touched by Phase 5: the substitution layer's NFR-005 + FR-033 enforces the env-namespace boundary that S-M3 was concerned about. Carry-forward verified during US2 implementation. |
| S-M4 (--fix --force dry-run) | Not Phase 5 scope. |
| S-M5 (summariser lock) | Not Phase 5 scope. |
| S-M8 (llama-cpp-2 traceability) | Not Phase 5 scope. |
| T-M1/M2/M3/M4/M5/M11 (fixture promotion) | Not Phase 5 scope; tests' fixture model is stable. |
| C-B1/C-B2/C-B3 / R-M3/M4/M5/M7/M8/M12 | Already shipped in Polish. |

Phase 5 does not carry forward any Phase 4 deferred item as a Foundational requirement. The substitution-layer-touches-S-M3 connection is recorded as a cross-link in `contracts/substitution-engine.md` for the implementer's awareness.

---

## R-16: Test injection seams for Phase 5

**Decision**: New test injection seams introduced in Phase 5 follow the established `OVERRIDE_SLOT + RAII Guard + Drop-cleans-slot` pattern (HarnessModulesGuard, MigrationsGuard, SummariserOverrideGuard precedents):

- **`SUBSTITUTION_CLOCK_OVERRIDE`** (`OnceLock<Option<time::OffsetDateTime>>`): tests inject a fixed clock so `${TOME_DATE}` and `${TOME_TIMESTAMP}` substitutions are deterministic. Guard: `ClockOverrideGuard`. Per NFR-001 + the Phase 4 P6 lesson ("Test fixtures + Clock injection seam in the same PR as the production code that consumes it").
- **`PROMPT_REGISTRY_OVERRIDE`** (`OnceLock<Option<Vec<PromptDescriptor>>>`): tests inject a synthetic prompt registry for collision testing, prompt-name-override testing, and `prompts/list` enumeration tests without a full DB seed. Guard: `PromptRegistryGuard`.
- **`PLUGIN_DATA_DIR_OVERRIDE`** (`OnceLock<Option<PathBuf>>`): tests redirect `${TOME_PLUGIN_DATA}` resolution to a tempdir-rooted path; same for `WORKSPACE_DATA_DIR_OVERRIDE`. Guards: `PluginDataDirGuard`, `WorkspaceDataDirGuard`.

Each guard's `Drop` impl clears the slot, restoring previous state. Tests using `HOME` environment mutation continue to use the existing `HOME_MUTEX` + `HomeGuard` pattern (Phase 4 US3).

**Rationale**: Per the Phase 4 P6 retro lesson: "If you add a `XYZ_OVERRIDE` slot, the same PR includes at least one `tests/*_end_to_end.rs` test that installs an override + invokes the production code path that consults the slot." Every new override seam in Phase 5 ships with at least one end-to-end consumer test in the same PR.

---

## R-17: Pre-emptive slice plans per user story (Phase 4 P3 retro lesson)

The Phase 4 P3 retro called for "pre-emptive sub-slice planning when the brief would exceed 8 KB". Phase 5 plans each user story with explicit slice splits up front, NOT discovered mid-implementation.

**User Story 1 (P1) — Commands as MCP prompts**. Slices:
- US1.a: Schema migration v2→v3 + frontmatter widening + plugin commands directory walk. (Foundational for Phase 5.)
- US1.b: MCP prompts capability declaration + `prompts/list` handler + prompt-name derivation + collision handling.
- US1.c: MCP `prompts/get` handler + substitution layer wiring (built-ins + env stages from US2; argument stage from US3).
- US1.d: Reviewer pass (4 agents) + closeout (sdd:map incremental + retro).

**User Story 2 (P2) — Substitution layer (paths/env)**. Slices:
- US2.a: `src/substitution/` module skeleton + builtins stage + clock injection seam.
- US2.b: Env passthrough stage + lazy data directory creation + workspace rename relocation (FR-025).
- US2.c: Reviewer pass + closeout.

**User Story 3 (P3) — Argument substitution**. Slices:
- US3.a: Argument substitution stage + four argument patterns + name binding.
- US3.b: Append-fallback footer + ARGUMENTS detection sentinel.
- US3.c: End-to-end via `prompts/get` with structured-arg input from harness; closeout.

**User Story 4 (P4) — Middle-tier discovery + when_to_use indexing**. Slices:
- US4.a: `get_skill_info` tool handler + resource enumeration + per-directory cap.
- US4.b: `when_to_use` indexing in embedding_text composer + reindex re-eval.
- US4.c: `search_skills` description truncation parameter; closeout.

**User Story 5 (P5) — Per-entry invocability flags + doctor extensions**. Slices:
- US5.a: `disable-model-invocation` + `user-invocable` honoured end-to-end through search + prompts surfaces.
- US5.b: `tome plugin show` annotations + `tome doctor` Phase 5 surface (FR-120–124).
- US5.c: Reviewer pass + closeout.

**Foundational / pre-US slices**:
- F1: New `TomeError` variants + exit codes 21–25 pre-allocated (Phase 4 P5 pre-emptive-allocation pattern).
- F2: `regex` promoted from transitive to direct dep in `Cargo.toml`.
- F3: `src/substitution/` module skeleton with `Summariser`-style trait + Stub impl for downstream tests.

**Polish phase** (post-US5):
- Phase-wide 4-reviewer pass (Phase 4 P8 pattern).
- Disposition.md routing.
- 5–6 polish PRs landing blockers + selected majors.
- Final sdd:map + retro + CLAUDE.md + version bump.

---

## R-18: Per-user-story reviewer pass — keep the 4-agent pattern from Phase 3 / Phase 4

**Decision**: Phase 5 maintains the per-US 4-reviewer pass pattern (contract / Rust-lens / test / security) established in Phase 3 / US1 and refined through Phase 4's user stories. Each US closeout:

1. Run 4 reviewer agents in parallel with per-scope briefs.
2. Each writes findings to `/tmp/tome-phase5-us<N>-<reviewer>.md`.
3. Main thread consolidates to `specs/005-phase-5-commands-prompts/review/us<N>-findings.md` + `disposition.md`.
4. Apply all blockers + selected majors before US closes.
5. Defer remaining majors / minors via disposition.md routing to Polish or follow-up issue.

**Rationale**: Carries forward Phase 4's established cadence; the pattern caught real production-vs-test divergences (Phase 4 US3, US4, US5 each had at least one blocker the reviewer pass surfaced).

Per the Phase 4 P5 + P6 + P8 retros' compounding "next time" item: extract a `review/template-{contract,rust,test,security}.md` brief template at Phase 5 start. The template defines per-reviewer scope + finding-classification rules + expected output paths. US1 onward instantiates the template.

---

## R-19: JSON wire-shape pin discipline (Phase 4 P8 lesson)

**Decision**: Every `#[derive(Serialize)]` struct landing in a CLI `--json` envelope or an MCP response shape ships with a byte-stable JSON pin test in the SAME PR that introduces the type. Per the Phase 4 P8 retro lesson.

Phase 5 introduces at least these new emit-only types requiring pin tests:

| Type | Pin test file |
|---|---|
| `EntryRow` (when surfaced via CLI) | `tests/entry_row_json_shape.rs` |
| `SearchResult` (post-truncation) | `tests/mcp_search_skills_json_shape.rs` |
| `SkillInfo` (`get_skill_info` response) | `tests/mcp_get_skill_info_json_shape.rs` |
| `PromptDescriptor` (`prompts/list` element) | `tests/mcp_prompts_list_json_shape.rs` |
| `PromptGetResponse` | `tests/mcp_prompts_get_json_shape.rs` |
| `ResourceEnumeration` | covered by `mcp_get_skill_info_json_shape.rs` |
| Doctor Phase 5 surface additions | extend `tests/doctor_json.rs` |

Pin tests assert the serialised string byte-for-byte against a fixture, including field order. Phase 4 P8 explicitly called this out as a "compounding lesson" from each User Story; Phase 5 acts on it.

---

## R-20: Constitution gate post-Phase 1 design re-check

**Decision**: A second constitution check runs after Phase 1 artifacts (data-model.md, contracts/) are written. Mirrors the Phase 3 / Phase 4 plan structure. Expected to pass — no new deps, no new top-level CLI command, no async outside `src/mcp/`, no log-secrets risk, all closed-enum discipline preserved.

If Phase 1 design introduces an unexpected violation (e.g. the contracts surface requires a new dep that pushes binary size beyond the cap), the plan returns to research with Complexity Tracking before continuing.
