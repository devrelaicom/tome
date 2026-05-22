# Doctor Extensions — Phase 4

**Spec source**: [spec.md FR-560 through FR-564](../spec.md)

Phase 3's `tome doctor` is extended with new subsystems covering binding state, harness integration, and the summariser. The Phase 3 surface and exit-code semantics (0 / 1 / 75) carry forward unchanged.

## New subsystems

In addition to Phase 3's `Embedder`, `Reranker`, `Index`, `Drift`, `Catalog(<name>)`, `Schema`, `Harness` (informational), Phase 4 adds:

| Subsystem | Health classification | What it checks |
|-----------|------------------------|----------------|
| `Summariser` | Ok / Broken (missing) / Drifted (checksum mismatch) | Same shape as `Embedder` / `Reranker`. Verified via `embedding::download::sha256_file` only when `--verify` is passed. |
| `Binding` | Ok / Broken (orphan: marker names missing workspace) | Reads `<project>/.tome/config.toml`; checks `workspaces` table for the named workspace. Skipped when outside any project marker. |
| `BindingRulesCopy` | Ok / Drifted (content mismatch) / Broken (file missing) | Reads `<project>/.tome/RULES.md` and `<root>/workspaces/<name>/RULES.md`; byte-compare. Skipped when `Binding` is broken. |
| `HarnessRules(<name>)` per harness in effective list | Ok / Drifted / Broken | For each harness in the effective list (computed via the layered settings walk), check the rules-file target: is the Tome block present? Is its body current (matches the project marker's RULES.md content, or is the `@`-include pointing at it)? Standalone-file harnesses: does the file exist and match? |
| `HarnessMcp(<name>)` per harness in effective list | Ok / Drifted / Broken / UserOwned | For each harness, parse the harness's MCP config file. Check: is the `tome` key present? Is it Tome-owned (`command == "tome"`, `args[0] == "mcp"`)? Does `--workspace` arg match the bound workspace's name? |

## Subsystem enum promotion

Per [research.md R-15](../research.md) and the P6 retro recommendation (promote at >6 arms), Phase 3's `subsystem: String` field on `SuggestedFix` is replaced by the typed enum `Subsystem` (see [data-model.md §15](../data-model.md)). The dispatch ladder in `fixes::apply` becomes a `match` on the enum.

## Effective-harness-list snapshot

When in a bound project, the doctor report includes:

```rust
pub struct DoctorReport {
    // ... existing fields ...
    pub project_binding: Option<ProjectBindingState>,
    pub effective_harness_list: Option<EffectiveHarnessList>,
    pub harness_rules: Vec<(String, SubsystemHealth)>,
    pub harness_mcp: Vec<(String, SubsystemHealth)>,
    pub detected_uninstalled_harnesses: Vec<String>,
    // ...
}
```

`detected_uninstalled_harnesses`: harnesses whose per-user directory exists on the local machine but who are NOT in the effective list. Reported informationally; never affects classification.

## Fix classes

`tome doctor --fix` repairs the following Phase 4 classes automatically:

| Subsystem | Repair action | Triggered by |
|-----------|---------------|--------------|
| `Summariser` (missing) | Re-download via `embedding::download::download_model` against the pinned URL. | `subsystem: Summariser, auto_fixable: true` SuggestedFix |
| `BindingRulesCopy` (missing or drifted) | Re-copy `<root>/workspaces/<name>/RULES.md` → `<project>/.tome/RULES.md` (atomic rename). | `subsystem: BindingRulesCopy, auto_fixable: true` |
| `HarnessRules(<name>)` (broken or drifted) | Re-run the rules-file slice of the sync algorithm for that harness only. | `subsystem: HarnessRules(<name>), auto_fixable: true` |
| `HarnessMcp(<name>)` (broken or drifted, NOT UserOwned) | Re-run the MCP-config slice of the sync algorithm for that harness only. | `subsystem: HarnessMcp(<name>), auto_fixable: true` |
| `Schema` (older-than-supported) | Run forward migration via `apply_pending`. | `subsystem: Schema, auto_fixable: true` (first registered in Phase 4 per FR-580) |

## Fix classes NOT safe (FR-562)

`--fix` does NOT repair:

- `Binding` broken (workspace named by marker is missing from `workspaces` table). Reason: ambiguity — should `--fix` re-create the workspace or rebind to a different one? Developer choice. Suggested action: "run `tome workspace use <existing-name>` to rebind, or `tome workspace init <name>` to recreate."
- `HarnessMcp(<name>)` user-owned conflict. Reason: developer-authored content under the `tome` key; refusing to overwrite without an explicit `--force` to the underlying sync command. Suggested action: "rerun `tome harness sync --force` to overwrite the user-owned entry."

These cases produce `auto_fixable: false` SuggestedFix items; the doctor reports them and the developer runs the named command explicitly.

## Exit codes

- 0: every subsystem `Ok`.
- 1: at least one subsystem `Degraded` or `Broken`; OR `--fix` ran and every repair succeeded but the overall classification is Degraded for other reasons.
- 75 (`DoctorFixNotSafe`): `--fix` ran AND at least one subsystem remains in a state that `--fix` could not safely repair (e.g. `Binding` broken, or user-owned MCP entry without `--force`).

## Filesystem-derived state (FR-546)

All Phase 4 subsystem checks are filesystem-derived. Tome maintains no sidecar state file or DB table tracking integration. The Tome block markers and the MCP entry shape are the source of truth. This makes doctor's checks idempotent and the `--fix` actions reproducible.

## Test coverage

- `tests/doctor_p4.rs` — extends Phase 3's `tests/doctor.rs`:
  - Binding subsystem: orphan binding → broken; valid binding → ok.
  - BindingRulesCopy: missing copy → broken; hand-edited copy → drifted; matching copy → ok; `--fix` re-copies.
  - HarnessRules: per-harness rules-file check; `--fix` reruns sync for that harness.
  - HarnessMcp: per-harness MCP config check; user-owned conflict → `auto_fixable: false`; `--fix` does NOT overwrite.
  - Summariser: missing model → broken; `--fix` re-downloads (CI-skipped real path; library-level cheap path tested).
  - `detected_uninstalled_harnesses`: harness directory exists but not in effective list → informational only; classification unchanged.
  - Subsystem enum promotion: `SuggestedFix.subsystem` is the typed enum; structured output deserialises round-trip stable.
