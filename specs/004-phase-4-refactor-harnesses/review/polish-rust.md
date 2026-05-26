# Phase 4 Polish — Rust-Lens Code Review

Phase-wide cross-slice Rust idiom review. Read-only audit against branch
`004-phase-4-polish-pr-a` at `/Users/aaronbassett/Projects/devrel-ai/tome`.
Per-US reviewers caught local concerns; this review surfaces what slipped
between slices.

## Blockers (0)

None. The Phase 4 surface compiles, the sync boundary holds, every Phase 4
error variant routes to a unique exit code, and the closed-set discipline is
preserved.

## Majors (12)

### M1. Three near-identical `atomic_write` helpers — `harness::rules_file`, `harness::mcp_config`, `catalog::store`

**Location:**
- `src/catalog/store.rs:97` `pub fn write_atomic`
- `src/harness/rules_file.rs:253` `fn atomic_write`
- `src/harness/mcp_config.rs:120` `fn atomic_write`

All three follow the same Phase 3 hardened pattern: refuse symlinks, capture
target mode on Unix, write to a `.tome.tmp.*` sibling, fsync, set permissions,
persist. `catalog::store::write_atomic` is `pub` and `settings::edit` already
reuses it. The two `harness/*` copies were forked because the rules-file +
MCP-config writes added the `.tome.tmp.` prefix to `NamedTempFile`, but that
is a one-line difference — `catalog::store::write_atomic` could accept an
optional `prefix: Option<&'static str>` without inflating its signature
meaningfully. As-is, future hardening (e.g. fsync the parent directory after
rename for crash-safety on ext4) has to land in three places. Promote the
canonical implementation to `crate::util::atomic_file` and have the three
callers reduce to one-line wrappers.

The `refuse_symlink` helper is also duplicated between `rules_file.rs:236`
and `mcp_config.rs:92` (and inlined in `catalog/store.rs:108`). Co-locate
with the promoted atomic-write so the policy lives in one place.

### M2. Three override registries with three different concurrency primitives

**Location:**
- `src/index/migrations.rs:195` `MIGRATIONS_OVERRIDE: thread_local! RefCell<Option<&'static [Migration]>>`
- `src/summarise/trigger.rs:57` `SUMMARISER_OVERRIDE: thread_local! RefCell<Option<Arc<dyn Summariser>>>`
- `src/harness/mod.rs:213` `HARNESS_MODULES_OVERRIDE: RwLock<Option<Vec<Box<dyn HarnessModule>>>>`

All three serve the same purpose — test-only injection of production
fallbacks — but each uses a different primitive:

| Override | Container | Thread-scope | Drop safety |
|----------|-----------|--------------|-------------|
| `MIGRATIONS_OVERRIDE` | `RefCell<Option<&'static [Migration]>>` | `thread_local!` | RAII guard in test file |
| `SUMMARISER_OVERRIDE` | `RefCell<Option<Arc<dyn Summariser>>>` | `thread_local!` | RAII guard in src |
| `HARNESS_MODULES_OVERRIDE` | `RwLock<Option<Vec<Box<dyn HarnessModule>>>>` | process-global | Guard pattern documented but lives in tests |

The harness override is process-global because `with_effective_modules` runs
the closure under the read guard — tests in parallel can read concurrently —
but the value is still `Vec<Box<dyn HarnessModule>>`, not a borrow. If two
parallel tests install different overrides, they race. With `cargo test`
defaulting to 1 binary per integration test file, this happens to be safe
today, but the contract isn't expressible in the type signature.

The three patterns also leak the test-injection convention into three module
shapes. Consider extracting a `crate::util::TestOverride<T>` newtype around
`thread_local! RefCell<Option<T>>` with an installable RAII guard so every
override site reads the same. At minimum, the `HARNESS_MODULES_OVERRIDE`
should be `thread_local!` like the other two — its current `RwLock` shape
buys nothing because the closure runs under the read guard (no concurrent
mutation possible without breaking the API contract).

### M3. `expect("HARNESS_MODULES_OVERRIDE poisoned")` panics every subsequent test call after one panicking test

