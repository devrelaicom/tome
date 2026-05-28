# Phase 6 — Doctor extensions

Authoritative contract for the Phase 6 additions to `tome doctor` and `tome plugin show`. Per FR-083/090/091; data-model §7; PRD § "Doctor extensions". All new records are emit-only `Serialize` types with byte-stable JSON wire-shape pins (NFR-011); both human and JSON output paths covered.

## Phase 1–5 doctor surface — unchanged

Phase 5 ships `DoctorReport` with subsystems through Phase 4 (Embedder, Reranker, Index, Drift, Catalog, Schema, Summariser, Binding, BindingRulesCopy, HarnessRules, HarnessMcp) plus the Phase 5 `prompts` / `orphan_data_dirs` / `entry_counts` sections. All preserved verbatim. The Phase 5 `entry_counts` surface is updated for the widened `EntryKind` (FR-070a) — agent-kind rows are counted separately or explicitly excluded, never folded into a catch-all.

## Phase 6 surface additions (FR-090)

The existing `DoctorReport` gains five new fields, each emit-only `Serialize`:

```rust
pub struct DoctorReport {
    // ... existing Phase 1–5 fields ...
    pub hooks: Option<HooksReport>,
    pub guardrails: Option<GuardrailsReport>,
    pub agents: Option<AgentsReport>,
    pub privilege_escalation: Option<PrivilegeEscalationReport>,
    pub personas: Option<PersonaReport>,
}
```

`Option<>` follows the Phase 4/5 convention: populated only when the resolved scope is a known workspace; `None` under `GlobalFallback` / outside-project modes.

### `HooksReport` (Claude Code)

Per enabled plugin contributing JSON hooks: the hook entries Tome contributed to `.claude/settings.local.json`, and any plugin-derived hook entries Tome **expected but could not find** (drift from user edits — a re-derived entry with no structural match in the file).

```rust
pub struct HooksReport {
    pub plugins: Vec<HookPluginEntry>,   // { catalog, plugin, contributed: Vec<HookEventEntry>, missing: Vec<HookEventEntry> }
}
```

### `GuardrailsReport`

Per target file: the present `<catalog>:<plugin>` regions, orphaned regions (plugin no longer enabled / harness gone), and regions suppressed by JSON hooks on Claude Code.

```rust
pub struct GuardrailsReport {
    pub files: Vec<GuardrailsFileEntry>, // { path, present: Vec<CatalogPlugin>, orphaned: Vec<CatalogPlugin>, suppressed: Vec<CatalogPlugin> }
}
```

### `AgentsReport`

Per harness: present and orphaned `<plugin>__*` namespaced agent files, fields dropped during translation (informational), and the privilege-escalation grouping (cross-referenced from `PrivilegeEscalationReport`).

```rust
pub struct AgentsReport {
    pub harnesses: Vec<AgentHarnessEntry>, // { harness, present: Vec<String>, orphaned: Vec<String>, dropped_fields: Vec<DroppedFieldEntry> }
}
```

### `PrivilegeEscalationReport` (FR-051)

Installed agents carrying any of `hooks` / `mcpServers` / `permissionMode`, **grouped by plugin**, so the escalation surface is auditable regardless of the `strip_plugin_agent_privileges` setting's value (`settings-p6.md`).

```rust
pub struct PrivilegeEscalationReport {
    pub plugins: Vec<PrivilegePluginEntry>, // { catalog, plugin, agents: Vec<PrivilegeAgentEntry { name, fields: Vec<String> }> }
}
```

### `PersonaReport`

Populated only when `expose_agents_as_personas` resolves true at the doctor scope. The effective persona prompt list with resolved names and any clash-prefixed names (`agent-personas.md`).

```rust
pub struct PersonaReport {
    pub personas: Vec<PersonaEntry>, // { catalog, plugin, agent_name, resolved_persona_name, clash_prefixed: bool }
    pub drop_persona: String,        // always "drop-persona"
}
```

