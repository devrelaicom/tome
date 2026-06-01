# Tome — Public MVP Release Readiness Report

**Date:** 2026-05-29 · **Target:** first public MVP release (crates.io publish + tagged GitHub release + an install/usage story an external user can follow) · **Audited version:** 0.6.0 (untagged, never published)

**Method:** an 8-agent workflow — 7 parallel evidence-gathering finders (build gate, packaging, licensing, security, docs/UX, feature-completeness, CI/release) each running real commands, plus a completeness/severity critic over the full set — followed by two direct maintainer verifications (repo visibility, crates.io name availability). Every claim below is backed by a command or `file:line`.

---

## Verdict: **NEARLY READY — not blocked on the product, blocked on the release wrapper**

Tome the *product* is genuinely shippable: every quality gate passes, security is strong, and there are essentially zero code defects or unimplemented features. **The gaps are all in the release/distribution/docs/metadata layer**, not the code.

Two hard blockers and a short list of should-fix items stand between the current state and "a member of the public can discover, install, and use Tome." With focused effort the whole list is roughly a **half-day to a day** of work (excluding optional release automation and prebuilt binaries).

---

## ✅ What's already solid (verified green)

| Area | Evidence |
|---|---|
| **Release build** | `cargo build --release` → exit 0; clean compile of all native deps (bundled SQLite, ONNX Runtime, llama.cpp, vendored sqlite-vec). |
| **Binary size** | **26.85 MB** (28,150,336 B) — ~46% under the 50 MB cap (NFR-001). Fresh, post-Phase-6 artifact. |
| **Lint / format** | `cargo clippy --all-targets --all-features -- -D warnings` → 0 warnings (also proves the whole test suite *compiles*). `cargo fmt --check` clean. |
| **Publishability** | `cargo publish --dry-run` → exit 0. Tarball **1.8 MiB compressed** (well under the 10 MiB limit); the isolated clean-checkout verify build compiled in 58 s, **proving `vendor/sqlite-vec` + `build.rs` ship correctly and the crate builds from the tarball alone**. |
| **CI** | `main` is **green** (run 26642955829): fmt/clippy/build/`test --workspace` pass on ubuntu+macOS × stable+MSRV; the 50 MB size gate passes on ubuntu/stable. Security workflow (cargo audit + cargo deny) green. |
| **Security** | `cargo audit` → **0 active vulnerabilities** (3 unmaintained-transitive warnings only). No telemetry/analytics/crash-reporting anywhere. Model downloads are HTTPS + SHA-256-pinned and **prompted, not automatic**. Credential scrubber wired into download error/diag paths. `git` shell-outs are argv-based (no injection). 0600 perms + symlink refusal on sensitive files. |
| **Code hygiene** | **Zero** real TODO/FIXME/`unimplemented!`/`todo!`/`dbg!` in `src/` or `tests/`. All 12 `panic!` are test-only; the one `unreachable!` is a dispatch guard. ~45 production `unwrap()`/`expect()` are all idiomatic-safe (static regex, const invariants, mutex poison) — **no reachable-on-user-input panic**. Stubs are `#[doc(hidden)]` + LTO-eliminated. All 12 advertised commands exist and execute. |
| **Licensing (legality)** | Dual MIT OR Apache-2.0; both LICENSE files present and consistent. `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok` across 474 packages. No GPL/AGPL. Vendored sqlite-vec ships its own licenses. `tome models list` shows correct per-model licenses. |

---

## 🚫 Blockers — must resolve before a public MVP

### B1 — crates.io name `tome` is **already taken** → `cargo publish` will fail
- **Evidence (verified live):** `GET https://crates.io/api/v1/crates/tome` returns an existing crate — created 2021-08-16, `"description":"WIP. A simple installer."`, 2 versions, `max_version 0.0.0`, **`yanked: true`**, owned by an unrelated account.
- **Why it blocks:** crates.io **does not release a name when its versions are yanked** — ownership is permanent. A non-owner `cargo publish` to `tome` is rejected (403). This gates the entire crates.io distribution channel, which the project's metadata and the "`cargo publish` reserved for user push" note clearly intend to use.
- **Fix (pick one):**
  - **(Recommended, fast)** Publish under a different *crate* name (e.g. `tome-cli`, `tome-plugins`, `tomekit`) while keeping the *command* `tome` via `[[bin]] name = "tome"` in Cargo.toml. Users run `cargo install <new-name>` and still get a `tome` binary. Propagate the chosen name to README, `repository`, and docs **before** publishing.
  - **(Slower, uncertain)** Request the abandoned name via the crates.io team's [policy for reclaiming names](https://crates.io/policies) — discretionary, not timeline-safe for an MVP.
- **Effort:** small (a naming decision + find/replace) · **Owner decision required.**

