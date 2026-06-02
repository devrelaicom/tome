# Release binary size

Recorded post-build measurements of `target/release/tome`. The Tome
constitution (v1.2.0+) enforces a hard cap of **50 MB** for the release
binary; CI re-measures on Linux x86_64.

Reproducible measurement command:

```sh
cargo build --release
stat -f '%z' target/release/tome      # macOS (Darwin)
stat -c '%s' target/release/tome      # Linux
```

| Version  | Platform           | Bytes       | MiB     | Notes                                                                   |
| -------- | ------------------ | ----------- | ------- | ----------------------------------------------------------------------- |
| v0.3.0   | macOS arm64        | ~23 100 000 | ~22.0   | Phase 3 close, post-PR-H polish phase                                   |
| v0.4.0-rc | macOS arm64       | 27 595 936  | 26.31   | Phase 4 / Polish PR-E (post S-M1–S-M7 + T-M8 + T416 + T419)             |
| v0.5.0-dev / F2 | macOS arm64  | 27 595 888  | 26.31   | Phase 5 / F1+F2 (5 new TomeError variants pre-allocated; `regex` already direct from Phase 1's catalog::git scrubber — no real promotion needed) |
| v0.6.0   | macOS arm64        | 28 071 936  | 26.77   | Phase 7 / REL1 post crate-rename `tome-mcp`; functionally identical binary; under the 50 MB cap |

## Phase 5 size accounting

Phase 5 introduces **no new top-level dependencies**. The originally-planned
"promote `regex` from transitive to direct" reduced to a no-op: `regex = "1"`
has been a direct dep since Phase 1 (used by `catalog::git::scrub_credentials`).
F1's 5 new `TomeError` variants are pure additions to a `#[derive(Debug)]`
enum — the compiler folds the unused variants out entirely until production
consumers wire them in subsequent slices.

## Phase 4 size accounting

Phase 4 added two direct dependencies that contributed to the size:

1. **`llama-cpp-2 = "=0.1.146"`** — bundled local-LLM inference for the
   summariser (US4). Pulls in the llama.cpp C++ runtime statically.
2. **`encoding_rs = "0.8"`** — required by `llama-cpp-2`'s token
   decoder API (not re-exported, so we depend on it directly).

Plus the `toml_edit` crate's surface widened to cover the harness
MCP-config + workspace marker rewrite paths (Phase 3 used it only for
catalog config; Phase 4 has four consumers now).

The summariser model itself (Qwen2.5-0.5B-Instruct, ~491 MB) is **not**
bundled in the binary — it's downloaded on demand by
`tome models download summariser` and lives under
`<home>/.tome/models/summariser/`.

## When to update this file

- Every release (v0.x.0): add a new row.
- Any PR that adds a direct dependency: re-measure on macOS arm64 and
  update if the delta exceeds 200 KiB.
- CI surfaces the Linux x86_64 measurement on every push to `main`; if
  it crosses 80% of the 50 MB cap (40 MB), open an issue.
