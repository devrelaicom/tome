# Phase 5 — Entry frontmatter

The lenient YAML frontmatter format Phase 5 reads from plugin `skills/*/SKILL.md` and `commands/*.md` files. Authoritative shape for `src/plugin/frontmatter.rs`.

## Boundary

Per constitution IV (Strict Schemas, Helpful Errors) and the spec's reaffirmation in FR-007: third-party plugin frontmatter is parsed **leniently**. Unknown fields tolerated. Recognised fields with malformed values fail loudly with `InvalidArgumentFrontmatter` (exit 23) naming the file + field.

## Recognised fields (all optional)

| Field | YAML key | Type | Default | Source |
|---|---|---|---|---|
| `name` | `name` | string | sanitised filename stem | Existing (Phase 2) |
| `description` | `description` | string | first 500 chars of body | Existing (Phase 2) |
| `when_to_use` | `when_to_use` | string | (none) | NEW — Phase 5 |
| `arguments` | `arguments` | string or YAML list | (none — `Vec::new()`) | NEW — Phase 5 |
| `argument_hint` | `argument-hint` | string | (none) | NEW — Phase 5 |
| `disable_model_invocation` | `disable-model-invocation` | bool | false | NEW — Phase 5 |
| `user_invocable` | `user-invocable` | bool | `false` for skill, `true` for command | NEW — Phase 5 |
| `prompt_name` | `prompt_name` | string | (none) | NEW — Phase 5 |

YAML key conventions:
- `when_to_use` uses snake_case in YAML (matches Claude Code's documented convention).
- `prompt_name` uses snake_case in YAML (matches the PRD's documented spelling).
- The other new Phase 5 keys use kebab-case: `disable-model-invocation`, `user-invocable`, `argument-hint`.
- `name`, `description`, `arguments`: existing keys, no case change.

(Implementation hint: serde `#[serde(rename_all = "kebab-case")]` on the struct works for everything EXCEPT the two snake_case fields — `when_to_use` and `prompt_name` — each of which needs an explicit `#[serde(rename = "...")]` attribute to override the struct-level rule.)

## `arguments` accepts both string and list forms

Per Claude Code's documented behaviour, accepted shapes:

```yaml
# Form A: space-separated string
arguments: component from to

# Form B: YAML list
arguments:
  - component
  - from
  - to
```

Both produce the same in-memory `Vec<String> = ["component", "from", "to"]`. Deserialiser: a custom `deserialize_with` that accepts either form. Empty `arguments` (absent OR empty list OR empty string) produces `Vec::new()`.

Names within `arguments` MUST match `^[a-z_][a-z0-9_]*$`. Names with illegal characters fail parse with exit 23.

## Lenient unknown fields

Unknown frontmatter fields (e.g. Claude Code's `allowed-tools`, `agent`, `context`, `hooks`, `paths`, `model`, `effort`, `shell`) MUST be tolerated silently. Tome ignores them; future Tome versions or downstream tooling may consume them.

## Malformed recognised fields

Recognised fields with malformed values produce a parse error (exit 23 or 70 depending on Tome-owned vs third-party — third-party uses 23 per Phase 5 spec) naming the file + field:

```
Failed to parse frontmatter in /path/to/SKILL.md: field `arguments` must be a string or list, got integer
```

## Boolean parsing

`disable-model-invocation` and `user-invocable` accept YAML truthy/falsy: `true`, `false`, `yes`, `no`, `True`, `False`, `Yes`, `No`. `serde_yaml`'s default behaviour suffices.

## Resolved defaults

After parsing, two helper methods compute the effective values:

```rust
impl EntryFrontmatter {
    pub fn resolved_searchable(&self) -> bool {
        !self.disable_model_invocation.unwrap_or(false)
    }
    pub fn resolved_user_invocable(&self, kind: EntryKind) -> bool {
        self.user_invocable.unwrap_or(match kind {
            EntryKind::Skill => false,
            EntryKind::Command => true,
        })
    }
}
```

Defaults:
- `searchable`: true (`disable-model-invocation: false` resolves; absent default false).
- `user_invocable`: depends on kind — `false` for skills, `true` for commands.

## Body extraction

The frontmatter parser yields `(EntryFrontmatter, String)` where the second element is the body text (everything after the closing `---` line). The body is preserved verbatim — no whitespace stripping, no encoding conversion, no markdown normalisation. The substitution layer operates on this exact string.

## Fallback semantics

Per FR-006:

| Field absent in frontmatter | Fallback | Recorded |
|---|---|---|
| `name` | sanitised filename stem (no extension) | `tracing::debug!` log noting fallback |
| `description` | first 500 chars of body | `tracing::debug!` log noting fallback |
| `when_to_use` | (no fallback; NULL in DB) | — |

`description` fallback: if the body's first 500 chars contain a newline, truncate at the newline. If the body is empty or contains only whitespace, the description is the empty string.

## Tests

| Behaviour | Test |
|---|---|
| Each new field round-trips | `tests/frontmatter_p5_fields.rs::*` |
| `arguments` as string AND as list both parse to same `Vec<String>` | `tests/frontmatter_p5_fields.rs::arguments_string_or_list_both_parse` |
| Unknown fields tolerated | `tests/frontmatter_p5_fields.rs::unknown_field_does_not_fail` |
| Malformed `arguments` (integer) → exit 23 | `tests/frontmatter_p5_fields.rs::malformed_arguments_field_fails_loudly` |
| Illegal argument name (`1foo`) → exit 23 | `tests/frontmatter_p5_fields.rs::illegal_argument_name_fails` |
| `user_invocable` default depends on kind | `tests/frontmatter_p5_fields.rs::default_user_invocable_per_kind` |
| `description` fallback to first 500 chars | `tests/frontmatter_p5_fields.rs::description_fallback_to_body_prefix` |
| Existing Phase 4 frontmatter (no Phase 5 fields) parses without error | `tests/frontmatter_p5_fields.rs::backwards_compat_phase4_only_frontmatter` |
