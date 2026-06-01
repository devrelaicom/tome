# Contract: `harness/sync.rs` Decomposition (FR-011)

**FR**: FR-011 · **NFR**: NFR-005 · **SC**: SC-011 · **Research**: §R-10

Decompose the 1,737-LOC `src/harness/sync.rs` into a thin orchestrator + per-sink reconciler modules. **Strictly behaviour-preserving** — a *file move* of already-factored functions, not a redesign. Lands **first** (before FR-007/FR-008) so the harness fixes land in the clean structure.

## Module boundaries (the move map)

```
src/harness/
├── sync.rs                 # THIN orchestrator (stays)
│   ├── sync_project, SyncDeps, SyncOutcome, SyncChange, SyncSubsystem, HarnessDecision, Action
│   ├── HarnessSnapshot, collect_harness_snapshots, snapshot_for, group_by_path
│   ├── read_workspace_settings, read_global_settings, relative_path
│   └── rules/mcp helpers: compute_rules_body, write/clean_rules_for_path, classify_block,
│       write/clean_mcp_for_harness   (FR-008's OpenCode LCD body edit lands in compute_rules_body)
└── reconcile/
    ├── mod.rs              # re-exports + any shared reconcile type
    ├── hooks.rs            # HooksReconciliation, reconcile_hooks, compute_plugins_with_hooks_json,
    │                       #   merge_hooks_for_harness, remove_hooks_for_harness
    ├── guardrails.rs       # GuardrailsReconciliation, PreparedGuardrails, reconcile_guardrails,
    │                       #   guardrails_target_path, guardrails_action_to_action
    └── agents.rs           # AgentReconciliation, PreparedAgent, reconcile_agents, prepare_agent,
                            #   emit_agents_for_harness, cleanup_all_owned_agents, removed_disabled_owned,
                            #   all_owned_in_dir, AgentWrite, write_agent_file, record_action
```

`sync.rs` is **already internally factored** into the `reconcile_<sink>` shape (hooks ≈l.752, guardrails ≈l.1007, agents ≈l.1196, orchestrator l.187–447) — the move is mechanical.

## Preserved invariants (the behaviour-preservation contract, NFR-005)

1. **Fixed sink order** `first_clash → hooks → guardrails → agents`.
2. **`first_error` precedence** (hooks 43 > guardrails 46 > agents 45) when more than one sink fails.
3. **Mass-delete safeguard**: each reconcile fn opens the central DB read-only and **propagates** on an existing DB (never `.ok()`-swallow → empties the enabled set → mass-deletes every reconciled file). Carried into each module verbatim. *(The single biggest behaviour-preservation risk of the refactor.)*
4. **`SyncOutcome` byte-stable JSON wire-pin**: the `SyncSubsystem` arm order and the `<sink>_action`-appended-last field order are unchanged — the pin must not move a field.
5. **Idempotence**: re-sync rewrites no bytes (mtime-stable).
6. **Atomic writes + symlink refusal** at every sink (the FR-007 hardening rides on top, in the next slice).

## Behaviour-preservation evidence (NFR-005, SC-011)

The decomposition ships with these **pre-existing suites unchanged and green** — they ARE the evidence, not a new test:
- `tests/sync_idempotence.rs`, `tests/harness_sync_p6_idempotence.rs` (mtime-stable re-sync; `MTIME_TICK = 1500ms`).
- `tests/harness_sync_p6_first_error.rs` (multi-sink precedence: hooks+guardrails→hooks; guardrails+agents→guardrails; all-three→hooks).
- the `SyncOutcome` JSON-shape pin.
- the mass-delete-safeguard regression (assert a DB-open error aborts the sync rather than producing an empty enabled set — carry the existing one, or add it in this slice as the explicit guard).

SC-011: every pre-existing harness test passes with **no behavioural diff**.

## Sub-slice order (≤8 KB briefs, §R-22)

- **D.a** — scaffold `reconcile/mod.rs`; move `reconcile_agents` + its helpers → `reconcile/agents.rs` (the largest block).
- **D.b** — move `reconcile_guardrails` + helpers → `reconcile/guardrails.rs`.
- **D.c** — move `reconcile_hooks` + helpers → `reconcile/hooks.rs`; finalise the thin orchestrator + module docs.

Each sub-slice is independently gate-green with the evidence suites unchanged.

## Anti-requirements

- MUST NOT change behaviour (NFR-005) — no logic edits, only relocation + visibility adjustments (`pub(crate)` where a moved fn is now cross-module).
- MUST NOT fold in RUST-1/RUST-2 efficiency cleanups (out of scope; they would risk behaviour change).
- MUST NOT reorder `SyncSubsystem` arms or move a `SyncOutcome` field.
- MUST NOT weaken the mass-delete safeguard or the per-sink atomic-write/symlink discipline.
- `/sdd:map incremental` runs at this closeout (structural change → ARCHITECTURE + STRUCTURE diffs).
