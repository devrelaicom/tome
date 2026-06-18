# Embedded harness plugin shims

This tree holds the per-harness TypeScript **session-steering shims** Tome
ships for plugin/extension-based harnesses (Cline, Pi, OpenCode). Each shim is
embedded into the Tome binary at build time (`build.rs`, `include_bytes!`,
mirroring the Phase-9 `assets/meta-skills/**` pipeline) and written to the
harness's plugin directory by `src/harness/reconcile/plugins.rs` during sync.

## The shim contract

A shim is **executed by the harness's own JS/Bun runtime — never by Tome**.
This is load-bearing: Tome's `src/harness/` tree is sync-only
(`tests/sync_boundary.rs`), so Tome never spawns Node/Bun to run or validate a
shim. The shim is static embedded data from Tome's point of view.

Every shim MUST:

1. **Defer to the Rust directive source.** At session start it shells out to
   `tome harness session-start --harness <name>` (fixed, escaped command) and
   injects that stdout through the harness's plugin API. The directive bytes
   live in one place — `src/harness/routing.rs::build_directive` — never
   duplicated in TypeScript.
2. **No-op on a missing `tome` binary.** If `tome` is not on `PATH`, the shim
   silently injects nothing (no crash, no error text). A developer without
   Tome installed sees an inert plugin, not a broken session.
3. **Import nothing from npm.** Shims use only the harness-provided plugin API
   and the runtime's built-in child-process primitive. Zero `node_modules`
   dependency keeps them embeddable and tamper-evident.

## Layout

```
assets/harness-plugins/
├── README.md            # this file
├── cline/tome.ts        # registerMessageBuilder shim          (--harness cline)
├── pi/tome.ts           # before_agent_start return shim        (--harness pi)
└── opencode/tome.ts     # experimental.chat.system.transform    (--harness opencode)
```

Exactly **one** entrypoint file per harness directory; `build.rs` validates
that invariant and rejects any non-`Normal` relative path at build time.
