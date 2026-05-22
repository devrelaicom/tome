# Summariser — Contract

**Spec source**: [spec.md FR-420 through FR-427](../spec.md)
**Research**: [research.md R-2, R-3, R-14, R-15](../research.md)

## Model

| Property | Value |
|----------|-------|
| Model | `Qwen2.5-0.5B-Instruct` |
| Format | GGUF |
| Quantisation | Q4_K_M (~400 MB on disk) |
| Licence | Apache 2.0 |
| Runtime | `llama-cpp-2` Rust bindings (static-linked `llama.cpp` CPU-only) |
| Storage | `<root>/models/qwen2.5-0.5b-instruct/model.gguf` with `manifest.json` sibling |
| Identity recorded in `index.db.meta` | `summariser_name = "qwen2.5-0.5b-instruct"`, `summariser_version = "<pinned>"` |

Downloaded via the same `embedding::download` pipeline as BGE models — pinned SHA-256, atomic-rename, byte-progress indicator (TD-010 from P10 retro lands here).

## Runtime singleton

`llama-cpp-2` requires a process-wide `LlamaBackend` instance:

```rust
use std::sync::OnceLock;
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
pub fn backend() -> Result<&'static LlamaBackend, TomeError> {
    BACKEND.get_or_try_init(|| {
        LlamaBackend::init().map_err(|e| TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed { source: e.to_string() }
        })
    })
}
```

"Unload after use" (FR-421) refers to dropping the `LlamaModel` and `LlamaContext`, NOT the backend. The backend lives for the lifetime of the process.

## Trait

```rust
pub trait Summariser: Send + Sync {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError>;
}

pub struct PluginSummariesInput { pub plugins: Vec<PluginSummaryItem> }
pub struct PluginSummaryItem { pub catalog: String, pub plugin: String, pub description: String, pub skills: Vec<SkillSummaryItem> }
pub struct SkillSummaryItem { pub name: String, pub description: String }

pub struct SummariserOutput { pub short: String, pub long: String }
```

See [data-model.md §13](../data-model.md) for full type definitions including the test-side `StubSummariser`.

## Prompts

Two prompts run sequentially; both are `&'static str` constants in `src/summarise/prompts.rs`.

### Short prompt (target output ~400–800 chars)

```text
You are summarising a developer's skill library. Given the descriptions below,
produce a single comma-separated phrase listing the topics these skills cover.
No prose, no lead-in, no bullet points. Maximum 700 characters.

Skill descriptions:
{descriptions}
```

`{descriptions}` is substituted via `format!` with one line per skill: `<plugin>: <skill-name> — <skill-description>`.

### Long prompt (target output ~1500–2500 chars)

```text
You are writing a short rules section for an AI coding agent. The agent has access
to a search tool that retrieves skills relevant to a task. Below are the topics the
user's skill library covers. Write a 4–6 sentence rules section that
(1) tells the agent which topics the skill library covers,
(2) instructs the agent to call the search_skills tool when working on tasks
   involving those topics,
(3) is written for the agent to read at session start.
Plain prose, no headings, no bullet points. Maximum 2400 characters.

Topics:
{topics}
```

`{topics}` is substituted with the short summary's output (cascading from short to long; the long prompt benefits from the short summary's already-compressed topic list).

## Length windows

| Output | Min | Target | Max (warning emitted above) | Fatal-failure | 
|--------|-----|--------|------------------------------|---------------|
| Short summary | 1 char (non-empty) | 400–800 chars | 800 chars (tracing `warn!`) | 0 chars OR unparsable → exit 20 |
| Long summary | 1 char (non-empty) | 1500–2500 chars | 2500 chars (tracing `warn!`) | 0 chars OR unparsable → exit 20 |

Per FR-425, a too-long short summary that gets embedded into the MCP tool description is a tracing warning, not a hard error — the value is still cached and used.

## Triggers (FR-423)

Summary regeneration runs after:

1. `tome plugin enable` (workspace's enabled-plugin set changed).
2. `tome plugin disable` (same).
3. `tome plugin reindex` if any skill's `content_hash` changed in the resolved workspace's enabled set.
4. `tome catalog update` triggering reindex passes that change content hashes (per-workspace trigger).
5. `tome workspace regen-summary <name>` (explicit).

The MCP server does NOT trigger regeneration in-process (FR-425). It reads the cached short summary from the workspace's settings file at startup; missing cache → use scaffold-only tool description.

## Forward-progress on failure (FR-385)

When summariser fails during enable/disable/reindex/catalog-update triggers:

1. The underlying skill-state mutation (workspace_skills row insert/delete) MUST be committed before the summariser is invoked.
2. The summariser failure exits with code 20.
3. The workspace's existing cached summary (if any) is left in place — partial cache is better than no cache.
4. Doctor reports the summariser subsystem as broken AND the workspace's cached summary as stale.

The developer can re-attempt by running `tome workspace regen-summary <name>` after fixing the underlying cause (download the model, fix the checksum, etc.).

`regen-summary` is the exception: failure here is the result of the command (not a side-effect); cached summary is not modified.

## Inference invocation

```rust
impl Summariser for LlamaSummariser {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        let backend = backend()?;
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(backend, &self.model_path, &model_params)
            .map_err(/* model load failure → ModelMissing or ChecksumMismatch */)?;
        let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(4096));
        let mut ctx = model.new_context(backend, ctx_params)?;

        let descriptions = format_input_descriptions(input);
        let short = run_inference(&mut ctx, &model, SHORT_PROMPT.replace("{descriptions}", &descriptions), MAX_SHORT)?;
        check_length_window(&short, ShortOrLong::Short);

        let topics = short.clone();
        let long = run_inference(&mut ctx, &model, LONG_PROMPT.replace("{topics}", &topics), MAX_LONG)?;
        check_length_window(&long, ShortOrLong::Long);

        // model + ctx drop here; backend stays alive
        Ok(SummariserOutput { short, long })
    }
}
```

`run_inference` is a thin wrapper around llama-cpp-2's decode + sample loop with a `max_tokens` budget translating from the character maxima. Sampling parameters: `temperature = 0.3`, `top_p = 0.9`, `repeat_penalty = 1.1` (deterministic-leaning but not so cold the model hedges).

## Cache shape in `settings.toml`

```toml
[summaries]
short = "..."
long = "..."
generated_at = 2026-05-14T15:00:00Z
```

`generated_at` is RFC 3339. The cache is rewritten atomically when summarisation succeeds; failure leaves the prior cache in place.

## CI vs production

- **CI tests**: every test uses `StubSummariser` (deterministic, no model load). The stub records call counts so tests can assert "summariser invoked exactly N times" for trigger correctness.
- **One CI-skipped real-model test**: `tests/summariser_real.rs` runs only when `TOME_TEST_REAL_MODELS=1` is set (developer-machine pass). Asserts that a fixture workspace's input produces summaries within the length windows.
- **No real-model load in CI** — same discipline as the embedder + reranker.

## Test coverage

- `tests/summariser_stub.rs` — stub correctness + call-count assertions.
- `tests/summariser_triggers.rs` — every trigger from FR-423 invokes the stub exactly once.
- `tests/summariser_forward_progress.rs` — enable + simulated stub failure leaves skill state committed, cache untouched, exit 20.
- `tests/summariser_cache.rs` — cache write/read round-trip; `generated_at` updates on regeneration.
- `tests/summariser_real.rs` (CI-skipped) — real-model produce-and-validate.