**Location:** `src/harness/mod.rs:259`

```rust
let guard = HARNESS_MODULES_OVERRIDE
    .read()
    .expect("HARNESS_MODULES_OVERRIDE poisoned");
```

If any test that writes the override panics while holding the write guard,
the `RwLock` is poisoned and every subsequent `with_effective_modules` call
panics. `src/summarise/mod.rs:213` already gets this right:

```rust
let _guard = INIT_LOCK
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
```

Apply the same `PoisonError::into_inner` recovery on `HARNESS_MODULES_OVERRIDE`.
The read guard's only invariant is "I can see the current `Vec`"; a
poisoned-but-still-readable lock still satisfies that.

(If you take M2's suggestion and migrate the harness override to
`thread_local!`, M3 disappears.)

### M4. Duplicate `ProjectMarkerConfig` types (`settings::` and `workspace::resolution::`)

**Location:**
- `src/settings/mod.rs:139` `pub struct ProjectMarkerConfig { workspace, harnesses }`
- `src/workspace/resolution.rs:160` `pub struct ProjectMarkerConfig { workspace, harnesses }`

The two types are structurally identical and both `#[serde(deny_unknown_fields)]`.
`workspace::resolution::ProjectMarkerConfig` is `pub` but only used inside
the file (`read_project_marker`). The settings version is what every other
caller imports (`harness::sync`, `doctor::binding`, `doctor::mod`,
`commands::harness::list`).

Drop the `workspace::resolution::ProjectMarkerConfig` and use
`settings::ProjectMarkerConfig` throughout. The two types drifting silently
in a future spec change would create a real bug.

### M5. Three near-identical `read_project_marker` functions

**Location:**
- `src/workspace/resolution.rs:170` `fn read_project_marker(marker_path: &Path) -> Result<ProjectMarkerConfig, TomeError>`
- `src/harness/sync.rs:371` `fn read_project_marker(marker_path: &Path) -> Result<ProjectMarkerConfig, TomeError>`
- `src/commands/harness/list.rs:168` `fn load_project_marker(scope: &ResolvedScope) -> Result<Option<ProjectMarkerConfig>, TomeError>`

Plus `src/doctor/mod.rs:220-224` and `src/doctor/binding.rs:60-62` inline
the read-and-parse pattern. All map IO/parse errors to
`TomeError::WorkspaceMalformed`. After M4 lands, promote one
`settings::parser::read_project_marker(path) -> Result<ProjectMarkerConfig, TomeError>`
that does the read + parse + error mapping in one place. The doctor variants
that swallow errors via `.ok()` can call it and drop the result.

This same shape applies to `read_workspace_settings` / `read_global_settings`
— see `harness/sync.rs:382-410` vs `commands/harness/list.rs:182-212`.

### M6. Duplicate `relative_path` impls (`doctor::harness_integration` and `harness::sync`)

**Location:**
- `src/harness/sync.rs:618` `fn relative_path(from: &Path, to: &Path) -> PathBuf`
- `src/doctor/harness_integration.rs:138` `fn relative_path(base: &Path, target: &Path) -> std::path::PathBuf`

Same algorithm, different surface. They subtly differ on the `common == 0`
case: `harness/sync` falls back to `to.to_path_buf()`; `doctor/harness_integration`
walks the full `up + down` even when no common ancestor exists. Either is
defensible, but the two implementations can drift.

The `doctor/harness_integration.rs:120` `expected_body` is documented as
"mirrors the shape of `harness::sync::compute_rules_body`" without the I/O
error propagation. Promote `compute_rules_body` (with a `read_strict: bool`
parameter or a sibling `expected_body` constructor) and have doctor reuse it.
The risk today is that a future change to the AtInclude path format
(`@{relative}`) lands in sync but not in doctor, silently flipping every
harness from Ok to Drift.

### M7. SQL `SELECT id FROM workspaces WHERE name = ?1` repeated nine times

