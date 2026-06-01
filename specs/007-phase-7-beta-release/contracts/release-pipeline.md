# Contract: Release Pipeline (US4)

**FRs**: FR-017, FR-018, FR-019, FR-020 · **NFRs**: NFR-007, NFR-008, NFR-010 · **SCs**: SC-007, SC-009 · **Research**: §R-12/13/14/15
**Gate**: requires the FR-023 constitution amendment **merged first** (see `constitution-amendment.md`).

The release wrapper the product has never had. Wired but **not triggered** — the actual `cargo publish`, git tag, tap-PR merge, and release-notes posting remain **user-reserved** actions.

---

## FR-017 — Crate `tome-mcp`, command `tome`

**Site**: `Cargo.toml`.

**Invariant**: the published crate MUST be `tome-mcp`; the installed binary and the Homebrew formula MUST be `tome`. `cargo install tome-mcp` and `brew install …/tome` both yield a `tome` command.

**Mechanism** (§R-12): `[package] name = "tome-mcp"` + `[[bin]] name = "tome"`. Treat the rename as its own slice (REL1) with a full `cargo check` + `--locked` re-verify, and a sweep of every `tome`-as-package reference:
- `Cargo.lock` `name` field (regenerate via `cargo check` **before** committing so the gate doesn't dirty the lock).
- CI workflows (the binary-size assertion reads `target/release/tome` — **stays valid** because the binary name stays `tome`).
- `tests/exit_codes_e2e.rs` (invokes the CLI binary by name).
- README install lines; the `--version`/`-V` pre-parse (`CARGO_PKG_NAME`).

**Test**: `cargo install tome-mcp` (or a path/dry-run equivalent) yields a runnable `tome`; `tome --version` / `-V` correct post-rename.

---

## FR-018 — Self-contained prebuilt binaries, verified per-target

**Invariant**: the pipeline MUST produce self-contained prebuilt binaries for **Linux + macOS (x86_64 + aarch64)** that run on a clean machine of the target with **no separately-installed inference runtime**; each target's build MUST verify the **absence of an application-specific sidecar** (e.g. `libonnxruntime`) by inspecting the binary's dynamic dependencies in CI.

**Mechanism** (§R-13): cargo-dist matrix over the four targets; a CI step runs `ldd` (Linux) / `otool -L` (macOS) and asserts no `libonnxruntime` (ONNX is statically linked via `ort`'s `download-binaries`; llama.cpp + vendored sqlite-vec are static). "Self-contained" = no application-specific sidecar; linking the platform's own C/C++ runtime + libc is acceptable, so the **Linux build targets a glibc baseline** so it runs across mainstream distributions (spec Edge Case). macOS is already verified self-contained; Linux confirmed per-target.

**Test/SC**: SC-007 — a clean Linux machine and a clean macOS machine each install with one command and run with no additional runtime-library install; the per-target dynamic-dependency check is the mechanical gate.

---

## FR-019 — Tagged release: archives + checksums + tap + crates.io + licence bundle

**Invariant**: a tagged release MUST produce per-platform archives with checksums, push a Homebrew formula to **`aaronbassett/homebrew-tap`** (a **least-privilege** credential with cross-owner write — §R-14), publish to crates.io, and attach an **aggregated third-party-licence document** covering **both** the cargo dependency graph **and** the vendored/statically-linked native components (ONNX Runtime, llama.cpp, vendored sqlite-vec).

**Mechanism** (§R-13/14/15): cargo-dist generates archives + checksums + the formula + the installer; the cross-owner tap push uses an operator-provided, tap-repo-write-only PAT (never logged, NFR-010). The licence bundle (`THIRD-PARTY-LICENSES`) is `cargo-about` over the cargo graph **plus** manually-appended native-component notices (those linked outside the cargo graph). `cargo-deny` stays green (NFR-007).

**User-reserved**: the actual `cargo publish`, the `v0.6.0` tag, the tap-PR merge, and release-notes posting are operator actions (the standing project discipline). The pipeline is wired and dry-run-validated; the trigger is the user's.

---

## FR-020 — Binary stays under the size cap

**Invariant**: the release binary MUST remain under the documented 50 MB cap.

**Mechanism**: CI keeps asserting `target/release/tome` size; record the post-rename size in `RELEASE-BINARY-SIZE.md`. `rustix` adds ≈0 (already compiled). Current ~27 MiB macOS arm64 / ~35–37 MiB Linux x86_64 — ample headroom. SC-009: size cap holds.

---

## NFR-008 — `--locked` everywhere; `Cargo.lock` authoritative + shipped

Release and CI builds MUST be `--locked`; `Cargo.lock` is committed, authoritative, and **shipped in the crate tarball**, so a tagged artifact matches the audited dependency set and no transitive bump silently enters a release.

## NFR-010 — Release workflows gated like CI; PAT least-privilege

The cargo-dist-generated `release.yml` MUST be subject to the same gates as CI (fmt/clippy/`cargo-deny`, version-pinned third-party actions **including the upgraded `actions/checkout@v5`**). The Homebrew-tap PAT MUST be least-privilege and **never logged**.

---

## Anti-requirements

- MUST NOT re-architect the inference linking to dynamic-load ONNX (`ort-load-dynamic` + `brew install onnxruntime`) — that pushes a runtime-library install onto the user, the opposite of self-contained.
- MUST NOT trigger `cargo publish`/tag/tap-merge from the orchestrator — user-reserved.
- MUST NOT let a release-tooling change unpin `ort` (RC) or `llama-cpp-2` (`=0.1.146`) or change the MSRV.
- MUST NOT log the PAT or any credential-shaped string (§XIII).
