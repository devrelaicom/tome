# Git hooks for Tome

Native, version-controlled git hooks. No external hooks manager. To opt in
once per clone:

```sh
git config core.hooksPath .githooks
```

After that the three hooks run automatically on every commit and push.

## What runs when

| Hook | Command |
|---|---|
| `pre-commit` | `cargo fmt --check`, `typos`, `cargo clippy --all-targets --all-features -- -D warnings` |
| `commit-msg` | `cog verify --file <commit-msg-file>` (Conventional Commits) |
| `pre-push` | Mirrors `pre-commit`: `cargo fmt --all -- --check`, `typos`, `cargo clippy --all-targets --all-features -- -D warnings` |

These mirror the gates the constitution requires (§Lints, §IX, §X). No hook
runs the test suite — duplicating it locally costs 30+ minutes for no signal
CI doesn't already produce (see the `pre-push` header comment); the full
suite + build matrix runs in CI on every PR. Locally the hooks are a fast
feedback loop; upstream CI remains the source of truth.

## Why not lefthook?

Phase 2 ran into a reproducible deadlock when `git push` invoked
`lefthook → cargo test` under this repo's command-wrapping layer
(`specs/002-phase-2-plugins-index/retro/P2.md`). The constitution says
"inherit, don't reimplement" (principle XII) and "boring, idiomatic Rust
beats novel and clever" (principle VI). Git's own `core.hooksPath`
mechanism is the boring option and removes one external moving part.

## Bypassing for a known reason

`git commit --no-verify` and `git push --no-verify` exist. Routine use is a
smell. If you need to bypass, explain why in the commit message body.

## Contributing

When you add or modify a hook, also update:

- This README's command table.
- `CONSTITUTION.md` §Development Workflow if the *gates* change (not just
  the implementation).
- `CLAUDE.md` "Common Commands" section if a new command becomes worth
  surfacing.
