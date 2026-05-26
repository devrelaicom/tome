# Phase 5 — Doctor extensions

Authoritative contract for the Phase 5 additions to `tome doctor`. Read-only by default (FR-124).

## Phase 4 doctor surface — unchanged

Phase 4 ships `DoctorReport` with subsystems for: Embedder, Reranker, Index, Drift, Catalog (per-name), Schema, Summariser, Binding, BindingRulesCopy, HarnessRules (per-harness), HarnessMcp (per-harness). All preserved verbatim in Phase 5.

## Phase 5 surface additions

### `prompts` section

Per FR-120 + FR-121.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsReport {
    pub prompts: Vec<PromptDescriptor>,    // exact shape per data-model.md §4.4
    pub collisions: Vec<CollisionRecord>,  // exact shape per data-model.md §4.6
}
```

Doctor populates this by reading the active workspace's enabled-and-user-invocable entries (same query as `prompts/list`), deriving prompt names, resolving collisions. The collision-resolution pass is identical to what the MCP server runs at startup — same algorithm, same input.

#### Human-mode rendering

```
Prompts surface for workspace "midnight-dev":

midnight-expert (2 prompts):
  /mcp__tome__midnight_expert__compact_circuits   skill          (was: compact-circuits)
  /mcp__tome__midnight_expert__fix_issue           command (override)

compact-cli-dev (1 prompt):
  /mcp__tome__compact_cli_dev__fix_issue2          command  ← collision with midnight-expert__fix_issue
```

Collisions visually annotated; the original entry name is shown in parens when sanitisation altered it.

#### JSON-mode rendering

```json
{
  "prompts": {
    "prompts": [ /* PromptDescriptor[] */ ],
    "collisions": [ /* CollisionRecord[] */ ]
  }
}
```

### `orphan_data_dirs` section

Per FR-122. Informational only (no `--fix` repair handler in Phase 5; cleanup deferred to Phase 6+).

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanDataDirReport {
    pub plugin_data: Vec<PathBuf>,    // orphaned <home>/.tome/plugin-data/<catalog>/<plugin>/ paths
    pub workspace_data: Vec<PathBuf>, // orphaned <home>/.tome/workspaces/<ws>/plugin-data/<catalog>/<plugin>/ paths
}
```

#### Detection algorithm

1. Walk `<home>/.tome/plugin-data/` to enumerate `(catalog, plugin)` pairs that have on-disk data dirs.
2. Build the set of enabled plugins via `SELECT DISTINCT catalog, plugin FROM workspace_skills` across ALL workspaces.
3. Any `(catalog, plugin)` pair on disk that's NOT in the enabled set → plugin-data orphan.
4. Walk `<home>/.tome/workspaces/<ws>/plugin-data/<catalog>/<plugin>/` for every workspace dir.
5. For each: check `(ws, catalog, plugin)` is in the workspace's enabled set (`workspace_skills` filtered by workspace).
6. Not enrolled → workspace-data orphan.

Both walks are read-only `fs::read_dir` traversals. No directory creation.

#### Human-mode rendering

```
Orphan persistent data directories:

  plugin-data (no longer enabled in any workspace):
    /Users/aaron/.tome/plugin-data/old-catalog/removed-plugin/

  workspace-data:
    /Users/aaron/.tome/workspaces/midnight-dev/plugin-data/midnight-expert/legacy-plugin/

Cleanup: not auto-fixable in Phase 5. Manual rm -rf required; future phases will add tooling.
```

### `entry_counts` section