**Location:**
- `src/workspace/init.rs:126`
- `src/workspace/binding.rs:174`
- `src/workspace/regen_summary.rs:104`
- `src/workspace/remove.rs:140`
- `src/workspace/rename.rs:123`, `:140`
- `src/workspace/sync.rs:108`
- `src/workspace/resolution.rs:106` (slight variant: `SELECT 1`)
- `src/doctor/binding.rs:105` (variant: `SELECT 1`)
- `src/plugin/lifecycle.rs:508` (subquery)

Each caller maps `rusqlite::Error::QueryReturnedNoRows` to either
`WorkspaceNotFound` or a `None` sentinel, and other SQL errors to
`IndexIntegrityCheckFailure`. Several sites diverge in their error-message
wording. Promote
`index::workspaces::resolve_id(conn, name) -> Result<Option<i64>, TomeError>`
or `Result<i64, TomeError>` (returning `WorkspaceNotFound` for absent) and
collapse the duplication. This same module would be the natural home for
`workspaces_table_exists(conn) -> bool` which the resolver already needs.

### M8. `lookup` is `pub` but only used inside its own module

**Location:** `src/harness/mod.rs:231` `pub fn lookup`

The skill prompt explicitly asked about lookup vs `with_effective_modules`
usage. Every dispatch site uses `with_effective_modules` correctly — there
is zero production caller of `lookup`. The only references are:
- The doc comment of `commands/harness/use_.rs:7` (still says "validates against `lookup`" — outdated)
- The mod.rs tests inside `harness/mod.rs:290`

