# Phase 6 Research — Hooks and Agents

**Branch**: `006-phase-6-hooks-agents` | **Date**: 2026-05-28
**Input**: [spec.md](./spec.md), [PRDs/phase-6.md](../../PRDs/phase-6.md)

Most Phase 6 design questions are settled by the PRD's "Resolved decisions" table and the Rust-lens spec review folded into spec.md. This file records each decision in the standard Decision / Rationale / Alternatives form, plus the choices informed by the Phase 4 (P3) and Phase 5 (P7/P8) retros. There are no open `NEEDS CLARIFICATION` items.

---

### R-1 — Exit-code cluster for the four new failure classes

- **Decision**: Claim the contiguous run **43, 44, 45, 46** (43 malformed `hooks.json`; 44 hook settings-file read/merge/write failure; 45 agent frontmatter malformed / translation failed; 46 guardrails render/write failed).
- **Rationale**: The occupied set is 1–9, 13–37, 40–42, 50–54, 60–61, 70, 73–75. The PRD-proposed 30–33 collide with the model-on-disk cluster and 34–37 are the inference/vector cluster. 43–49 is the first free contiguous run large enough for four pairwise-unique codes; 43–46 leaves 47–49 free for future use. Same reassignment precedent as the Phase 4 summariser code (proposed 20 → shipped 24) and the Phase 5 cluster (proposed 21–23 → shipped 25–29).
- **Alternatives**: 38–39 (only two free, insufficient); 55–59 or 62–69 (further from the existing clusters, no advantage); reusing 30–33 (rejected — would silently change shipped exit-code meanings, violating constitution §II NON-NEGOTIABLE).

### R-2 — `HarnessModule` trait extensions

- **Decision**: Extend the existing trait (Phase 4) with: `hooks_strategy() -> HooksStrategy` (`RealJson` for claude-code, `GuardrailsOnly` otherwise); `hook_settings_path(project) -> Option<PathBuf>` (`.claude/settings.local.json` for claude-code, `None` otherwise); `guardrails_target(project) -> GuardrailsTarget` (in-file region vs standalone sibling + the Claude-Code-only suppression predicate); `supports_native_agents() -> bool` (true for the four); `agent_dir(project) -> Option<PathBuf>`; `agent_format() -> Option<AgentFormat>` (`MarkdownYaml` | `Toml`); `translate_agent(canonical) -> TranslatedAgent`. Default impls make a new harness `GuardrailsOnly` + no-native-agents.
- **Rationale**: Mirrors how Phase 4 grew the trait (rules-file + mcp-config capabilities) without restructuring. Default impls keep the five existing modules compiling through F3 and make future harnesses safe-by-default. `detect_path(&home)` already exists (Phase 4 PR-C).
- **Alternatives**: Separate `HookProvider`/`AgentProvider` traits (rejected — splits per-harness behaviour across multiple impls for no benefit; the harness module is already the natural home).

### R-3 — Real-hooks merge mechanism

- **Decision**: Parse the plugin's `hooks/hooks.json` and the project's `.claude/settings.local.json` with `serde_json` (`preserve_order` feature, already enabled since Phase 4). Merge by **deep structural equality** of the post-rewrite hook entry under its event key: append if no structurally identical entry exists, skip otherwise; remove the structurally identical entry on teardown, skip if absent. Prune empty event arrays; leave an otherwise-empty `hooks` object.
- **Rationale**: `serde_json::Value` gives `PartialEq` deep equality for free; `preserve_order` keeps the file diff-stable across syncs (NFR-001 idempotence). No sidecar — ownership is re-derived (FR-005, NFR-003), matching the Phase 4 filesystem-inferred model.
- **Alternatives**: A sidecar manifest of Tome-written hooks (rejected — violates the no-sidecar model and would drift from hand edits); `toml_edit`-style surgical JSON (rejected — `serde_json` round-trips JSON natively; `toml_edit` is for TOML).