## `--fix` (FR-091)

Repair mode repairs **only the safe, derivable cases**:

- re-render stale guardrails regions (overwrite-between-markers in place);
- re-emit missing agent files (re-translate from source);
- remove orphaned `<plugin>__*` agent files.

It **NEVER**:

- removes a hook entry from `.claude/settings.local.json` that does not **exactly** match a re-derived plugin entry (a user-edited hook is left in place — ownership is proven by structural re-derivation, NFR-003);
- deletes any user-authored content (rules-file text outside Tome markers, hand-written agents not matching the `<plugin>__*` glob).

Hooks drift (entries Tome expected but could not find) is **reported but not auto-fixed** — re-merging on the next sync/enable is the remediation path. Phase 5 repair classes (Summariser, BindingRulesCopy, HarnessRules, HarnessMcp, Schema) continue to work.

## `tome plugin show` extensions (FR-083)

`tome plugin show <catalog>/<plugin>` additionally lists:

- the plugin's **agents** (one per `agents/*.md` source, with the resolved displayed name);
- whether the plugin ships **`hooks/hooks.json`** and/or **`hooks/GUARDRAILS.md`** (presence booleans);
- for agents, when `expose_agents_as_personas` resolves true, the **resolved persona name** per agent.

These are read-only filesystem + index reads; no writes.

## Read-only enforcement

Consistent with Phase 5 (FR-124): doctor's Phase 6 surfaces are read-only by default. Hooks/guardrails/agent inspection is `fs::read`/`read_dir` only; persona enumeration derives names from frontmatter + entry rows without invoking the substitution layer or creating any data directory. `--fix` is the only write path and is gated on the explicit flag.

## Exit codes

Doctor's existing exit-code semantics are preserved (`0` healthy / `1` degraded / `75` `--fix` ran but unfixable issues remain). The Phase 6 surfaces are **informational** and do not by themselves trigger `degraded`: hooks drift, orphaned guardrails/agents, dropped fields, and the privilege-escalation report are reported, not classified as failure. The four Phase 6 failure exit codes (43–46, per `exit-codes-p6.md`) are raised by the sync/translation paths, not by doctor's read-only inspection.

## Tests

| Behaviour | Test |
|---|---|
| Hooks report: contributed + missing (drift) | `tests/doctor_p6.rs::hooks_report_contributed_and_drift` |
| Guardrails report: present/orphaned/suppressed per file | `tests/doctor_p6.rs::guardrails_report_present_orphan_suppressed` |
| Agents report: present/orphaned + dropped fields | `tests/doctor_p6.rs::agents_report_present_orphan_dropped` |
| Privilege-escalation report grouped by plugin | `tests/doctor_p6.rs::privilege_report_grouped_by_plugin` |
| Persona report present when personas on, absent off | `tests/doctor_p6.rs::persona_report_only_when_enabled` |
| `--fix` re-renders stale guardrails | `tests/doctor_p6.rs::fix_rerenders_stale_guardrails` |
| `--fix` re-emits missing agents + removes orphans | `tests/doctor_p6.rs::fix_reemits_and_removes_orphan_agents` |
| `--fix` never removes a non-matching hook entry | `tests/doctor_p6.rs::fix_never_removes_unowned_hook` |
| `--fix` never deletes user-authored content | `tests/doctor_p6.rs::fix_never_deletes_user_content` |
| `plugin show` lists agents + hooks/guardrails presence + persona name | `tests/plugin_show_p6.rs::shows_agents_hooks_guardrails_personas` |
| Doctor Phase 6 surfaces create no dirs | `tests/doctor_p6.rs::phase6_surface_creates_no_dirs` |
| Outside-project: Phase 6 fields are None | `tests/doctor_p6.rs::outside_project_phase6_fields_none` |
| Byte-stable JSON wire pins (all five records) | `tests/doctor_p6_json_shape.rs` |
| `plugin show` JSON wire pin | `tests/plugin_show_p6_json_shape.rs` |