Per FR-123.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryCountsByKind {
    pub skills: u32,
    pub commands: u32,
    pub pending_re_embedding: u32,
}
```

`pending_re_embedding` = count of enabled entries whose **source file modification time is newer than their stored `indexed_at` timestamp**. This is a heuristic — it flags entries that have been touched on disk since their last reindex, but does NOT recompute `content_hash` (which would require reading and parsing every entry's full body + frontmatter, bounded only by total entry size). The mtime comparison is bounded by one `fs::metadata` call per enabled entry — microseconds each, suitable for doctor's read-only-cheap invariant (FR-124).

Heuristic caveats documented for users:
- A skill whose source file was `touch`ed without actual content change will be flagged as pending — false positive. Mitigation: `tome reindex` will detect that `content_hash` is unchanged and skip re-embedding.
- A skill whose `embedding_text` composer rules changed between Tome versions (e.g. the Phase 5 addition of `when_to_use` to the composer) will NOT be flagged by mtime alone. Mitigation: `tome reindex --force` is the recommended remediation after a Tome upgrade.

Doctor renders this count for situational awareness only; it does NOT trigger reindexing. Users run `tome reindex` to act on the signal.

#### Human-mode rendering

```
Entry counts:
  Skills:    47
  Commands:  3
  Pending re-embedding: 5
```

## DoctorReport struct extension

The existing `DoctorReport` gains three new fields:

```rust
pub struct DoctorReport {
    // ... existing Phase 1–4 fields ...

    pub prompts: Option<PromptsReport>,
    pub orphan_data_dirs: Option<OrphanDataDirReport>,
    pub entry_counts: Option<EntryCountsByKind>,
}
```

`Option<>` because all three sections are only populated when the resolved scope is a known workspace; doctor running in GlobalFallback or outside-project modes emits `None` (per Phase 4's existing optionality convention).

## Read-only enforcement (FR-124)

Phase 5 doctor surfaces MUST be read-only by default:

- Enumerating the prompts surface MUST NOT lazy-create `${TOME_PLUGIN_DATA}` or `${TOME_WORKSPACE_DATA}` directories. The PromptDescriptor list is derived from frontmatter + entry rows only; the substitution layer is NOT invoked.
- Orphan detection is a `fs::read_dir` walk; no `create_dir_all`, no writes.
- Entry counts are pure SQL queries.

A test pins the read-only invariant: `tests/doctor_p5.rs::doctor_phase5_surface_creates_no_dirs` snapshots `<home>/.tome/` before/after `tome doctor` (no `--fix`) and asserts no new directories appear.

## `--fix` interactions

Phase 5 adds NO new `--fix` repair handlers. The Phase 4 repair classes (Summariser, BindingRulesCopy, HarnessRules, HarnessMcp, Schema) all continue to work. The Phase 5 surface additions are informational only:

- Prompts surface: not auto-fixable. The remediation surface for collisions is `prompt_name` frontmatter in the plugin author's tree.
- Orphan data dirs: not auto-fixable in Phase 5 (deferred to Phase 6+).
- Entry counts: not auto-fixable (pending re-embedding is fixed by `tome reindex`).

`tome doctor --fix` with no Phase 5-fixable issues but Phase 4 issues remaining still works for those.

## Exit codes

Doctor's existing exit code semantics are preserved:
- `0` healthy.
- `1` degraded (Phase 5 surfaces don't trigger degraded by themselves — orphan dirs and collisions are informational; `pending_re_embedding > 0` is also informational, not degraded).
- `75` if `--fix` ran but unfixable issues remain.

Phase 5 surfaces do NOT introduce new classification rules.

## Tests

| Behaviour | Test |
|---|---|
| Prompts surface enumerated with collisions | `tests/doctor_p5.rs::prompts_surface_enumerates_with_collisions` |
| Orphan plugin-data detected | `tests/doctor_p5.rs::orphan_plugin_data_detected` |
| Orphan workspace-data detected | `tests/doctor_p5.rs::orphan_workspace_data_detected` |
| Entry counts split by kind | `tests/doctor_p5.rs::entry_counts_by_kind` |
| `pending_re_embedding` counts dirty entries | `tests/doctor_p5.rs::pending_re_embedding_count_matches_dirty_rows` |
| Doctor Phase 5 surfaces create no dirs | `tests/doctor_p5.rs::doctor_phase5_surface_creates_no_dirs` |
| Doctor outside-project: Phase 5 fields are None | `tests/doctor_p5.rs::outside_project_phase5_fields_none` |
| JSON wire shape extension preserved | `tests/doctor_json.rs::phase5_fields_serialise_correctly` |
