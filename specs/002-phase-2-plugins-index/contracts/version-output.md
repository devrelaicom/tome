# `tome --version` — Phase 2 Extension

Phase 1 shipped a minimal `--version` from `clap`'s derive macro: a single line with the crate version. FR-055 requires the version output to also identify the configured embedder and reranker.

## New default output

```
$ tome --version
tome 0.2.0
embedder: bge-small-en-v1.5 1.5
reranker: bge-reranker-base base
```

Three lines. Each model line is `<role>: <name> <version>` with a single space separator. No colour. No structured output by default — the Unix tradition for `--version` is plain text.

## `--version --json`

```json
{
  "tome": "0.2.0",
  "embedder": { "name": "bge-small-en-v1.5", "version": "1.5" },
  "reranker": { "name": "bge-reranker-base", "version": "base" }
}
```

Used by bug-report templates, status commands, and CI gates that pin model versions.

## Rationale

A `tome query` result's meaning depends on which embedder and reranker produced and ranked the vectors. When a developer files a bug, the trio (Tome version, embedder version, reranker version) is the minimum reproducibility set. Pinning all three in `--version` makes this trivially copyable.

## Implementation note

The model identities are compile-time constants pulled from `MODEL_REGISTRY` in `src/embedding/registry.rs`. Adding a model bump to the registry automatically bumps `--version` output. A test in `tests/version_output.rs` asserts the format and that both models are present.
