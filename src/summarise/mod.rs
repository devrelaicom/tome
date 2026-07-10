//! Workspace summariser — Qwen2.5-0.5B-Instruct (GGUF) served via
//! `llama-cpp-2`. Compresses a workspace's enabled-plugin / skill
//! catalogue into a `(short, long)` summary pair that feeds the MCP
//! tool description and the harness-side `RULES.md`.
//!
//! F6 ships the **skeleton**: trait surface, types, prompt constants,
//! registry entry, deterministic [`StubSummariser`] for tests, and the
//! `LlamaBackend` singleton. The production `LlamaSummariser`
//! constructor returns a clearly-attributed failure so accidental
//! reachability before US4.a is loud, not silent.
//!
//! The `LlamaBackend` is process-wide (`llama-cpp-2` enforces this at
//! the C++ level — only one `LlamaBackend` may exist per process).
//! Tome reaches it through [`backend()`], which uses `std::sync::OnceLock`
//! to lazily initialise. Per-summarise invocations create a new
//! `LlamaModel` + `LlamaContext`, then drop them — the "unload after
//! use" guidance in `contracts/summariser.md` refers to the model +
//! context, **not** the backend.
//!
//! The whole module is on the sync side of the
//! `tests/sync_boundary.rs` invariant. `llama-cpp-2` is a sync API;
//! there's no async involved. Tokio is restricted to `src/mcp/`.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use llama_cpp_2::llama_backend::LlamaBackend;

use crate::config::Config;
use crate::error::{SummariserFailureKind, TomeError};
use crate::paths::Paths;
use crate::provider::Capability;

pub mod download;
pub mod llama;
pub mod prompts;
pub mod registry;
pub mod remote;
pub mod stub;
pub mod trigger;

pub use llama::LlamaSummariser;
pub use remote::RemoteSummariser;
pub use stub::StubSummariser;
pub use trigger::{regenerate_for_trigger, regenerate_for_trigger_with_summariser};

// ---------------------------------------------------------------------------
// Length-window constants — single source of truth (US4.d-1 / C-B3-R-B1).
//
// Before US4.d-1 these constants were duplicated in `prompts.rs`
// (`LONG_MAX_CHARS = 2400`) and `workspace::regen_summary`
// (`LONG_MAX_CHARS = 2500`) — the two warn predicates fired at different
// boundaries. The contract pins the long max at 2500 chars; US4.d-1
// unifies on that value and re-exports from `prompts.rs` and
// `regen_summary` so a single edit here moves both warn boundaries.
//
// `SHORT_TARGET_*` + `LONG_TARGET_*` (advisory windows, no warning
// emitted) stay private to `prompts.rs`; they are wiring details of the
// inference loop, not user-visible cache boundaries.
// ---------------------------------------------------------------------------

/// Hard upper bound for the short summary. Outputs strictly above this
/// emit a `tracing::warn!` (per `contracts/summariser.md` §"Length
/// windows"). Outputs at or below are cached without comment.
pub const SHORT_MAX_CHARS: usize = 800;

/// Hard upper bound for the long summary. Outputs strictly above this
/// emit a `tracing::warn!`. The MCP tool description / `RULES.md`
/// surface above this cap is still useful — the warning is advisory,
/// not a hard error (FR-425).
pub const LONG_MAX_CHARS: usize = 2500;

