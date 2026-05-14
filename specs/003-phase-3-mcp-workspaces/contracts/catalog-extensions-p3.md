# Catalog Extensions — Phase 3 Contract

Phase 1 introduced catalogs. Phase 2 added the reindex cascade and the remove cascade. Phase 3 layers two additions:

1. **Per-scope catalog list.** A workspace's `config.toml` records its own `[catalogs]` table. The global config remains, unchanged in shape.
2. **Reference-counted on-disk clones.** The `${XDG_DATA_HOME}/tome/catalogs/<sha256-of-url>/` cache directories are shared across all scopes. A clone is removed only when the last referencing config is gone.

This document is the contract for the new reference-counting behaviour. It does NOT modify the Phase 2 cascade-disable semantics (those operate within one scope's index and are unchanged).

## Reference-counting algorithm

`catalog::store::reference_count(url: &str) -> Vec<ScopePath>`:

1. Walk the global config; if `[catalogs]` contains an entry whose URL matches, push `Scope::Global`.
2. Walk every workspace path recorded in `${XDG_STATE_HOME}/tome/workspaces.txt` (best-effort; missing/unreadable file → empty list).
3. For each workspace path, load its `.tome/config.toml` if present; if `[catalogs]` contains the URL, push `Scope::Workspace(path)`.
4. Return the (possibly empty) list of referencing scopes.

URL equality is exact-string match after credential scrubbing.

## On `tome catalog add`

```
let resolved_scope = …;                   # current command's scope
let cache_path = paths.catalogs_dir.join(sha256(url));

if cache_path.exists():
    # Some other scope already cloned this URL. Reuse.
    # No git clone; record the catalog in the resolved scope's config.
else:
    # First time on this machine. Clone (Phase 1 path).
    git::clone(url, cache_path, ref)
    # Record the catalog in the resolved scope's config.
```

Behaviour change vs Phase 1/2: the existence check is unchanged for the global scope (Phase 1 already content-addressed clones). What's new is that workspace-scoped `catalog add` may reuse a clone the global scope already brought down — or vice versa.

## On `tome catalog remove`

```
let resolved_scope = …;
let entry = config.catalogs.remove(name)?;          # Phase 1 path: drop the config entry
write_config(&config)?;

# NEW IN PHASE 3:
let refs = reference_count(&entry.url);
if refs.is_empty():
    fs::remove_dir_all(cache_path).ok();             # best-effort; orphaned clone is fine
    # workspaces.txt is not pruned here; only `tome workspace init --force` removes entries
```

The reference-count check runs **after** the config write, so a crash between the two is benign: the clone persists, the developer can re-add the catalog if needed.

## On `tome catalog remove --force` (cascade)

Phase 2 cascade-disable semantics (drop all enabled plugins' index rows under one lock window) are unchanged. Phase 3 adds the reference-count check at the very end, exactly as in the non-cascade path.

## Concurrency

The reference-count read is **not** taken under any lock. Two processes simultaneously removing the last reference race benignly:

- Both compute `refs.is_empty() == true`.
- Both call `fs::remove_dir_all(cache_path)`.
- One succeeds; the other gets `NotFound` and silently continues.

A worse case — process A removes the last reference, process B re-adds the URL to a workspace before process A's `fs::remove_dir_all` runs — leaves the clone gone and the new reference dangling. The next `tome catalog update <name>` on the workspace re-clones. No data loss; one extra round-trip.

This is the same TOCTOU profile as Phase 2's `cascade_disable_for_catalog` pre-check (documented in `CONCERNS.md`).

## Doctor reporting

`tome doctor` enumerates every catalog clone on disk and matches each against the reference count:

- **Referenced and reachable** — `state: "ok"`.
- **Referenced but missing on disk** — `state: "missing"`. Suggested fix: `tome catalog update <name>` (auto-fixable: re-clone from the recorded URL).
- **Referenced but cache directory is not a Git repo** — `state: "not_a_repo"`. Suggested fix: same.
- **Cache exists but no config references it** — orphan. Reported in the catalog section as an additional informational entry; suggested fix: `rm -rf <path>` (auto-fixable: no, manual decision).
- **Cache exists, is a Git repo, but `tome-catalog.toml` doesn't parse** — `state: "manifest_invalid"`. Suggested fix: `tome catalog show <name>` for diagnosis (auto-fixable: no).

## Workspace registry (opt-in)

`${XDG_STATE_HOME}/tome/workspaces.txt`:

- Plain text file, one absolute path per line, deduplicated.
- `tome workspace init` appends if the file exists. Does NOT create the file (registration is opt-in).
- `tome workspace info` may read it (informational only; not authoritative).
- `tome doctor` reads it to compute reference counts.
- No "remove" command. Stale entries (workspaces deleted by hand) are tolerated — they simply fail to load a config during the reference count and are ignored.

To opt in, the developer runs once:

```sh
mkdir -p ~/.local/state/tome
touch ~/.local/state/tome/workspaces.txt
```

Future `tome workspace init` calls then auto-register the new workspace's absolute path.

## Why opt-in (rationale)

A non-opt-in workspace registry would silently track every project a developer initializes a workspace inside. The list is small but security-sensitive (it leaks project locations). Making it opt-in keeps the catalog-clone GC working in 95% of cases (the developer who cares about disk hygiene opts in) and avoids creating a tracking artefact for the developer who doesn't.

Doctor reports the opt-in status:

```
Workspace registry: opt-in (file present, 3 workspaces tracked)
```

or:

```
Workspace registry: opt-in (file absent — catalog cleanup will not consider workspaces other than the resolved scope)
```

## What this contract does NOT change

- The Phase 1 `tome-catalog.toml` schema is unchanged.
- The Phase 1 catalog URL hashing (sha256 of URL) is unchanged.
- The Phase 2 cascade-disable atomicity (single advisory-lock window) is unchanged.
- The Phase 2 auto-disable-on-orphan path (`lifecycle::auto_disable_orphan`) is unchanged.
- The catalog refresh path's reindex cascade (Phase 7) is unchanged.

## Exit codes

No new exit codes specific to this contract. Phase 3 reuses:

- 0 — success.
- 3 — `CatalogNotFound` (existing).
- 4 — `CatalogAlreadyExists` (existing).
- 7 — `Io` — including `remove_dir_all` failures, reported but non-fatal (the config write already succeeded).
- 53 — `CatalogHasEnabledPlugins` (Phase 9) — unchanged.