### B2 — README's first getting-started command points at a 404 repo (`DOC-01`)
- **Evidence:** `README.md:21` `tome catalog add midnight/midnight-experts`; `gh repo view midnight/midnight-experts` → 404 (also `devrelaicom/midnight-experts` → 404). `tome catalog add` resolves `owner/repo` to a clone URL, so the **very first copy-paste command in the Quick example fails** for every public user — even those installing from source.
- **Fix:** point the worked example at a real, public, stable catalog (or a `file://` local-path example that always works), then **run every command in the Quick-example block end-to-end** before release. If no public catalog exists yet, ship a tiny demo catalog so the getting-started path is real.
- **Effort:** small.

---

## ⚠️ Should-fix — before a *polished* public launch (majors)

### M1 — No publish/install path for the public *(merge of REL-1, REL-2, PKG-6, DOC-02)*
The crate is unpublished, there are **no git tags and no GitHub releases**, install is source-only (`git clone` + `cargo install --path .`), and there are no prebuilt binaries or `cargo binstall` metadata. So today `cargo install tome` doesn't exist and there's no binary to download.
- **Fix:** after B1's naming decision, `cargo publish`; add `cargo install <name>` as the primary README install line (keep `--path .` as fallback). Strongly consider **prebuilt binaries** (e.g. [`cargo-dist`](https://opensource.axo.dev/cargo-dist/)) so users skip the heavy native build entirely.
- **Note:** correctly **non-blocking** — the manual publish path works (dry-run passed) — but it's the core of the "public can install" criterion.
- **Effort:** small (publish + README) / medium (binaries + release.yml).

### M2 — Build prerequisites are undocumented for a heavy native build (`DOC-02`)
Install docs list only "Rust ≥ 1.93 + git", but the build pulls a **C/C++ compiler + CMake** (`llama-cpp-sys-2`) and **downloads a prebuilt ONNX Runtime at build time** (`ort` `download-binaries`). A clean machine without a toolchain — or behind a proxy that blocks the ORT download — fails with no warning.
- **Fix:** add a Prerequisites section (macOS: Xcode CLT; Debian/Ubuntu: `build-essential` + `cmake`), note the build is slow and fetches the ORT binary, and state platform support explicitly. (Downgraded from blocker → major: the from-source build genuinely works on an equipped machine, and the realistic first audience is developers.)
- **Effort:** medium.

### M3 — Security disclosure channel is dead on an already-public repo *(merge of ADD-1, DOC-04)*
`CONTRIBUTING.md:66` lists `security@example.invalid` (a literal placeholder), and there's **no `SECURITY.md`** / no GitHub private vulnerability reporting. The repo is **already public** and invites contributions, so there is currently no working way to report a vulnerability privately to a security-conscious codebase.
- **Fix:** add `SECURITY.md` + enable GitHub's private vulnerability reporting (Settings → Code security), and replace the placeholder email. (Major; non-blocking for install-and-use, but important the day the repo invites public scrutiny — which it already does.)
- **Effort:** trivial.

### M4 — No third-party license bundle for the binary distribution (`LIC-001`)
The binary statically links MIT/BSD/Apache-2.0 code (ONNX Runtime, llama.cpp, sqlite-vec, ~470 crates). Those licenses require carrying upstream notices in binary redistributions; Tome ships its own LICENSE files but no aggregated notice.
- **Fix:** generate `THIRD-PARTY-LICENSES` (e.g. `cargo about` or `cargo-bundle-licenses`) and attach it to each tagged binary release (optionally `tome --licenses`). Not legally blocking (all permissive) but a standard compliance obligation for shipping a compiled binary.
- **Effort:** small.

---

## 🔧 Nice-to-have — cheap polish (minors)