/// Workspace summariser interface. Implementations must be `Send + Sync`
/// so callers can stash one inside an `Arc<dyn Summariser>` and share
/// it across the summary-regeneration triggers in US4 (enable / disable
/// / catalog-update / reindex / explicit `regen-summary`).
///
/// `summarise` is the only required method. The trait deliberately
/// does not surface model identity (name / version) — the identity
/// recorded in `index.db.meta` as `summariser_name` / `summariser_version`
/// comes from the registry entry, not from the runtime trait
/// (mirrors the `Embedder`/`Reranker` precedent where production code
/// reads identity from `MODEL_REGISTRY`, not from the trait).
///
/// `long_max_chars` is the effective character cap for the long summary,
/// resolved from `config.summariser.long_max_chars.unwrap_or(LONG_MAX_CHARS)`
/// at the call site. Implementations use it to set the "Maximum N characters"
/// instruction in the long prompt and to apply the post-generation char cap.
pub trait Summariser: Send + Sync {
    fn summarise(
        &self,
        input: &PluginSummariesInput,
        long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError>;
}

/// The tighter per-call timeout used by the POST-COMMIT auto-regen trigger
/// (FR-027). The trigger runs after a `plugin enable`/`disable`/etc. has already
/// committed; a slow remote summariser must not make the command feel hung, so
/// the trigger caps the provider timeout lower than the foreground default
/// ([`crate::provider::config::DEFAULT_PROVIDER_TIMEOUT`], 30s) and degrades any
/// timeout to a non-fatal `warn!`.
pub const TRIGGER_TIMEOUT: Duration = Duration::from_secs(10);

/// Build the summariser the workspace should use: a [`RemoteSummariser`] when
/// `[summariser] provider` references a configured provider, else the bundled
/// [`LlamaSummariser`].
///
/// This is the single selection chokepoint both the foreground
/// (`tome workspace regen-summary`) and the post-commit trigger route through,
/// so "remote vs bundled" is decided in exactly one place.
///
/// - `tighter_timeout`: when `true` (the auto-trigger path) the resolved
///   provider's per-call timeout is lowered to [`TRIGGER_TIMEOUT`] so a slow
///   remote can't make `plugin enable` feel hung.
/// - On the remote path, the one-time first-run notice
///   ([`crate::provider::notice::notify_remote_use`]) fires so the user is told
///   that skill text is sent off-box.
///
/// NFR-006: the `None` (bundled) branch is byte-identical to the pre-Phase-12
/// behaviour — `LlamaSummariser::new(paths)`.
pub fn build_summariser(
    cfg: &Config,
    paths: &Paths,
    tighter_timeout: bool,
) -> Result<Box<dyn Summariser>, TomeError> {
    match crate::provider::resolve(cfg, Capability::Summariser)? {
        Some(mut resolved) => {
            if tighter_timeout {
                resolved.timeout = TRIGGER_TIMEOUT;
            }
            crate::provider::notice::notify_remote_use(paths, &resolved.name);
            Ok(Box::new(RemoteSummariser::new(resolved)))
        }
        None => Ok(Box::new(LlamaSummariser::new(paths)?)),
    }
}

/// Input to the summariser: every enabled plugin and its skill set
/// for a single workspace, in the order the workspace's catalog
/// registry returns them. Determinism matters — the same workspace
/// state must produce the same prompt, so tests can assert against
/// stable output (the stub) and production caching keys off the input
/// content hash (US4).
#[derive(Debug, Clone, Default)]
pub struct PluginSummariesInput {
    pub plugins: Vec<PluginSummaryItem>,
}

/// One enabled plugin in [`PluginSummariesInput`]. `description` is the
/// `plugin.json` description (lenient parse, may be empty); `skills`
/// is the plugin's enabled skill list, in catalog order.
#[derive(Debug, Clone)]
pub struct PluginSummaryItem {
    pub catalog: String,
    pub plugin: String,
    pub description: String,
    pub skills: Vec<SkillSummaryItem>,
}

/// One skill in a [`PluginSummaryItem`]. `name` is the SKILL.md
/// frontmatter `name`; `description` is the frontmatter `description`.
#[derive(Debug, Clone)]
pub struct SkillSummaryItem {
    pub name: String,
    pub description: String,
}

/// Output of a single summariser invocation. Both fields are required;
/// either being empty is a hard failure
/// (`SummariserFailureKind::OutputEmpty`, exit 24). The short summary
/// feeds the MCP `search_skills` tool description; the long summary
/// feeds the workspace's `RULES.md`.
#[derive(Debug, Clone)]
pub struct SummariserOutput {
    pub short: String,
    pub long: String,
}

// ---------------------------------------------------------------------------
// LlamaBackend singleton.
//
// `llama-cpp-2` requires a single process-wide `LlamaBackend`. We hold
// it in a `std::sync::OnceLock<Result<LlamaBackend, _>>`-style pattern,
// but since `OnceLock::get_or_try_init` is unstable on Rust 1.93, we
// wrap the fallible init in a `Mutex` and store the successfully
// initialised backend in a separate `OnceLock<LlamaBackend>`. The
// `Mutex` lives only for the duration of the (one-shot) init; once
// `BACKEND` is set, subsequent `backend()` calls hit the lock-free
// fast path on the `OnceLock`.
//
// Re-initialisation is not attempted: if the first `LlamaBackend::init()`
// fails, every subsequent `backend()` call returns the same
// `BackendInitFailed` error message (cached in `INIT_RESULT`). This
// matches FR-424's "fail loud, fail same" requirement — a half-broken
// backend doesn't get to lazily recover on the next call.
// ---------------------------------------------------------------------------

static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
static INIT_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Return the process-wide `LlamaBackend` singleton, initialising it
/// on first call. Subsequent calls hit the fast path. A first-call
/// failure is cached — every later `backend()` returns the same
/// `SummariserFailure { kind: BackendInitFailed { source } }`.
///
/// ## Concurrency / race semantics (T320, M8 fold-in)
///
/// - **Fast path**: once `BACKEND` is populated, every subsequent
///   `backend()` call is a single `OnceLock::get` — lock-free, no
///   serialisation, no allocation. This is the steady-state cost.
/// - **First call**: the slow path takes the `INIT_LOCK` mutex. If
///   two threads race here, exactly one of them wins the `init()`
///   call; the other blocks on the mutex, then re-checks `BACKEND`
///   on entry and returns the winner's result.
/// - **Init failure**: the first failure is cached into
///   `INIT_RESULT` and returned verbatim on every subsequent call.
///   We do *not* retry — a half-broken backend doesn't get to
///   lazily recover. This matches FR-424's "fail loud, fail same"
///   semantics.
/// - **`LlamaCppError::BackendAlreadyInitialized`**: signals a
///   single-process invariant violation (someone bypassed `backend()`
///   and called `LlamaBackend::init()` directly). The C++ side
///   permits at most one backend per process. Our `INIT_LOCK` +
///   `OnceLock` discipline makes this unreachable through the Tome
///   API, but if it ever surfaces it is cached just like any other
///   init failure — the resulting error message carries the upstream
///   `BackendAlreadyInitialized` text so the operator can correlate
///   it with the wider process state.
/// - **Mutex poisoning**: a panic *inside* the lock guard (e.g. a
///   panicking allocator during `LlamaBackend::init()`) poisons the
///   mutex. US4.d-1 (R-M7) the next `backend()` call recovers via
///   `PoisonError::into_inner` and proceeds with its own init attempt;
///   the poisoned mutex doesn't permanently disable the summariser.
///   `INIT_RESULT` still discriminates: if the panicking thread set
///   it before panicking, the cached failure wins; otherwise the new
///   caller gets a fresh attempt.
///
/// Note: this whole module sits on the sync side of
/// `tests/sync_boundary.rs`. The MCP server (async) reaches the
/// summariser only through CLI subprocesses, never in-process.
pub fn backend() -> Result<&'static LlamaBackend, TomeError> {
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    if let Some(Err(source)) = INIT_RESULT.get() {
        return Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: source.clone(),
            },
        });
    }

    // Slow path: take the init lock so concurrent first-callers
    // serialise on a single `LlamaBackend::init()` attempt.
    //
    // US4.d-1 (R-M7): on poisoning, recover via `into_inner()` instead
    // of bubbling. The init lock guards only the brief
    // `LlamaBackend::init()` call below; any panic inside that scope
    // is already a hard failure for the calling thread. The next
    // caller doesn't need to be poisoned by association — they get
    // their own attempt at init, and the cached `INIT_RESULT`
    // (populated after init returns) discriminates between "first
    // attempt failed cleanly" (cached error) and "first attempt
    // panicked" (no cache; this caller tries again). The lock
    // poisoning is purely about cross-thread panic propagation; the
    // backend state itself is owned by `OnceLock`s and is unaffected.
    let _guard = INIT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // Re-check after acquiring the lock — another thread may have
    // raced through init while we were blocked.
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    if let Some(Err(source)) = INIT_RESULT.get() {
        return Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::BackendInitFailed {
                source: source.clone(),
            },
        });
    }

    // Route llama.cpp/ggml C-side logging through `tracing` instead of raw
    // stderr (send_logs_to_tracing sets BOTH llama_log_set and ggml_log_set,
    // so the Metal/sched chatter is captured too). Every such log is emitted
    // under the `llama-cpp-2` tracing target, which the default CLI directive
    // (`logging::DEFAULT_CLI_DIRECTIVE`) demotes to `error` — so the info-level
    // load/buffer lines AND the benign `n_ctx` WARN (issue #501) disappear at
    // the default level while genuine errors still surface; `-vv` restores the
    // full output. Must run at most once per process — guaranteed by the
    // surrounding once-only init path.
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());

    match LlamaBackend::init() {
        Ok(backend) => {
            // `set` may fail if a racing init beat us; in that case
            // discard our backend and return the one that won.
            let _ = BACKEND.set(backend);
            let _ = INIT_RESULT.set(Ok(()));
            Ok(BACKEND
                .get()
                .expect("LlamaBackend was set above or by a racing init"))
        }
        Err(e) => {
            let source = e.to_string();
            // Cache the failure so subsequent calls return the same
            // message without re-attempting init.
            let _ = INIT_RESULT.set(Err(source.clone()));
            Err(TomeError::SummariserFailure {
                kind: SummariserFailureKind::BackendInitFailed { source },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_types_are_debug_clone() {
        // Smoke test for the trait bounds the data-model contract pins.
        let input = PluginSummariesInput::default();
        let cloned = input.clone();
        let _ = format!("{cloned:?}");
    }

    #[test]
    fn stub_summariser_matches_trait_object_shape() {
        // The stub must coerce to `Arc<dyn Summariser>` — that's how
        // US4 holds it for cross-trigger reuse.
        let s: std::sync::Arc<dyn Summariser> = std::sync::Arc::new(StubSummariser::new());
        let out = s
            .summarise(&PluginSummariesInput::default(), LONG_MAX_CHARS)
            .expect("stub never errors");
        assert!(out.short.is_empty());
        assert!(out.long.contains("This workspace covers"));
    }
}
