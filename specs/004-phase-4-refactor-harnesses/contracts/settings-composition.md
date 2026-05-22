# Layered Settings + Composition — Contract

**Spec source**: [spec.md FR-440 through FR-450](../spec.md)
**Research**: [research.md R-9](../research.md)

## Three settings layers

In priority order:

1. **Project**: `<project_root>/.tome/config.toml`. Field: `harnesses: Vec<String>` (optional).
2. **Workspace**: `<root>/workspaces/<name>/settings.toml`. Field: `harnesses: Vec<String>` (optional).
3. **Global**: `<root>/settings.toml`. Field: `harnesses: Vec<String>` (optional).

## Resolution algorithm

```text
effective_list(project_marker, workspace_settings, global_settings, central_db):
  // visited stores (scope_kind, scope_key) pairs. scope_key is a String:
  //   - Workspace: the workspace's name (workspace.name)
  //   - Global: the literal "<global>"
  //   - Project: the project's canonical path as a string
  visited: HashSet<(ScopeKind, String)> = {}

  resolve_scope(scope_kind, scope_key):
    if (scope_kind, scope_name) in visited:
      return Err(Cycle { path: visited })
    visited.add((scope_kind, scope_name))

    declared = directly_declared_list(scope_kind, scope_name)
    if declared is None:
      return Ok({inclusions: [], exclusions: []})

    inclusions = {}
    exclusions = {}
    for entry in declared:
      match parse(entry):
        Inclusion(name) =>
          if name not in SUPPORTED_HARNESSES: return Err(HarnessNotSupported(name))
          inclusions.add(name with source: scope_kind)
        Exclusion(name) =>
          if name not in SUPPORTED_HARNESSES: return Err(BadExclusion(name))
          exclusions.add(name)
        CurrentWorkspace =>
          if scope_kind != Project: return Err(WorkspaceRefOutsideProject)
          if project_marker is None or project_marker.workspace is None:
            // project marker without workspace name is impossible (binding pointer requires it)
            unreachable
          sub = resolve_scope(Workspace, project_marker.workspace)
          merge sub into inclusions/exclusions
        NamedWorkspace(name) =>
          if not central_db.has_workspace(name): return Err(UnknownWorkspace(name))
          sub = resolve_scope(Workspace, name)
          merge sub into inclusions/exclusions
        Global =>
          sub = resolve_scope(Global, "<global>")
          merge sub into inclusions/exclusions
        BadComposition(s) =>
          return Err(BadExclusion(s))   // covers ![global], ![workspaces.x], malformed brackets

    return Ok({inclusions, exclusions})

  // Walk priority order: stop at first scope that declares
  if project_marker.harnesses is Some:
    result = resolve_scope(Project, project_root)
  elif bound_workspace.harnesses is Some:
    result = resolve_scope(Workspace, bound_workspace.name)
  elif global_settings.harnesses is Some:
    result = resolve_scope(Global, "<global>")
  else:
    return Ok(EffectiveHarnessList { harnesses: [], excluded: [] })

  // Final pass: subtract exclusions from inclusions
  effective = result.inclusions - result.exclusions
  return Ok(EffectiveHarnessList {
    harnesses: effective sorted by first-included-from chain,
    excluded: result.exclusions.collect(),
  })
```

## Composition reference forms (FR-443)

All array entries are TOML string values. Each string matches one of:

| Form | Meaning | Valid in scope |
|------|---------|----------------|
| `"<name>"` | Include harness `<name>` | any |
| `"!<name>"` | Exclude harness `<name>` (subtract from union) | any |
| `"[workspace]"` | Include the bound workspace's directly-declared list | **project only** |
| `"[workspaces.<name>]"` | Include `<name>` workspace's directly-declared list | any |
| `"[global]"` | Include global's directly-declared list | any |

The bracketed forms are **string literals containing brackets**, not TOML table headers (FR-450). The parser is a string-match ladder; no TOML structure inspection.

## Cycle detection

DFS tracks visited `(scope_kind, scope_name)` tuples. On re-visit, the cycle path is the current visited list. Error code 17 with the path included in the message: `composition cycle: workspace `a` → workspace `b` → workspace `a``.

`[global]` is terminal — it can be referenced from any layer but resolving global goes no further.

## Stop-at-first-declarer (FR-441)

The walk visits scopes in priority order. The FIRST scope that has a `harnesses` key (regardless of its value, including empty `[]`) terminates the priority walk. Further scopes are consulted ONLY via composition references from the first declarer's list.

- Project marker has `harnesses = []` → effective list is `[]`. Workspace and global are NOT consulted.
- Project marker has no `harnesses` key → walk to workspace.
- Workspace has `harnesses = ["claude-code"]` → effective list is `["claude-code"]`. Global is NOT consulted.
- Workspace has `harnesses = ["[global]", "claude-code"]` → walk to global to resolve `[global]`, union with `claude-code`.

## Composition references resolve to *directly-declared* lists (FR-449)

A composition reference resolves to the referenced scope's **as-written** harness list, not its computed effective list. Concretely: project's `harnesses = ["[workspace]", "!cursor"]` whose bound workspace has no `harnesses` key resolves to `[]` (empty inclusion set, then subtract `cursor` from nothing) — NOT to global via the workspace's hypothetical priority walk.

This is the "composition is a one-level reference, not a re-entrant resolver" rule. Without it, every composition reference would re-trigger the full priority walk and the rules become unintelligible.

## Errors

| Failure | Exit code |
|---------|-----------|
| Cycle | 17 |
| `[workspace]` outside project scope | 17 |
| `!`-prefixed bracketed form (e.g. `![global]`) | 17 |
| Composition reference to non-existent workspace | 13 |
| Composition reference names an unsupported harness | 18 |
| Direct inclusion of unsupported harness | 18 |

## Order of entries

Within an array, order doesn't affect the result (set union + set subtraction). `["a", "!b"]` and `["!b", "a"]` produce the same effective list. This means tools that emit the array (e.g. `tome harness use --scope project`) can choose any deterministic ordering (e.g. alphabetical) without changing semantics.

## Test coverage

- `tests/settings_composition.rs` — every form individually, multi-level composition, cycle detection, project-only `[workspace]` enforcement, bad `!`-prefix rejection.
- `tests/settings_priority.rs` — three-layer walk; empty `[]` opt-out at each layer; absent-key fall-through.
- `tests/settings_composition_resolves_to_as_written.rs` — the FR-449 invariant: composition refs do NOT re-trigger priority walks.
