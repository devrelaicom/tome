# Phase 6 — Agent personas (MCP-prompt fallback)

Authoritative contract for exposing enabled agents as MCP-prompt "personas" through the Phase 5 prompts capability. Per FR-060–FR-067; research R-10/R-12; PRD §2.4. Reuses the Phase 5 prompt + substitution machinery unchanged (NFR-007); no parallel substitution path.

## Opt-in and scope

- Enabled by the `expose_agents_as_personas: bool` setting (default `false`) — see `settings-p6.md` for the struct + layering. With it off (the Phase 5 default), no persona prompt and no `drop-persona` prompt appear in `prompts/list`; the prompt surface behaves exactly as in Phase 5 (NFR-008, SC-008).
- **Global exposure**: because MCP prompts are not harness-scoped, enabling the setting exposes personas to **all** connected clients — including the four that already get native agent files. The redundancy is harmless and is why the feature is global and off by default (FR-060).
- **Effective value is read from the MCP server's single startup scope** (the resolved workspace, or global fallback) — the same scope the server binds its prompt registry to (FR-067). The running MCP server is not project-bound, so **project-scope layering of `expose_agents_as_personas` has no effect on a running server**; this MUST be documented. The on-disk first-declarer-wins resolution (`settings-p6.md`) governs *which* value a given scope produces; the persona toggle's effective value for a session is whatever the server's startup scope resolves.

## Persona name resolution (FR-061)

- A persona prompt's name is `<name>-persona` normally, and `<plugin>-<name>-persona` **only** for agents whose `<name>` clashes across two or more enabled plugins (the FR-072 clash set; see `agent-translation.md`), applied to the clashing agents only.
- `<name>` is taken from the agent's frontmatter `name` field, **read before the frontmatter is stripped** during conversion, and falls back to the agent Markdown filename stem when absent.
- Persona names are subject to the Phase 5 sanitisation, length-limit, and collision-counter rules as a **final backstop** (`mcp-prompts.md` § Sanitisation). The agent-clash plugin prefix is applied **before** the Phase 5 counter-suffix backstop (FR-066), so the backstop only fires on residual collisions the prefix did not resolve.

Users invoke `/mcp__tome__<name>-persona <prompt text>` (or `/mcp__tome__<plugin>-<name>-persona …` on clash). Tome contributes the `<name>-persona` portion; the harness prepends `mcp__tome__`.

## Persona prompt body (FR-062)

On `prompts/get` for a persona:

1. Strip the agent's frontmatter.
2. Wrap the body in the role-assumption template (reproduced verbatim from PRD §2.4):

```
Assume the following <Name> persona until instructed otherwise.

<<name>-persona>
<agent markdown body, with Phase 5 substitution applied>
</<name>-persona>

While acting as the <Name> persona, you must: $ARGUMENTS
```

3. Apply Phase 5 **built-in + environment substitution** to the body (the `${TOME_*}` / `${TOME_ENV_*}` sweep, via the shared `build_context_for_entry`, which is keyed on the entry path / `.claude-plugin` ancestor walk and works identically for an agent `.md` — R-10).
4. Resolve a **single catch-all free-form argument** (`args`) through the **exact Phase 5 argument pipeline**, including the documented `ARGUMENTS:` append fallback when the template consumes no argument reference. `$ARGUMENTS` in the template is resolved by that pipeline.

**No parallel substitution path may be introduced** (NFR-007). Caller-supplied persona arguments are treated as opaque strings exactly as Phase 5 treats prompt arguments. The persona's prompt-schema argument is the single catch-all `args` (Case B in `mcp-prompts.md` § Argument schema derivation): `{ "name": "args", "required": false }`.

## `drop-persona` (FR-063)

A single **global, unnamespaced, RESERVED** prompt, exposed exactly **once** when `expose_agents_as_personas` is on (not per plugin), regardless of agent count. Its body is reproduced verbatim from PRD §2.4:

```
Stop acting as any assumed persona and return to your default behaviour
and personality.
```

The name `drop-persona` is **reserved**: if any command, skill, or persona would derive to `drop-persona`, the **other** entry is counter-suffixed (via the Phase 5 counter-suffix backstop) and `drop-persona` remains unnamespaced and unique.

## Collision namespace (FR-066)

Persona derived names join the **single** Phase 5 prompt-name collision namespace — the collision-resolution pass runs over the **union** of command, skill, and persona derived names (not a separate persona namespace; slash names share one MCP namespace). Order of precedence within that pass:

1. The agent-clash plugin prefix (FR-061) is applied first.
2. The Phase 5 counter-suffix backstop fires only on residual collisions the prefix did not resolve.
3. The `drop-persona` reservation (above) wins over any other entry deriving to that name.

## Emission path (FR-064)

Persona prompts are emitted by the MCP server's `prompts/list` and `prompts/get` layer **from agent rows** (`kind = 'agent'`) as a **specialised path parallel to** — not folded into — Phase 5's command/skill prompts:

- `prompts/list`: when the toggle is on, the persona path appends one `<name>-persona` entry per enabled agent (description truncated per the Phase 5 300-char `prompts/list` cap) plus the single `drop-persona` entry, into the same response, after collision resolution over the union namespace.
- `prompts/get`: a `*-persona` name (or `drop-persona`) is resolved by the persona path (template-wrapped, frontmatter-stripped body); all other names resolve through the Phase 5 command/skill path unchanged.

The persona path does not alter the Phase 5 capability declaration, the `prompts/list` envelope shape, or the `prompts/get` single-`user`-message envelope (`mcp-prompts.md`). Persona prompt entries are pinned by byte-stable JSON wire-shape tests (NFR-011).

## Advisory-state caveat (FR-065)

User-facing documentation MUST state plainly that a persona is **advisory conversational context, not enforced configuration** — the agent can drift or ignore it, exactly like guardrails. Personas MUST NOT be presented as the isolation or tool-scoping a native subagent provides.

## Tests

| Behaviour | Test |
|---|---|
| Personas off (default) → no persona/drop-persona in list | `tests/persona_prompts.rs::off_by_default_no_personas` |
| Personas on → `<name>-persona` per agent + one `drop-persona` | `tests/persona_prompts.rs::on_exposes_personas_and_drop` |
| Clashing agents only get `<plugin>-<name>-persona` | `tests/persona_prompts.rs::clash_prefix_only_on_clash` |
| Persona body: frontmatter stripped, template-wrapped, substituted | `tests/persona_prompts.rs::get_wraps_and_substitutes` |
| Persona name from frontmatter, else filename stem | `tests/persona_prompts.rs::name_from_frontmatter_else_stem` |
| `drop-persona` reserved → other entry counter-suffixed | `tests/persona_prompts.rs::drop_persona_reserved` |
| Persona toggle resolved against server startup scope | `tests/persona_prompts.rs::toggle_from_startup_scope` |
| Byte-stable JSON wire pin (persona list + get) | `tests/persona_prompts_json_shape.rs` |