Either delete `pub fn lookup` (it's misleading), demote it to `pub(crate) fn`
for the tests only, or document it as a deprecated alias and route to
`with_effective_modules`. Leaving it `pub` invites a future contributor to
reach for it and silently bypass test overrides.

Also fix the `use_.rs` doc comment — it references `lookup` but the code
uses `with_effective_modules` (which is correct).

### M9. `CompositionErrorKind::BadExclusion` is being used for non-exclusion errors

**Location:** `src/settings/composition.rs:107`

```rust
// R-M8 (US3 review): inputs that LOOK bracketed but don't match
// any recognised form (...) are rejected here rather than falling
// through to `Include`.
if s.starts_with('[') {
    return Err(CompositionErrorKind::BadExclusion(s.to_owned()));
}
```

The variant's `Display` impl says `"malformed `!`-prefixed exclusion"`, but
the code path now emits it for malformed `[...]` expressions too. The
comment acknowledges this is misleading. Add a `MalformedReference` variant
to `CompositionErrorKind` (still routing to exit 17) so the user-visible
error message matches the actual failure. The closed-error-set discipline
permits new sub-variants on existing categories without churning the wire
contract.

Similarly, `settings/composition.rs:90` uses `BadExclusion` for a
malformed name inside `[workspaces.<name>]`. Either fold both into
`MalformedReference` with a descriptive payload, or use
`WorkspaceNameInvalid` directly (the validation already produces one).

### M10. `Subsystem::HarnessRules(_) | Subsystem::HarnessMcp(_)` unreachable arm

**Location:** `src/doctor/fixes.rs:235`

```rust
Subsystem::HarnessRules(_) | Subsystem::HarnessMcp(_) => {
    // Unreachable: `apply()` coalesces all harness fixes into a
    // single `repair_harness_sync_with` invocation outside the
    // `apply_one` dispatch (R-M2). If a caller dispatches one
    // directly we still want safe behaviour, so fall through to
    // the (one-shot) sync.
    repair_harness_sync_with(ctx, ctx.force)?;
    ...
}
```

The "unreachable" arm is defensive code that still calls the full
`repair_harness_sync_with` orchestrator. If `apply_one` is ever called
directly (the public surface today is `apply`, but `apply_one` is reachable
from inside the same module), the harness-sync runs unconditionally without
the dedup that `apply()` provides. Either:

1. Make `apply_one` truly private (it's already non-`pub`; nothing outside
   the file calls it) and `unreachable!()` here, or
2. Document that direct `apply_one` callers must NOT pass `HarnessRules` /
   `HarnessMcp` subsystems — and `debug_assert!(false)` in this branch so
   debug builds catch the misuse.

The current "fall through to safe behaviour" is the worst of both worlds:
it permits incorrect usage while making the contract opaque.

### M11. `BindingRulesCopy` repair re-implements per-project sync to avoid bothering siblings

**Location:** `src/doctor/fixes.rs:342-362`

The comment explains a real bug found during US5: the canonical
`workspace::sync::sync_one` walks every bound project of a workspace, so
calling it from `doctor --fix` against THIS project's binding would silently
clobber sibling projects' hand-edited rules. The fix surfaces
`sync_one_project` as a narrower entry point.

This is correct as far as it goes, but the underlying API — having both
`sync_one` (every project of a workspace) and `sync_one_project` (this
project only) — is brittle. The default `sync_one` is the dangerous one:
the doctor pass needs the safer variant precisely because the default
silently affects unrelated state. Consider:

1. Renaming `sync_one` to `sync_all_bound_projects_for_workspace` so its
   blast radius is in the name, and
2. Making `sync_one_project` the default-named primitive.

Or have `sync_one(name, paths)` return only the bound-projects list and
require callers to opt-in to the cascade via an explicit
`sync_each(projects, …)` helper. Either approach makes the safer call the
ergonomic one.

### M12. Workspace lifecycle reuse of `Usage` for "same name on rename"

**Location:** `src/workspace/rename.rs:96-99`

```rust
if old == new {
    return Err(TomeError::Usage(format!(
        "workspace rename: `<old>` and `<new>` are the same name (`{}`)",
        old.as_str(),
    )));
}
```

This is the only Phase 4 site that surfaces semantic-input-error through
`TomeError::Usage` (exit 2 — clap's usage error code). Every other
workspace-name failure routes through `WorkspaceNameInvalid` (exit 15) or
`WorkspaceAlreadyExists` (exit 14). The user receives "invalid usage" for
what is semantically a tautological-rename refusal.

Either:
- Use `WorkspaceNameInvalid` with `reason: "rename old == new"`, or
- Promote a new `WorkspaceRenameNoOp` variant if the wire-contract test
  flags this case specifically.

The current choice means CI surfaces this as a CLI usage failure, which
will confuse anyone scripting `tome workspace rename` from a loop that
happens to no-op.

## Minors (8)

### m1. `harness::lookup` doc says it consults the override but doesn't

**Location:** `src/harness/mod.rs:222-243`

The doc comment claims `lookup` "consults `HARNESS_MODULES_OVERRIDE` first";
the actual implementation reads `SUPPORTED_HARNESSES` only. The function
body even has a `// The override path is intended for tests...` block
explaining why it doesn't. This is a contract bug between doc and code. If
the function genuinely cannot honour overrides (because `'static`), say so
at the top of the doc rather than burying it three paragraphs in.

### m2. `WorkspaceName::parse` `.unwrap()`s defensible but undocumented

**Location:** `src/workspace/name.rs:63`, `:70`

The `s.chars().next().unwrap()` / `.next_back().unwrap()` are correctly
guarded by the prior `is_empty` check, and there's an inline comment. Fine.
But these are exactly the kind of "I proved it" unwrap that a future
refactor of `parse` (e.g. moving the `is_empty` check) could quietly break.
Prefer `if let Some(first) = s.chars().next()` pattern matches — they're
self-documenting and let the compiler enforce the invariant.

### m3. `home.canonicalize().unwrap_or_else(|_| home.to_path_buf())` masks an unreadable HOME

**Location:** `src/workspace/binding.rs:85`

If `$HOME` is unreadable (NFS hiccup, missing dir), the safety check that
forbids binding `$HOME` itself silently falls back to literal-path comparison.
A user whose `$HOME` is a symlink that doesn't resolve would bind happily.
The fallback is documented in the doc comment but the failure mode is
worth a `tracing::warn!` so the operator can correlate later "I bound my
home" surprise.

### m4. `is_dir()` vs `exists()` inconsistency in harness `detect()`

**Location:** all `src/harness/<harness>.rs`

`claude_code.rs:34`, `codex.rs`, `cursor.rs`, `gemini.rs`, `opencode.rs` all
use `home.join(".<harness>").is_dir()`. The `doctor::harness_detect` test
at `src/doctor/harness_detect.rs:83-87` exercises the case where the path
exists but is a file (not a directory) and confirms `detect()` returns
`false` — but the test only checks `claude_code`. Confirm the discipline
holds for all five and pin it via a table-driven test.

### m5. `compute_rules_body` and `expected_body` use `unwrap_or(Path::new(""))`

**Location:** `src/harness/sync.rs:427`, `src/doctor/harness_integration.rs:125`

```rust
let parent = snap.rules_path.parent().unwrap_or(Path::new(""));
```

A rules path with no parent (e.g. a relative "AGENTS.md" passed in by a
hostile test) silently falls through to the empty path. This is unreachable
because `rules_file_target` always returns `project_root.join(...)`, but the
defensive `unwrap_or` doesn't communicate that. Make it
`debug_assert!(snap.rules_path.parent().is_some(), ...)` so debug builds
catch the misuse and release builds keep the safety.

### m6. `commands::harness::info::ModuleSnapshot` carries `#[allow(dead_code)]` for `block_body_style`

**Location:** `src/commands/harness/info.rs:79-80`

The field is set on construction but never read. It looks like a forward-
looking field for the `tome harness info` body-style report that wasn't
wired. Either:

1. Wire it into the human/JSON output (the info command already reports
   `rules_strategy`, `mcp_format`, etc. — adding body style is a one-line
   change), or
2. Drop the field.

A `#[allow(dead_code)]` on a struct field in a polish phase typically signals
unfinished scope; pin it down.

### m7. Five `#[allow(clippy::too_many_arguments)]` attributes on doctor helpers

**Location:** `src/doctor/mod.rs:279`, `:305`, `:354`, `:437`; `src/settings/resolver.rs:322`, `:453`

These accumulate during cross-slice integration where each user-story slice
added an argument. After Phase 4 closes, the 9-arg `classify_pub` /
`build_suggested_fixes_pub` would benefit from a struct refactor:

```rust
struct ClassifyInputs<'a> {
    embedder: &'a ModelHealth,
    reranker: &'a ModelHealth,
    summariser: &'a ModelHealth,
    index: &'a IndexHealth,
    drift: &'a DriftStatus,
    catalogs: &'a [CatalogCacheHealth],
    binding: Option<&'a ProjectBindingState>,
    harness_rules: &'a [HarnessSubsystemReport],
    harness_mcp: &'a [HarnessSubsystemReport],
}
```

The `FixContext<'a>` in `doctor/fixes.rs:63` already follows this pattern;
extend it to the classify pipeline.

### m8. Tests duplicate `fn ws()` and `fn project()` helpers across 9 settings test files

**Location:**
- `fn ws(name, harnesses)` in 9 settings/workspace test files
- `fn project(workspace, harnesses)` in 8 settings test files

Each is a one-line wrapper constructing a `WorkspaceSettings` /
`ProjectMarkerConfig`. The skill prompt explicitly asked about test-helper
promotion. Move both to `tests/common/mod.rs` as
`make_ws(name, harnesses)` / `make_project(workspace, harnesses)`. Same
applies to `fn open_central(paths)` (7 files), `fn seed_bound_project(paths, name, project_root)`
(5 files), `fn fallback_scope() -> ResolvedScope` (5 files), `fn parse(name)` /
`fn ws_name(name)` (6 files), and `fn global_scope() -> ResolvedScope`
(3 files). The rule-of-three has been crossed multiple times.

## Nits (4)

### n1. `error.rs:300` admits a contract typo without fixing it

```rust
// 24 — Phase 4 summariser. Note: `contracts/exit-codes-p4.md`
// ships code 20 for this variant, which collides with Phase 2's
// pre-existing `PluginNotFound` (20). ... F3 lands `SummariserFailure`
// here and flags the contract typo for reconciliation in F4+.
```

The recent-changes log says "Contract docs were corrected in US4.d-1 (PR #74)".
Update the comment in `error.rs` to reflect that the contract is now fixed,
or remove the historical-note paragraph. Same applies to the lengthy
`SummariserFailureKind` doc comment.

### n2. `paths::canonicalize` falls back to `to_path_buf` silently

**Location:** `src/workspace/resolution.rs:134`

```rust
let canon = here.canonicalize().unwrap_or(here.clone());
```

Same pattern as m3 — silent fallback on canonicalisation failure. The
walk-for-project-marker case is less safety-critical than binding (we're
discovering, not committing), but the inconsistency between "fall back
silently" and the binding-side "bubble the error" is worth aligning.

### n3. `BACKEND.get().expect("LlamaBackend was set above or by a racing init")`

**Location:** `src/summarise/mod.rs:237`

The expect message is accurate; the situation is genuinely unreachable
because the line above just `set()` it. This is fine, but if you ever
change the racing-init handling, the expect becomes misleading. Prefer:

```rust
match BACKEND.get() {
    Some(b) => Ok(b),
    None => Err(TomeError::SummariserFailure {
        kind: SummariserFailureKind::BackendInitFailed {
            source: "LlamaBackend set/get raced; this is a Tome bug".into(),
        },
    }),
}
```

— so the failure is loud and recoverable rather than a panic.

### n4. `tracing::warn!` discipline is consistent but `tracing::error!` is never used

**Location:** all of `src/` per grep

Phase 4 introduces `tracing::warn!` for downgraded failures (cascade
continues, fix attempt failed, orphan removal failed). Zero `tracing::error!`
calls. That's a deliberate choice — errors bubble through `TomeError` and
the CLI surface emits them — but it's worth a one-line comment in the
logging module documenting the convention: "warn for downgraded continue,
error never (errors bubble)". The MCP server's tracing setup may differ;
worth cross-checking.

## Verdict

**Accept with majors**. Phase 4 ships in a structurally sound state; the
sync boundary holds, every new error variant maps to a unique exit code,
the override-aware dispatch convention is honoured at every site (`lookup`
itself is unused), strict-on-Tome-owned-inputs holds, and atomic-write
discipline is followed even where it duplicates.

The recurring theme of these 12 majors is **consolidation owed**: three
override patterns, three atomic-write helpers, two `ProjectMarkerConfig`s,
two `relative_path`s, nine `SELECT id FROM workspaces WHERE name`
copy-pastes, eight `fn project()` test-helper copies. Per-US reviewers
caught local issues; this is what falls between them. None of these block
merge — they're polish-phase work, which is exactly what `004-phase-4-polish-pr-a`
is for. The Phase 3 polish phase shipped 8 PRs against a similar quantity
of findings; budget for a comparable Phase 4 polish round and these all
become small, bounded PRs.

The Phase 4 retro should pin the lesson explicitly: the more US slices a
phase has (5 here vs Phase 3's 5), the more cross-slice duplication
accumulates because each slice owns its local shape. The next phase
should rotate `/sdd:map incremental` runs MID-phase (not just at slice
end) so emerging duplication surfaces before the polish phase has to
sweep it up.

## Files referenced

- `/Users/aaronbassett/Projects/devrel-ai/tome/src/error.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/util/atomic_dir.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/catalog/store.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/harness/mod.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/harness/rules_file.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/harness/mcp_config.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/harness/sync.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/settings/mod.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/settings/composition.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/settings/resolver.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/settings/edit.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/summarise/mod.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/summarise/trigger.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/summarise/registry.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/name.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/init.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/remove.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/rename.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/binding.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/resolution.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/sync.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/workspace/regen_summary.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/doctor/mod.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/doctor/binding.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/doctor/fixes.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/doctor/harness_integration.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/doctor/harness_detect.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/commands/harness/info.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/commands/harness/list.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/commands/harness/use_.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/commands/harness/remove.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/src/index/migrations.rs`
- `/Users/aaronbassett/Projects/devrel-ai/tome/tests/common/mod.rs`
