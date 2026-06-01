# Phase 7 Quickstart — Beta Hardening and Public Release

**Branch**: `007-phase-7-beta-release` | **Date**: 2026-06-01
**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md)

This is a **mature brownfield repo** — the toolchain already exists (clippy/fmt/typos in pre-commit + CI, cocogitto Conventional Commits, `cargo-deny`/`cargo-audit`, the full test suite). Phase 7 stands up **no new project tooling**; it adds two **release-time CI tools** (cargo-dist, cargo-about) and a small set of phase-specific validation steps. This file is the practical "how to build, validate, and release" guide.

## Development setup (existing — verify, don't re-create)

```sh
git clone https://github.com/devrelaicom/tome.git      # repo name unchanged this phase
cd tome
git config core.hooksPath .githooks                    # one-time, per clone
cargo build                                            # debug
```

### Quality gates (also in `.githooks/pre-commit` + CI)

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings   # -D promotes doc-lints plain `cargo test` ignores
typos
cargo test                                                 # full suite (~20–25 min locally; CI is the gate)
cargo audit
cargo deny check                                           # advisories + bans + licences + sources
cog verify --file <commit-msg-file>                        # Conventional Commits
```

> **Local test timing**: the full suite is slow locally. Use targeted runs (`cargo test --test <file>`) per slice and let CI run the matrix. The pre-push hook is deliberately slim (<1 min) — don't reverse it.
> **ubuntu/MSRV CI flake**: heavy C/C++ builds intermittently fail with bus-error/no-space on the smallest runner — infra, not code; `gh run rerun --failed` clears it. The cargo-dist matrix lands on these same runners; budget retry tax.

## Phase-7-specific validation

### F2 — the `rustix` symlink-primitive spike (gates FR-007)

```sh
# Confirm rustix is already transitive (NO new package expected):
cargo tree -i rustix -e features        # expect rustix v1.1.4 via tempfile (default) + crossterm; `fs` enabled

# After promoting to a direct dep (`rustix = { version = "1", features = ["fs"] }`),
# the spike test confirms the symlink-safe primitive is reachable:
cargo test --test symlink_intermediate_guard      # exercises openat2(RESOLVE_NO_SYMLINKS)/openat+O_NOFOLLOW
cargo deny check licenses                          # rustix (Apache-2.0 OR MIT) stays on the allowlist
```

- **Spike passes** → harden the intermediate-component walk on every sink (R2).
- **Spike fails** → FR-007 degrades to final-node `O_NOFOLLOW` + the documented trust-model mitigation (no new package either way; NFR-004). Record the outcome against FR-007.

### K1 — the one-time real-model recall check (SC-001)

The stub embedder cannot prove recall. Run the real-model check **once** (it downloads the BGE models on first use; it is **not** in the fast CI suite):

```sh
# Real embedding models (downloads ~325 MB on first run into ~/.tome/models/):
cargo test --test search_knn_recall_realmodel -- --ignored --nocapture
# Verifies: on a realistically-populated multi-workspace index where ≥top_k nearer
# vectors are filtered out, the matching entry is present (0 → present) and the count
# does not shrink as the corpus grows.
```

The fast regression (`tests/search_knn_recall.rs`, stub) runs in the normal suite and must place ≥`top_k` nearer non-matching rows ahead of the match on a corpus large enough that a fixed-multiplier over-fetch would still miss it.

### Per-slice validation (targeted)

```sh
# US1 beta gate
cargo test --test search_knn_recall            # FR-001 (stub)
cargo test --test doctor_readonly_schema       # FR-002 (no abort, no lock, no unlocked migration)
cargo test --test catalog_ssh_roundtrip        # FR-003 (scrubbed-URL cache key; zero orphaned clones)
cargo test --test prompt_collision_global      # FR-004 (command+skill+foo2 all resolvable)
cargo test --test workspace_toml_control_chars # FR-005 (newline-bearing catalog name)

# US2 robustness
cargo test --test bounded_reads                # FR-006 (oversized files across the site list → named error)
cargo test --test symlink_intermediate_guard   # FR-007 (intermediate-component refusal, sink-specific code)
cargo test --test rules_opencode_inline        # FR-008 (OpenCode gets the inline body)
cargo test --test catalog_remove_toctou        # FR-009 (re-derive cascade in lock)

