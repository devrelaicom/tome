<!-- Keep PRs small: ~400 lines / two modules. Bigger change? Split it. -->

## What this changes

<!-- One or two sentences. Link the issue it fixes: Fixes #NNN -->

## Checklist

- [ ] The PR title is a Conventional Commit (`type(scope): subject`) — squash-and-merge uses it.
- [ ] `cargo fmt --all -- --check` is green locally.
- [ ] `CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features -- -D warnings` is green locally.
- [ ] `typos` is green locally.
- [ ] `CARGO_INCREMENTAL=0 cargo test --no-fail-fast` is green locally.
- [ ] Documentation lands in this PR — README, command help text, `site/docs/`, and the changelog where they apply.
- [ ] If the change adds a dependency or embeds new assets, note whether it could affect the release binary size (CI asserts a 50 MB cap).
