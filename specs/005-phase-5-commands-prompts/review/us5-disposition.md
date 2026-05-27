# Phase 5 / US5 — Reviewer disposition

Decisions on which `us5-findings.md` items get applied in US5.c vs deferred. Conservative: blockers + selected majors + cheap minors + test gaps.

## Applied (this PR)

### Production code

1. **R-M1** — Promote `Paths::plugin_data_root()` accessor; consume from orphan walk + writers. Single source of truth for the `<root>/plugin-data/` layout.
2. **R-M3** — Wrap `count_entries_by_kind` in `conn.unchecked_transaction()` for snapshot consistency across the two SELECT statements.
3. **R-M4** — Emit `tracing::warn!` in `plugin show`'s `list_entries` when a description exceeds `MAX_DESCRIPTION_MAX_CHARS`. Surface misbehaving catalogs without changing wire shape.
4. **R-M5** — Tighten the `build_phase5_surfaces` carve-out doc comment to specifically reference `ScopeSource::GlobalFallback` (not "outside a workspace context").
5. **R-m2** — Drop `n.max(0)` defensive code from two `u32::try_from` sites.
6. **R-m3** — Add `tracing::warn!` to three `read_dir` silent-bail sites in doctor checks.
7. **R-m4** — Replace `std::collections::HashSet<std::path::PathBuf>` with bare `HashSet<PathBuf>` at 4 sites (use statement already covers).
8. **R-m5** — Read `fs::metadata().len()` for `IndexHealth.size_bytes` when `check_index` errors but the file exists.
9. **R-m10** — Drop dead `skill_count: Option<u32>` from `PluginListEntry`.
10. **R-m11** — Promote `/mcp__tome__` prefix to `pub const MCP_SLASH_PREFIX` in `src/mcp/mod.rs`; consume from `commands/doctor.rs`.
11. **R-n3** — Update `src/mcp/prompt_collision.rs:25` doc comment to mention doctor's `PromptsReport` reuse.

### Tests

1. **T-G1** — Add `dormant_not_annotated_when_searchable_true` to `tests/plugin_show_p5.rs`.
2. **T-G2** — Extend `skill_default_flags_in_json` + `command_default_flags_and_derived_prompt_name` to assert the complementary section is an empty array.
3. **T-W1** — Tighten `entry_counts_by_kind` to exact assertions against the deterministic fixture. (`pending_re_embedding` `>=1` assertion is preserved per reviewer note — heuristic.)

## Deferred to v0.6+ / Polish backlog

### Contract-level / structural

- **R-M2** — Orphaned-workspace-dir floods the orphan report. Requires either a new `workspace_dirs: Vec<PathBuf>` field on `OrphanDataDirReport` OR repurposing `workspace_data` to carry workspace-root paths when the workspace itself is orphaned. Contract amendment + Phase 4 doctor JSON schema impact. Out of US5.c closeout scope; tracked for v0.6+ Polish.

### Cosmetic / perf-not-material

- **R-m1** — Per-iteration `String` allocations in `walk_plugin_data_for_orphans` predicate. Read-only diagnostic; not material.
- **R-m6** — Defensive `out.dedup()` after sort in `collect_detected_uninstalled`. `inventory::submit!` already guarantees uniqueness; documented invariant defence-in-depth.
- **R-m7** — Duplicate path-safety check in `commands/plugin/show.rs` vs `index::skills::resolve_entry_body_path`. Promote-helper refactor; defer until 3rd consumer.
- **R-m8** — `prompt_name: null` serialization vs field-absent. Contract reviewer confirmed `null` is acceptable; no test breakage.
- **R-m9** — `.as_deref().map(Result)` then `.ok()` chain. Works; rewrite optional.
- **R-n1** — Single-positional `writeln!` trailing comma. rustfmt-clean.
- **R-n2** — `classify_pub` / `build_suggested_fixes_pub` naming suffix. Works; renaming touches multiple sites.
- **MINOR** (Subsystem owned-String alloc) — Per-report alloc cost is negligible; closed-set discipline > 8 allocations/report.
- **MINOR** (`PromptsReport` doc comment relocation) — Cosmetic.

### Security

- **S-MEDIUM** (symlink path disclosure in orphan output) — Documented carve-out: threat materializes only if (a) attacker can write `~/.tome/`, (b) doctor output captured and parsed by untrusted tooling, (c) tool interprets path as syscall arg. v0.6+ cap-std hardening eliminates this surface entirely; no US5.c action.
- **S-LOW** (terminal escape sequences in plugin/entry names) — Design surface, not a security boundary. Catalog authors can declare descriptive names. v0.6+ could add control-char sanitization at parse time.

### Test gaps not flagged

- Real BGE model verification (SC-001 / SC-002) — manual; tracked since US1.
- cap-std/TOCTOU coverage — v0.6+ when hardening lands.
- Concurrent doctor passes — read-only path; no new race surface introduced by US5.b.

## Application order

1. Two `docs(review)` commits (findings + this disposition) — committed first per convention.
2. One `fix(phase5/us5)` commit applying the 11 production-code items.
3. One `test(phase5/us5)` commit applying the 3 test improvements.

Tests must remain green at every commit boundary (pre-commit hook gate).