### R-4 — Hook path-variable rewriting

- **Decision**: A targeted **two-variable textual substitution** over string values of the hook JSON: `${CLAUDE_PLUGIN_ROOT}` → absolute installed-plugin root, `${CLAUDE_PLUGIN_DATA}` → the Phase 5 `${TOME_PLUGIN_DATA}` path. Every other `${CLAUDE_*}` is left verbatim. Implemented with a small `regex` replace over string leaves, NOT the Phase 5 substitution pipeline.
- **Rationale**: Claude Code resolves `${CLAUDE_PROJECT_DIR}`/`${CLAUDE_SESSION_ID}` natively at runtime; rewriting them would break them. The two plugin-root variables have no runtime resolver once the hook is lifted into project settings (Tome installs to its own cache, not via Claude Code's plugin manager). Reusing the full substitution pipeline would over-substitute and pull in argument/built-in semantics that don't belong in a config file (FR-003).
- **Alternatives**: Full Phase 5 substitution (rejected — over-broad, wrong namespace); leaving `${CLAUDE_PLUGIN_ROOT}` for Claude Code (rejected — it can't resolve it for a Tome-installed plugin).

### R-5 — Guardrails marker regions + per-file reconciliation

- **Decision**: A new `src/harness/guardrails.rs` renders each enabled-plugin-with-`GUARDRAILS.md` into a per-plugin marker region using the literal `<!-- START GUARDRAILS: <catalog>:<plugin> -->` … `<!-- END GUARDRAILS: <catalog>:<plugin> -->` (distinct from the Phase 4 `tome:begin/end` rules block). Reconciliation is per target file: deterministic placement (rules-include block first, then regions in lexicographic `<catalog>:<plugin>` order), overwrite-between-markers in place, remove orphaned regions, delete the Cursor sibling when empty. The region-find/replace reuses the `rules_file.rs` block machinery generalised to a parameterised marker pair.
- **Rationale**: Marker pairs = filesystem-inferred state, no sidecar (NFR-004). Deterministic ordering keeps re-syncs from reordering (idempotence, FR-011/NFR-001). Distinct markers avoid colliding with the Phase 4 rules block on the same file.
- **Alternatives**: One combined Tome-managed block holding all guardrails (rejected — breaks per-plugin removal); reusing the exact `tome:begin/end` markers (rejected — conflates two distinct managed regions on one file).

### R-6 — Claude Code rules-file correction (the Phase 4 fix)

- **Decision**: Change `claude_code`'s rules-file candidate list to `CLAUDE.md` > `.claude/CLAUDE.md` (first existing wins; create `CLAUDE.md` if none), dropping `AGENTS.md` entirely from Claude Code's set. Codex/Gemini/OpenCode keep sharing one `AGENTS.md` block. Both include directives resolve the same `.tome/RULES.md`. No `@AGENTS.md` scaffolding.
- **Rationale**: Claude Code does not natively read `AGENTS.md` (the feature request is open/unshipped); the Phase 4 table listed `AGENTS.md > CLAUDE.md`, so a project with `AGENTS.md` and no `CLAUDE.md` had its rules block invisible to Claude Code (FR-020/021/022). Both pointers resolving one `RULES.md` means no content duplication (NFR-009).
- **Alternatives**: Scaffold `CLAUDE.md` with `@AGENTS.md` import (out of scope per PRD — adds a transitive chain and an opinionated file the user didn't ask for); a transitive `CLAUDE.md → @AGENTS.md → @RULES.md` (rejected — depends on import semantics and is fragile).

### R-7 — Agent field/value translation

- **Decision**: Per-harness `translate_agent` passes through only the fields the target supports or that map cleanly, dropping the rest (FR-032). The body goes to the file body (MD+YAML harnesses) or a triple-quoted `developer_instructions` TOML string (Codex, FR-033). `model` mapping is **same-vendor only**, driven by a per-harness alias table (R-8 below); unmapped values are dropped (FR-034). Read-only intent is inferred from the source tool posture by a documented rule and expressed in each harness's mechanism, dropped where indeterminate (FR-036). OpenCode defaults `mode: subagent` and falls back to the first non-empty body line for the required description (FR-035).
- **Rationale**: Matches the PRD §2.1 field table and the spec's tightened FR set. Dropping over guessing keeps Tome from emitting values a harness can't interpret (NFR-005).
- **Alternatives**: Pass unknown fields through assuming tolerance (rejected — a stray key can break a harness parser); cross-vendor "strongest-to-strongest" model mapping (rejected by the PRD — rots and surprises users).

### R-8 — Model-alias table as a single source of truth

- **Decision**: A per-harness, same-vendor `model` alias table declared in the harness modules and pinned in `contracts/agent-translation.md` (e.g. `opus` → OpenCode `anthropic/claude-opus-4.7`; `opus` → Codex dropped; `inherit` → dropped everywhere). It is the named artefact SC-002 verifies against. Exact identifiers confirmed against current harness docs at implementation time (Phase 4 ecosystem caveat).
- **Rationale**: FR-037 + SC-002 need a concrete, testable mapping. A declared table makes the success criterion verifiable and centralises the same-vendor policy.
- **Alternatives**: Inline per-call mapping logic (rejected — un-auditable, can't pin a wire test against it).

### R-9 — Agent file naming, provenance, removal

- **Decision**: Filenames always `<plugin>__<name>.<ext>` (sole provenance; no frontmatter provenance key). Displayed/registered name is clean `<name>`, plugin-prefixed `<plugin>-<name>` only on cross-plugin clash (FR-041); OpenCode's name is filename-derived so always prefixed (FR-042). Removal globs `<plugin>__*.<ext>` per agent dir (FR-043). The clash set is the workspace-enabled agent rows, computed once per sync (FR-072).
- **Rationale**: A filename convention can't break a harness parser the way an unknown frontmatter key could; the glob is a clean removal key. Matches PRD §2.2.
- **Alternatives**: A `tome_plugin:` frontmatter key (rejected by the PRD — parser-break risk outweighs the rare filename-collision risk).

### R-10 — Personas reuse the Phase 5 prompt + substitution machinery

- **Decision**: A specialised persona path in `src/mcp/prompts.rs` builds `<name>-persona` prompts (frontmatter stripped, body wrapped in the role-assumption template, Phase 5 built-in + env substitution applied, single catch-all `args` through the Phase 5 argument pipeline) plus one global reserved `drop-persona`. Persona derived names join the **single** Phase 5 prompt-name collision namespace (FR-066). Off by default; the toggle is resolved against the MCP server's startup scope (FR-067).
- **Rationale**: `build_context_for_entry` (Phase 5 Polish M-2) is keyed on entry path / `.claude-plugin` ancestor walk and works identically for an agent `.md`; reuse is genuine, satisfying NFR-007's "no parallel substitution path". Sharing the collision namespace prevents two prompts registering the same slash name.
- **Alternatives**: A parallel persona substitution path (rejected — NFR-007); a separate persona collision namespace (rejected — slash names share one MCP namespace; FR-066).

### R-11 — Agent indexing + schema/enum widening

- **Decision**: Widen the Rust `EntryKind` enum (`src/plugin/identity.rs`) with an `Agent` variant and update every exhaustive match (per-kind counts in `plugin list/show` + doctor) — the load-bearing F2 change (FR-070a). At the storage layer the `kind` column is free-text TEXT, so no DDL/data migration is required; **register a marker-only migration to bump the schema version** so doctor's schema check and the migration registry agree the domain widened. Agent rows: `searchable = 0`, embedding skipped, reuse the `(catalog, plugin, kind, name)` constraint.
- **Rationale**: The first `kind='agent'` row would otherwise crash the existing closed-`EntryKind` matches (the spec's B1 blocker). A marker migration follows the Phase 4/5 framework pattern (Phase 3 shipped the framework; Phase 4 registered the first real migration) and keeps the schema version monotonic; it is cheap and makes the widening auditable.
- **Alternatives**: No migration at all (defensible since the column is free-text, but leaves the schema version not reflecting a real domain change — chose the marker for auditability); a new `agents` table (rejected — FR-071, no new tables; the entry table already models kind).

### R-12 — Two new config settings + scalar layering

- **Decision**: Add `expose_agents_as_personas: bool` and `strip_plugin_agent_privileges: bool` (both default `false`) as typed fields on the Tome-owned `GlobalSettings`/`WorkspaceSettings`/`ProjectMarkerConfig` structs (strict, `deny_unknown_fields`). Resolve them by a **first-declarer-wins priority walk** over project → workspace → global — NOT the `harnesses` composition reference/exclusion grammar. The persona toggle's *effective* value for a running MCP server is read from the server's single startup scope (FR-067); the strip setting is resolved at agent-emission (sync) time.
- **Rationale**: They are plain booleans, not lists; the composition grammar (references/exclusions/cycles) is meaningless for a scalar. First-declarer-wins gives the expected "project overrides global" semantics (FR-053, the spec's M2 fix).
- **Alternatives**: Full composition semantics for the scalars (rejected — over-engineered, no list to compose); "set anywhere ⇒ true" (rejected — can't express a project turning a global default off).

### R-13 — Cross-sink sync ordering + forward progress

- **Decision**: Within one harness sync, reconcile in the fixed order **hooks → guardrails → agents**; the hooks-presence determination is computed before guardrails so the Claude Code suppression predicate never reads stale state (FR-016). A failure in one sink does not roll back already-reconciled sinks; sync continues across remaining harnesses/sinks and surfaces the first failure's exit code (FR-084) — the Phase 4 binding-then-sync forward-progress discipline (FR-403). Each file write stays atomic.
- **Rationale**: The suppression predicate depends on hooks state, forcing the order. Forward progress matches the Phase 4 contract and the P3 retro's `HarnessClash` handling.
- **Alternatives**: All-or-nothing transactional sync across sinks (rejected — Phase 4 explicitly chose forward-progress; a mid-sync failure shouldn't undo correct work).

### R-14 — Codex TOML agent emission

- **Decision**: Emit Codex agents with `toml_edit` (existing dep), placing the Markdown body in a triple-quoted `developer_instructions` string and the surviving frontmatter keys as TOML keys.
- **Rationale**: `toml_edit` already powers Phase 4's `mcp_config.rs` TOML branch and Phase 4/5 settings edits; it gives comment/order-preserving output and correct triple-quote escaping. No new dep.
- **Alternatives**: Hand-rolled TOML string building (rejected — escaping triple-quoted strings correctly is exactly what `toml_edit` is for).

### R-15 — No new top-level dependencies

- **Decision**: Phase 6 adds zero new top-level crates. JSON merge → `serde_json`; Codex TOML → `toml_edit`; markers/rewrite → `regex`; personas → `rmcp` + the Phase 5 substitution module.
- **Rationale**: Every capability maps onto an existing dep. Keeps the binary-size budget flat (~23 MiB margin) and avoids a constitution amendment (§Dependencies, §Complexity budget). Phase 5 P8 lesson: phase-level dependency additions force amendments; aim for none.
- **Alternatives**: A JSON-merge crate or a YAML-frontmatter agent crate (rejected — `serde_json`/`serde_yaml` already cover it; the Rule of Three doesn't justify a new dep for a single merge).

### R-16 — Test-injection seams

- **Decision**: Reuse `HARNESS_MODULES_OVERRIDE` + `HarnessModulesGuard` (Phase 4) for synthetic-harness tests of the new trait methods via `StubHarness`. Persona tests drive the prompt registry against the workspace scope (Phase 5 pattern). The two new bool settings are exercised by writing real settings files in a `tempdir` workspace (no override slot needed — they're plain config). Idempotence tests follow the `MTIME_TICK = 1500ms` capture/sleep/re-run pattern (Phase 4 P3).
- **Rationale**: Reusing the established seams avoids new injection machinery. `StubHarness` already exercises the dispatch pipeline without the five real modules.
- **Alternatives**: A new config-override slot for the bools (rejected — config files in a tempdir workspace are the real path and need no seam).

### R-17 — Slice shape (pre-emptive)

- **Decision**: F1 (error codes) → F2 (EntryKind widening, load-bearing, first) → F3 (trait + StubHarness skeleton) → US1 (agents, 5 slices) → US2 (hooks, 3) → US3 (guardrails + correction, 3) → US4 (personas, 3) → US5 (privilege + doctor, 3) → Polish. Detailed in plan.md § Pre-emptive slice plan.
- **Rationale**: F2 first means no slice can introduce a crashing `kind='agent'` row (the B1 blocker). US1 (agents, P1) is the largest portable win and is independently demonstrable. Keeps each agent brief ≤ 8 KB (Phase 4 F11b / P3 lesson).
- **Alternatives**: Hooks before agents (rejected — agents are P1; hooks touch one harness, agents four).

### R-18 — Per-US + phase-wide reviewer discipline + JSON wire-shape pins

- **Decision**: Each user-story closeout runs the 4-reviewer parallel pass (contract / Rust-lens / test / security) as ONE message; findings + disposition committed before fixes; `/sdd:map incremental` at each closeout. The Polish phase runs a phase-wide pass even when per-US passes were thorough. Every new emit-only type gets a byte-stable JSON wire-shape pin (NFR-011).
- **Rationale**: Phase 5 P8: the phase-wide pass catches cross-US drift per-slice passes can't (e.g. a pattern fixed late in one US not propagated to an earlier one). JSON pins caught real shape regressions in Phases 3–5.
- **Alternatives**: Skip the phase-wide pass when per-US passes are clean (rejected — Phase 5 P8 explicitly found the phase-wide pass adds value regardless).

### R-19 — Single-source-of-truth promotion at the second consumer

- **Decision**: Apply the Phase 5 canonical-accessor rule to Phase 6's new shared shapes: promote to a `pub(crate)` accessor at the second consumer rather than duplicating. Candidates: the guardrails marker literal + region find/replace (reused from `rules_file.rs`), the `<plugin>__<name>` agent-filename builder, the clash-set query, the model-alias lookup, and `build_context_for_entry` (already promoted in Phase 5, reused by personas).
- **Rationale**: Phase 5 racked up 5 such promotions; the discipline is now firm. Prevents the truncate_description-style pattern drift the Phase 5 Polish pass caught.
- **Alternatives**: Duplicate-now-refactor-later (rejected past the second consumer by the established rule).

### R-20 — Deferred-item disposition carried into Phase 6

- **Decision**: From the Phase 5 P8 backlog, evaluate **cap-std hardening** as the Polish-phase security slice (it moots the MEDIUM symlink path disclosure + the `walk_resources` TOCTOU residual and applies to Phase 6's new file sinks). The **read-only DB open refactor** (Phase 3 backlog) is touched only if an agent-indexing read path needs it. Items explicitly NOT in Phase 6 (PRD non-goals): real hooks for Codex/Gemini, native agents for Gemini/Antigravity, semantic search over agents, server-side shell exec, hook-authoring tooling, persistent-data lifecycle/cleanup, new harnesses.
- **Rationale**: Phase 6 introduces four new write sinks; cap-std hardening is the right time to apply `openat`/`O_NOFOLLOW` across all file sinks at once (Phase 5 P8 "obvious v0.6+ first slice").
- **Alternatives**: Defer cap-std again (acceptable, but Phase 6's new sinks make it more valuable now than later).
