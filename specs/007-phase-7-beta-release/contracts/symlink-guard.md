# Contract: Symlink-Safe Write Guard (FR-007)

**FR**: FR-007 · **NFR**: NFR-004 · **SC**: SC-006 · **Research**: §R-1 · **Backlog**: `CONCERNS.md` TD-062 / SEC-019 / C-1

Finishes the long-deferred mechanical symlink hardening: a Tome-managed write MUST NOT traverse a **symlinked intermediate directory component**, in addition to the existing final-node refusal. Affordable now because `rustix` is already in the graph (transitive→direct, **no new package**).

## The spike (gates the approach — runs EARLY, F2)

Before the hardening lands, a short spike MUST confirm, under the feature set already enabled transitively (`rustix/fs` via `tempfile`), that:

1. **Linux**: `rustix::fs::openat2` with `ResolveFlags::NO_SYMLINKS` is reachable and refuses a symlinked component.
2. **Portable**: a per-component `rustix::fs::openat` + `OFlags::NOFOLLOW` directory walk is reachable on macOS (where `openat2` is Linux-only).

**Outcome recorded against FR-007**:
- **Spike passes** → the full-path primitive hardens the intermediate walk on every sink.
- **Spike fails** (primitive unreachable without a *new* package) → FR-007 **degrades** to final-node `O_NOFOLLOW` + the documented trust-model mitigation. **No new package either way** (NFR-004).

## The SSOT primitive

A single helper in `src/util/` (e.g. `symlink_safe.rs`) is the **sole** symlink-safe write-open path:

```text
fn open_write_no_follow(target: &Path) -> Result<File, io::Error>   // or returns a guard the caller persists into
```

- Primary: `openat2(RESOLVE_NO_SYMLINKS)` (Linux) / per-component `openat` + `O_NOFOLLOW` walk (portable).
- Fallback: final-node `O_NOFOLLOW`.
- The existing duplicated `refuse_symlink` copies (`util/atomic_dir.rs:249`, `harness/mcp_config.rs:92`, the `catalog/store.rs` final-node check) **delegate** to this primitive — one source of truth, closing the "fix one sink, miss its parallel" hazard structurally.

## All sinks, ONE pass (mandatory)

The consolidation MUST be applied across **every** Tome-managed write sink in a **single** slice (R2, §R-22) — never sink-by-sink (the project has twice shipped a one-sink fix that missed its parallel):

| Sink | Module | Refusal → exit code |
|---|---|---|
| Hooks settings (`settings.local.json`) | `harness/hooks.rs` | `HookSettingsWriteFailed` (44) |
| Guardrails in-file regions + Cursor sibling | `harness/guardrails.rs` | `GuardrailsWriteFailed` (46) |
| Agent files | `harness/agents.rs` | the agents write variant (existing) |
| Rules file | `harness/rules_file.rs` | existing rules write code |
| MCP config | `harness/mcp_config.rs` | existing mcp write code |
| Atomic dir landing | `util/atomic_dir.rs` | existing `Io`/dedicated code at its callers |

**Per-sink error code rule** (the CON-1 precedent, §exit-codes-p7): the primitive's refusal maps to the **caller's dedicated** write-guard code — never a regression to generic `Io` (7) on a dedicated sink. (Sequencing: this slice lands **after** the `harness/sync.rs` decomposition, so the sinks live in their clean per-sink modules.)

## Test obligations

- `tests/symlink_intermediate_guard.rs`: place a symlink as an **intermediate** directory component on each sink's write path; assert the write is **refused** with that sink's dedicated code, and the final-node refusal still holds. SC-006: refused 100% across supported platforms.
- Fixture gating: construct the symlink fixture under `#[cfg(target_os = "linux")]` where APFS would reject a non-UTF-8/edge fixture at `mkdir(2)` (Phase 4 P3); the production check is platform-independent. macOS exercises the portable `openat`+`NOFOLLOW` walk.
- If the fallback path is taken, a test asserts final-node refusal + a doc note records the trust-model mitigation (NFR-004 holds).

## Trust-model framing (carried into FR-010)

This is **defence-in-depth**, not a normal path: the operator owns/creates the harness + project directory trees; plugin content never supplies path *components* (only the final filename, validated as one safe segment + `target.parent() == Some(dir)`); an attacker who can swap an intermediate-dir symlink mid-sync already holds operator filesystem privileges. The security doc (FR-010) states this plainly.

## Anti-requirements

- MUST NOT add a new top-level package (no `cap-std`, no direct `libc`).
- MUST NOT leave any sink on the old final-node-only check after the consolidation slice.
- MUST NOT regress a dedicated sink's refusal to generic `Io` (7).
