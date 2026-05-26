# Phase 5 — Substitution engine

Authoritative contract for the `src/substitution/` module. The engine renders entry bodies by composing four ordered stages over a substitution context. Hand-rolled (no templating library per Phase 5 Non-goals + R-1).

## Public API

```rust
pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError>;

pub struct SubstitutionContext { ... }  // see data-model.md §3.2
pub struct SubstitutionContextBuilder { ... }
pub enum SubstitutionError { ... }
```

Single entry point: `substitution::render`. Idempotent against the same inputs; deterministic against a fixed clock (NFR-001 + R-16).

## Stage ordering (FR-050)

1. **Built-ins**: `${TOME_<NAME>}` references where `<NAME>` is a recognised Phase 5 built-in.
2. **Environment passthrough**: `${TOME_ENV_<NAME>}` references with optional `:-default` syntax.
3. **Argument substitution**: `$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`, `$<name>` references.
4. **`ARGUMENTS:` append fallback**: footer appended only when stage 3 made no replacements AND caller supplied arguments.

Each stage scans the body at most once (FR-051). Substituted values are NOT re-scanned by later stages (FR-051). References outside the Tome / Tome-env namespaces pass through unchanged (FR-052).

## Stage 1 — Built-ins

Regex (compiled once, `OnceLock`-cached):
```
\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}
```

Capture groups: `1` = variable name (uppercase ASCII + digits + underscores); `2` = default value (when `:-default` syntax used).

Whitelist of recognised names (12 built-ins per FR-020):