| ID | Item | Fix | Effort |
|---|---|---|---|
| **Packaging hygiene** *(5× reported: PKG-1/SEC-01/LIC-003/DOC-03/REL-3 — **not** release-blocking; crate builds fine)* | Tarball ships 578 files incl. ~2.8–4.4 MB of `specs/`, `tests/`, `PRDs/`, `retro/`, `review/`, `.sdd/`, `CLAUDE.md`, `constitution-review-report.md`, and two tracked `*.local.json` dev-state files. | Add a Cargo `include = [...]` allowlist (`src/**`, `build.rs`, `vendor/**`, `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `LICENSE-*`); `git rm --cached` the two `.local.json`; **re-run `cargo package --list` to confirm `vendor/sqlite-vec` still ships**. | small |
| PKG-2 | `docs.rs` lib-doc build will fail (`ort` downloads ONNX Runtime in a network-isolated sandbox). | Add `[package.metadata.docs.rs]` or feature-gate the fastembed embedder so docs build with it off. (Downgraded major→minor: docs.rs isn't on the install path.) | medium |
| DOC-05 / HYG-6 | README line 3 "(and eventually an MCP server)" — MCP shipped in Phase 3. | "A Rust CLI **and** MCP server…" | trivial |
| LIC-002 | README:15 mislabels Qwen2.5-0.5B as MIT; it's **Apache-2.0** (the tool itself is correct). | One-word fix. | trivial |
| ADD-4 | No explicit no-telemetry/network statement despite it being a genuine strength. | Add a one-line Privacy/Network note to README. | trivial |
| PKG-3 | README repo-relative links (`./PRDs/`, `./LICENSE-*`, …) 404 on the crates.io page. | Rewrite to absolute `https://github.com/...` URLs. | trivial |
| DOC-06 | `--help` text leaks internal refs (`contracts/*.md`, `FR-/NFR-` numbers). | Strip internal citations from `///` clap doc-comments in `src/cli.rs`. | small |
| ADD-6 / DOC-08 | GitHub repo has **no description and no topics** — invisible to discovery (the MVP's first criterion). | `gh repo edit --description … --add-topic rust,cli,mcp,claude-code,ai,plugins`. | trivial |
| CI-2 | `actions/checkout@v4` (Node 20) — forced to Node 24 by **2026-06-02** (~4 days out). | Bump to `@v5` in both workflows. | trivial |
| PLAT-1 | Windows untested and unstated. | Add "Supported platforms: Linux & macOS (x86_64/aarch64); Windows untested." (Defer Windows — correct call.) | trivial |
| REL-4 / DOC-08 | `CHANGELOG.md` has a mis-ordered `[Unreleased]` header and internal jargon; `RELEASE-BINARY-SIZE.md` lacks a v0.6.0 row. | Reorder `[Unreleased]` to top, trim jargon for public notes, add the size row. | trivial |
| PKG-4 | Missing `authors` / `homepage` / `documentation` in Cargo.toml. | Add them. | trivial |
| ADD-5 | No `CODE_OF_CONDUCT.md` / issue+PR templates. | Optional OSS furniture. | small |
| HYG-7 | Local debug binary is stale (`0.5.0`); source is correct (`env!("CARGO_PKG_VERSION")`). | Rebuild before tagging so smoke tests reflect 0.6.0. | trivial |

---

## 🧭 Decisions only you can make

1. **Crate name (blocks publish):** `tome` is taken on crates.io. Rename the published crate (keeping the `tome` command) or pursue a transfer? *(See B1.)*
2. **Version:** Keep **0.6.0** for the first public release — recommended (honest vs the CHANGELOG's documented 0.1.0→0.6.0 history; avoids both the dishonesty of resetting to 0.1.0 and the over-promise of 1.0.0). Stay in 0.x while the `ort` release-candidate dep and recent MSRV settle. *(ADD-3)*
3. **Already-public repo:** The repo has been public since 2026-05-11, so all internal artifacts (`specs/`, ~42 `review/disposition` files, `retro/`, `constitution-review-report.md`, `CLAUDE.md`, the two `.local.json`) are **already publicly visible**. They appear benign (process docs + null tool-state, no secrets per multiple finders), but confirm you're comfortable with that — the Cargo `include` fix only keeps them out of the *crate tarball*, not the repo.
4. **Distribution scope:** crates.io + prebuilt binaries, or a source-only MVP first? Drives whether M1's release automation is in scope now.
5. **GitHub license display:** GitHub detects only Apache-2.0 (cosmetic; the LICENSE files are authoritative). Leave as-is or add a license note.

---

## 📋 Suggested sequence (release runbook)

0. **Resolve the crate name** on crates.io (B1) — do this *first*; it propagates everywhere.
1. **Fix DOC-01** (B2): real catalog or `file://` example; smoke-test every Quick-example command end-to-end.
2. **Security channel** (M3): `SECURITY.md` + enable private reporting; fix the placeholder email.
3. **README pass:** prereqs (M2), install line, platform statement, no-telemetry note, Qwen license, absolute links, drop the "eventually an MCP server" line.
4. **Cargo `include` allowlist** + `git rm --cached` the two `.local.json` + re-verify `vendor/sqlite-vec` ships.
5. **Repo metadata:** description + topics (ADD-6); bump `checkout@v5` (CI-2).
6. **Compliance/docs:** THIRD-PARTY license bundle (M4); `[package.metadata.docs.rs]` (PKG-2).
7. *(Optional)* `release.yml` / `cargo-dist` for tag → `cargo publish` + per-platform binaries + release notes (M1).
8. **Cut the release:** rebuild, `cargo publish --dry-run`, tag `v0.6.0`, `cargo publish` (final name), `gh release create v0.6.0` with **public-friendly** notes (what Tome *is* across six phases, not just the 0.6.0 delta).

---

*Generated with [Claude Code](https://claude.com/claude-code). Finding IDs (e.g. `DOC-01`, `REL-1`) trace to the underlying audit evidence; ask to expand any one for the exact command output and `file:line` citations.*