# US3 decomposition + harness + cleanup — the decomposition's evidence is the UNCHANGED suites:
cargo test --test sync_idempotence --test harness_sync_p6_idempotence --test harness_sync_p6_first_error
cargo test --test exit_codes_e2e_mcp           # FR-012 (GAP-1 codes 9, 26–29 + FR-004 end-to-end)
cargo test --test exit_codes --test exit_codes_e2e   # closed-set guard gains NO new arm (NFR-002)
```

### Behaviour-preservation gate for the decomposition (FR-011, NFR-005)

The decomposition is **strictly behaviour-preserving**. Its proof is that the pre-existing suites stay **green and unchanged**:

```sh
# Before AND after each decomposition sub-slice (D.a/D.b/D.c) — must be identical results:
cargo test --test sync_idempotence
cargo test --test harness_sync_p6_idempotence
cargo test --test harness_sync_p6_first_error
cargo test sync_outcome_json_shape           # the SyncOutcome wire-pin must not move a field
```

If any of these changes behaviour, the refactor regressed (most likely the mass-delete safeguard or the first-error precedence).

## Release pipeline (US4/US5 — wired, not triggered)

> **Gated on the FR-023 constitution amendment being merged first.**

```sh
# Crate rename verification (REL1):
cargo check                                  # regenerates Cargo.lock `name` BEFORE committing (avoid dirtying --locked)
cargo build --release --locked
ls target/release/tome                       # binary name stays `tome` (via [[bin]] name = "tome")
./target/release/tome --version              # correct post-rename (CARGO_PKG_NAME = tome-mcp, command = tome)

# Self-contained check (FR-018) — no application-specific sidecar:
ldd target/release/tome    | grep -i onnx || echo "no libonnxruntime — self-contained"   # Linux
otool -L target/release/tome | grep -i onnx || echo "no libonnxruntime — self-contained" # macOS

# Binary size (FR-020) — record in RELEASE-BINARY-SIZE.md:
stat -c '%s' target/release/tome             # Linux  (cap: 50 MB)
stat -f '%z' target/release/tome             # macOS

# Packaging hygiene (FR-024):
cargo package --list --locked                # confirm vendor/sqlite-vec + build.rs ship; internal dirs excluded
cargo publish --dry-run --locked             # must exit 0 (publish itself is USER-RESERVED)

# Third-party licence bundle (FR-019/NFR-007):
cargo about generate about.hbs > THIRD-PARTY-LICENSES   # then append native notices (ONNX/llama.cpp/sqlite-vec)

# Getting-started smoke (SC-008) — automated check uses a file:// fixture:
tome catalog add "file://$(pwd)/tests/fixtures/<demo-catalog>"   # decoupled from the public catalog fork
# ...run every README getting-started command end-to-end (rebuild the binary first — HYG-7).
```

**User-reserved (do NOT run from the orchestrator)**: `cargo publish` (final), the `v0.6.0` git tag, the Homebrew-tap PR merge, and release-notes posting. **Operator-only** (cannot be scripted here): enable GitHub private vulnerability reporting; set repo description + topics; provide the least-privilege Homebrew-tap PAT secret.

## CI / release workflows

- `actions/checkout@v4 → @v5` at all three sites (`ci.yml:22`, `security.yml:20,38`) — **time-sensitive** (F3, early).
- The cargo-dist-generated `.github/workflows/release.yml` is subject to the same gates as CI (fmt/clippy/`cargo-deny`, version-pinned actions incl. the upgraded checkout) and builds `--locked` (NFR-008/010).

## Closeout cadence (per the project discipline)

- Per-slice review pass appropriate to risk (the decomposition + the symlink-guard one-pass slices get the full 4-reviewer treatment); findings + disposition committed **before** fixes.
- `/sdd:map incremental` after the decomposition (structural diff to ARCHITECTURE/STRUCTURE/CONCERNS) and at phase close.
- Phase-wide 4-reviewer pass at Polish over the assembled surface; retro fill + `CLAUDE.md` update **before** the FR-024 untracking step renders `CLAUDE.md` untracked.