| Name | Value |
|---|---|
| `SKILL_DIR` | Absolute path to the directory containing the entry's source file (parent of `.path`). |
| `SKILL_PATH` | Absolute path to the entry's source file. |
| `SKILL_NAME` | The entry's name (sanitised filename stem if no frontmatter `name`). |
| `PLUGIN_DIR` | Absolute path to the plugin's root directory. |
| `PLUGIN_NAME` | The plugin name (unsanitised — sanitisation applies only to path construction per FR-024). |
| `PLUGIN_VERSION` | The plugin's `version` from `plugin.json`. |
| `PLUGIN_DATA` | Absolute path `<home>/.tome/plugin-data/<catalog-sanitised>/<plugin-sanitised>/`; created lazily. |
| `CATALOG_NAME` | The catalog name (unsanitised). |
| `WORKSPACE_NAME` | The active workspace's name. |
| `WORKSPACE_DATA` | Absolute path `<home>/.tome/workspaces/<workspace>/plugin-data/<catalog-sanitised>/<plugin-sanitised>/`; created lazily. |
| `DATE` | Current date `YYYY-MM-DD` from `context.clock` (local time per the clock's offset). |
| `TIMESTAMP` | ISO 8601 timestamp from `context.clock` (with local offset). |

Unknown names in the Tome namespace (FR-023): the regex match is left in place verbatim; emit `tracing::debug!` recording the unknown reference's name.

Path components used in `PLUGIN_DATA` / `WORKSPACE_DATA` are sanitised per FR-024:
- Replace any character not in `[A-Za-z0-9._-]` with `_`.
- Sanitise applies ONLY when constructing the directory path. The `PLUGIN_NAME` / `CATALOG_NAME` built-ins return the unsanitised value.

`PLUGIN_DATA` and `WORKSPACE_DATA` directories: lazy-created on first reference within a single substitution pass via `create_dir_all`. Idempotent under concurrent retrievals (NFR-012). Failure surfaces `SubstitutionError::WorkspaceDataDirCreationFailed` (or `PluginDataDirCreationFailed`) → exit 25.

Default-value syntax (`${TOME_FOO:-default}`) per FR-022: for built-ins, the default never triggers in practice (all 12 are always set). The syntax is supported for uniformity with environment passthrough.

## Stage 2 — Environment passthrough

Regex (compiled once):
```
\$\{TOME_ENV_([A-Z0-9_]+)(?::-(.*?))?\}
```

Lookup: `std::env::var(format!("TOME_ENV_{}", name))` — the prefix is preserved when querying the host environment.

Behaviour per FR-030–FR-033:

| Host env state | Reference form | Resolved value |
|---|---|---|
| Set | `${TOME_ENV_FOO}` | The host env value |
| Set | `${TOME_ENV_FOO:-default}` | The host env value (default ignored) |
| Unset | `${TOME_ENV_FOO}` | Empty string; `tracing::debug!` recording the unset reference |
| Unset | `${TOME_ENV_FOO:-default}` | `default` |

References NOT in the `TOME_ENV_` namespace (e.g. `${GITHUB_TOKEN}`, `${AWS_SECRET_ACCESS_KEY}`, `${PATH}`) MUST NOT be matched by this stage's regex. They pass through unchanged (FR-033 + NFR-005).

## Stage 3 — Argument substitution

Regex (compiled once):
```
\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)
```

Patterns matched (in priority order — `regex::Regex` evaluates alternatives left-to-right):

| Pattern | Substitutes with | Notes |
|---|---|---|
| `$ARGUMENTS[N]` | Nth positional value (0-indexed); empty string if out of range | Highest priority — matched before bare `$ARGUMENTS` to avoid partial match. |
| `$ARGUMENTS` | All positional values joined by single space | Whole-string convention for single-string input per FR-042. |
| `$N` (single integer) | Same as `$ARGUMENTS[N]` | Per FR-040. |
| `$<name>` | Named argument value; empty if not provided | `<name>` matches `[a-z_][a-z0-9_]*` per FR-040. |

Caller's argument value coercion (R-10):

| Caller input | Entry declares named args? | Resolution |
|---|---|---|
| `Single("foo bar baz")` | Yes (e.g. `[a, b, c]`) | Shell-split: `a="foo"`, `b="bar"`, `c="baz"`. Positional `0=foo`, `1=bar`, `2=baz`. |
| `Single("foo bar baz")` | No | Whole-string single positional. `$ARGUMENTS = "foo bar baz"`. `$ARGUMENTS[0] = "foo bar baz"` (entire string). |
| `Object({a: "X", b: "Y", c: "Z"})` | Yes (e.g. `[a, b, c]`) | Named `a="X"`, etc. Positional `0="X"`, `1="Y"`, `2="Z"`. `$ARGUMENTS = "X Y Z"`. |
| `Object({a: "X"})` | Yes (e.g. `[a, b]`) | Named `a="X"`, `b=""`. Positional `0="X"`, `1=""`. |
| `Object({args: "foo bar baz"})` | No | Read the object's `args` key as a single string; resolve per the `Single("foo bar baz") + no named` row above. This is the MCP-prompts coercion case for the catch-all schema (FR-071). Object keys other than `args` for a no-named-args entry surface as `prompt_argument_mismatch` (exit 26). |
| `Object({other: "X"})` | No (or named-args with no matching declared name) | `prompt_argument_mismatch` (exit 26): caller supplied named keys that don't match any declared name. |
| `None` | (any) | Stage 3 skipped entirely. |

Shell-split rule (R-10): single string args are split via a simple shell-style quoting rule — whitespace separates tokens; single OR double quotes preserve internal whitespace; no nested quoting; no escape sequences. Implementation can reuse the `shell-words` algorithm without depending on the crate (the algorithm is ~30 lines of Rust). 

**Replacement-counting sentinel**: Stage 3 returns `(rendered_body, replacements_performed: bool)`. The boolean is set to `true` if any pattern matched and substituted a value (even if the substituted value was the empty string). It is `false` if no pattern matched.

Per NFR-007: substituted values are NOT re-scanned. A `$N`-substituted value that contains `${TOME_*}` is not subject to stage 1.

## Stage 4 — `ARGUMENTS:` append fallback

Runs only when caller supplied arguments AND stage 3 reported `replacements_performed = false`.

Append text appended to the END of the body (with a leading `\n\n` separator if the body did not end in `\n`):

```
ARGUMENTS: <value>
```

`<value>` is computed:

| Caller input | `<value>` |
|---|---|
| `Single("foo bar baz")` | `"foo bar baz"` (whole string, verbatim) |
| `Object({a: "X", b: "Y"})`, declared `[a, b]` | `"X Y"` (positional values joined by single space) |

Per FR-044.

## Error model

```rust
pub enum SubstitutionError {
    PluginDataDirCreationFailed { path: PathBuf, source: std::io::Error },  // → exit 25
    WorkspaceDataDirCreationFailed { path: PathBuf, source: std::io::Error }, // → exit 25
    InvalidArgumentFrontmatter { reason: String, file: PathBuf },             // → exit 23
    PromptArgumentMismatch { expected: usize, supplied: usize },              // → exit 26
}
```

Most substitution failures are best-effort (unknown references log + pass through, unset env vars empty-out). The four named error variants represent unrecoverable conditions where the rendering pass cannot continue safely.

## Pass-through semantics

References explicitly preserved verbatim:
- `${CLAUDE_*}` references (e.g. `${CLAUDE_SESSION_ID}`, `${CLAUDE_SKILL_DIR}`) — downstream Claude Code handles them.
- Other harness-native variable conventions (`${CURSOR_*}`, `${GEMINI_*}`, etc.) — same.
- Harness-native shell-execution syntax (`` !`cmd` `` at start of line or after whitespace; multi-line ` ```! ` fences) — Tome does NOT execute (Phase 6+).
- Unknown variables outside the Tome namespace — pass through.

The regex sets in stages 1–3 are constructed so that none of these forms match accidentally.

## Determinism

NFR-001 + R-16 contract:
- Same body + same `SubstitutionContext` (including `context.clock`) MUST produce the same rendered output byte-for-byte.
- Clock injection seam: `SUBSTITUTION_CLOCK_OVERRIDE` static (doc-hidden pub) sets a fixed `OffsetDateTime` for tests. Tests must use the `ClockOverrideGuard` RAII helper from `tests/common/mod.rs`.

## Memory bound

NFR-011: working memory is bounded by ≤ 2× body size (one input borrow, one output `String` per stage; final concatenation reuses the last stage's output). No full-body copy per stage; per-stage `replace_all` writes into a fresh `String` once.

## Concurrency

NFR-012: `create_dir_all` is concurrent-safe (kernel-atomic `mkdir`, `EEXIST` treated as success). The substitution layer does NOT take any lock around directory creation. Two concurrent `render()` calls against the same plugin can both lazy-create the same `${TOME_PLUGIN_DATA}` directory; both succeed.

## Call sites (FR-046)

| Surface | Stage 1 | Stage 2 | Stage 3 | Stage 4 |
|---|---|---|---|---|
| MCP `get_skill` | always | always | when args supplied | when args supplied + stage 3 made no replacements |
| MCP `prompts/get` | always | always | when args supplied | when args supplied + stage 3 made no replacements |
| MCP `search_skills` | — | — | — | — |
| MCP `get_skill_info` | — | — | — | — |

The CLI binary `tome plugin show` MUST NOT invoke the substitution layer either (FR-130 is metadata-only).

## Test pyramid

| Layer | Test file |
|---|---|
| Unit: each stage in isolation | `tests/substitution_builtins.rs`, `tests/substitution_env.rs`, `tests/substitution_arguments.rs` |
| Pipeline: stage ordering + once-pass invariant | `tests/substitution_pipeline.rs` |
| Data-dir creation: lazy + concurrent + rename relocation | `tests/substitution_data_dir.rs` |
| End-to-end: through `get_skill` and `prompts/get` | `tests/mcp_prompts.rs`, `tests/entry_e2e.rs` |
| Wire-shape pin: `SubstitutionError` JSON (when surfaced via CLI) | implied by exit-codes tests |
