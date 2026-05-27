//! Phase 5 / US3.a + US3.b — Stage 3 (argument substitution) +
//! Stage 4 (`ARGUMENTS:` append-fallback footer) tests.
//!
//! Covers the four Stage-3 patterns (`$ARGUMENTS[N]`, bare
//! `$ARGUMENTS`, `$N`, `$<name>`), the 6-row caller-coercion table
//! from `contracts/substitution-engine.md` § Stage 3, the no-rescan
//! invariant (NFR-007) across the Stage 1+2 ↔ Stage 3 boundary, and
//! the Stage 4 trigger rules from § Stage 4 + research §R-13.
//!
//! Env-var serialisation: the three NFR-007 cross-stage tests mutate
//! the host environment via `std::env::set_var` to construct hostile
//! payloads. They serialise via a file-local `ENV_MUTEX` mirroring
//! the convention in `tests/substitution_env.rs` /
//! `tests/substitution_pipeline.rs`. `EnvVarGuard` is RAII — snapshot
//! on install, restore on drop — so panics inside a test don't leak
//! state to siblings. The mutex stays file-local rather than promoted
//! to `tests/common/mod.rs`: a future US4 / US5 NFR test surface is
//! the natural third consumer to drive promotion.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use common::{PluginDataDirGuard, WorkspaceDataDirGuard, lifecycle_paths};
use time::OffsetDateTime;
use tome::substitution::{
    self, ArgumentValues, SubstitutionContext, SubstitutionContextBuilder, SubstitutionError,
};

// --- Env serialisation discipline ----------------------------------------

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvVarGuard {
    key: String,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: caller holds ENV_MUTEX; no other test mutates env.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key: key.to_owned(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: ENV_MUTEX is still held by the test for the
        // lifetime of this guard.
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

// --- Context plumbing ----------------------------------------------------

fn ctx_builder(home: &std::path::Path) -> SubstitutionContextBuilder {
    let paths = lifecycle_paths(home);
    SubstitutionContext::builder()
        .catalog_name("test-catalog")
        .plugin_name("test-plugin")
        .plugin_version("1.2.3")
        .entry_name("hello")
        .entry_path(PathBuf::from("/plugins/x/skills/hello/SKILL.md"))
        .entry_dir(PathBuf::from("/plugins/x/skills/hello"))
        .plugin_root_dir(PathBuf::from("/plugins/x"))
        .workspace_name("global")
        .clock(OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
}

fn ctx_with_args(
    home: &std::path::Path,
    args: ArgumentValues,
    declared: Vec<String>,
) -> SubstitutionContext {
    ctx_builder(home)
        .args(Some(args))
        .declared_args(declared)
        .build()
        .expect("builder")
}

fn obj(pairs: &[(&str, &str)], declared: &[&str]) -> ArgumentValues {
    let mut named = HashMap::new();
    for (k, v) in pairs {
        named.insert((*k).to_string(), (*v).to_string());
    }
    ArgumentValues::Object {
        named,
        declared_order: declared.iter().map(|s| (*s).to_string()).collect(),
    }
}

// --- Stage 3: shell-split + caller-coercion table -----------------------

#[test]
fn single_string_with_declared_args_shell_splits_into_positional() {
    // Row 1 of the coercion table: Single + declared → shell-split,
    // bind positionally to declared names.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("foo bar baz".into()),
        vec!["a".into(), "b".into(), "c".into()],
    );
    let body = "$0 / $1 / $2 + $a $b $c";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "foo / bar / baz + foo bar baz");
}

#[test]
fn single_string_with_no_declared_args_is_whole_string() {
    // Row 2: Single + no declared → whole-string single positional.
    // `$ARGUMENTS = "foo bar baz"`, `$ARGUMENTS[0] = "foo bar baz"`.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("foo bar baz".into()),
        vec![],
    );
    let body = "all=$ARGUMENTS first=$ARGUMENTS[0]";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "all=foo bar baz first=foo bar baz");
}

#[test]
fn single_string_with_quoted_token_preserves_internal_whitespace() {
    // R-10 shell-split: single quotes preserve internal whitespace.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("a 'b c' d".into()),
        vec!["x".into(), "y".into(), "z".into()],
    );
    let body = "$x | $y | $z";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "a | b c | d");
}

#[test]
fn single_string_with_double_quote_token_preserves_whitespace() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single(r#"a "b c" d"#.into()),
        vec!["x".into(), "y".into(), "z".into()],
    );
    let body = "$x | $y | $z";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "a | b c | d");
}

#[test]
fn object_with_named_args_resolves_by_name_and_position() {
    // Row 3: Object + declared, fully populated.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "X"), ("b", "Y")], &["a", "b"]),
        vec!["a".into(), "b".into()],
    );
    let body = "named: $a $b | positional: $0 $1";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "named: X Y | positional: X Y");
}

#[test]
fn object_with_partial_named_args_fills_missing_with_empty() {
    // Row 4: Object + declared, partial — missing declared names bind
    // to empty string.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "X")], &["a", "b"]),
        vec!["a".into(), "b".into()],
    );
    let body = "a=$a b=$b pos0=$0 pos1=$1";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "a=X b= pos0=X pos1=");
}

#[test]
fn object_with_args_key_and_no_declared_is_catchall_single_string() {
    // Row 5: Object{args} + no declared → catch-all coercion to
    // Single. Verifies MCP-prompts FR-071 reaches the same coercion
    // path via the library API.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), obj(&[("args", "foo bar baz")], &[]), vec![]);
    let body = "got: $ARGUMENTS";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "got: foo bar baz");
}

#[test]
fn object_with_unknown_named_returns_argument_mismatch() {
    // Row 6: Object{unknown} + declared → PromptArgumentMismatch.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("unknown", "X")], &["a"]),
        vec!["a".into()],
    );
    let err = substitution::render("body", &ctx).expect_err("must reject");
    match err {
        SubstitutionError::PromptArgumentMismatch { expected, supplied } => {
            assert_eq!(expected, 1);
            assert_eq!(supplied, 1);
        }
        other => panic!("expected PromptArgumentMismatch, got {other:?}"),
    }
}

#[test]
fn object_with_non_args_key_and_no_declared_returns_mismatch() {
    // Row 7 (a tightening of row 6 for catch-all): Object with a non-
    // `args` key when declared is empty → mismatch.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), obj(&[("other", "X")], &[]), vec![]);
    let err = substitution::render("body", &ctx).expect_err("must reject");
    matches!(err, SubstitutionError::PromptArgumentMismatch { .. });
}

// --- Stage 3: per-pattern resolution -------------------------------------

#[test]
fn dollar_arguments_index_resolves_positional() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("a b c".into()),
        vec!["x".into(), "y".into(), "z".into()],
    );
    let body = "first=$ARGUMENTS[0] middle=$ARGUMENTS[1] last=$ARGUMENTS[2]";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "first=a middle=b last=c");
}

#[test]
fn dollar_arguments_index_out_of_range_resolves_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("only-one".into()),
        vec![],
    );
    // declared empty → positional = ["only-one"]; $ARGUMENTS[5] out of
    // range → empty string (NOT verbatim) per FR-040.
    let body = "out=$ARGUMENTS[5]!";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "out=!");
}

#[test]
fn dollar_name_resolves_named() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("name", "Alice")], &["name"]),
        vec!["name".into()],
    );
    let body = "hello $name!";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "hello Alice!");
}

#[test]
fn dollar_name_not_provided_resolves_empty() {
    // Declared but not provided (Object with partial named) ⇒ empty.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), obj(&[], &["name"]), vec!["name".into()]);
    let body = "hello $name!";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "hello !");
}

#[test]
fn bare_arguments_joins_positional_values_with_single_space() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "X"), ("b", "Y"), ("c", "Z")], &["a", "b", "c"]),
        vec!["a".into(), "b".into(), "c".into()],
    );
    let body = "all=$ARGUMENTS";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "all=X Y Z");
}

#[test]
fn arguments_index_wins_over_bare_arguments() {
    // Regression guard for the leftmost-first ordering: `$ARGUMENTS[2]`
    // must NOT be parsed as bare-`$ARGUMENTS` followed by literal `[2]`.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("a b c".into()),
        vec!["x".into(), "y".into(), "z".into()],
    );
    let body = "$ARGUMENTS[2]";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "c");
}

// --- Stage 3: no-args path leaves references verbatim --------------------

#[test]
fn dollar_n_with_no_args_left_verbatim() {
    // Skeleton-era behaviour for the no-args path: when the caller
    // supplies no arguments at all, Stage 3 is structurally skipped
    // (research §R-10 last row) and references pass through verbatim.
    // (FR-040's "empty if not provided" describes the WITHIN-Stage-3
    // empty-string behaviour for missing declared names; it doesn't
    // apply when Stage 3 is skipped entirely.)
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_builder(tmp.path()).build().unwrap();
    let body = "$ARGUMENTS / $1 / $foo / $ARGUMENTS[5]";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, body);
}

// --- NFR-007 no-rescan invariant: Stage 1+2 ↔ Stage 3 ------------------

#[test]
fn stage_1_substituted_value_containing_arguments_pattern_is_not_rescanned_by_stage_3() {
    // Hostile plugin sets plugin_version to `$0`. A body referencing
    // ${TOME_PLUGIN_VERSION} would, under a double-pass design, see
    // the resolved `$0` rescanned by Stage 3 and substituted with the
    // first positional value. Single-sweep design (US3.a regex
    // extension) prevents this — Stage 1's output never re-enters the
    // scanner.
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_builder(tmp.path())
        .plugin_version("$0") // hostile
        .args(Some(ArgumentValues::Single("LEAKED".into())))
        .declared_args(vec![])
        .build()
        .unwrap();
    let body = "ver=${TOME_PLUGIN_VERSION}";
    let out = substitution::render(body, &ctx).unwrap();
    // Header line: Stage 1's `$0` output is preserved verbatim (NOT
    // substituted by Stage 3). The body has no Stage-3 references, so
    // Stage 4 fallback footer fires — verified separately via
    // `append_fallback_does_not_trigger_when_only_stage_1_substituted`.
    assert!(
        out.starts_with("ver=$0\n"),
        "Stage 1 output `$0` must NOT be rescanned by Stage 3; got: {out:?}",
    );
    assert!(
        !out.contains("ver=LEAKED"),
        "Stage 3 must NOT have substituted into Stage 1's output; got: {out:?}",
    );
}

#[test]
fn stage_3_arg_value_containing_tome_syntax_is_not_rescanned_by_stage_1_2() {
    // Caller passes `arg = "${TOME_ENV_SECRET}"`. A double-pass
    // design (Stage 3 first, then Stage 1+2) would have Stage 1+2
    // pick up the substituted text and resolve it against the
    // operator's env var. Single-sweep design prevents this — Stage
    // 3's output never re-enters the scanner.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_SECRET", "exfiltrated");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("arg", "${TOME_ENV_SECRET}")], &["arg"]),
        vec!["arg".into()],
    );
    let body = "value=$arg";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(
        out, "value=${TOME_ENV_SECRET}",
        "Stage 3 output `${{TOME_ENV_SECRET}}` must NOT be rescanned by Stage 1+2",
    );
}

#[test]
fn stage_3_arg_value_containing_dollar_name_is_not_rescanned_by_stage_3() {
    // Caller passes `a = "$b"`, `b = "leaked"`. A double-pass design
    // would substitute `$a` → `$b`, then re-scan and substitute `$b`
    // → `leaked`. Single-sweep design prevents this — each match is
    // resolved against the ORIGINAL body and emitted verbatim.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "$b"), ("b", "leaked")], &["a", "b"]),
        vec!["a".into(), "b".into()],
    );
    let body = "$a $b";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(
        out, "$b leaked",
        "Stage 3 output `$b` from $a must NOT be rescanned by Stage 3",
    );
}

#[test]
fn stage_2_substituted_value_containing_dollar_pattern_is_not_rescanned_by_stage_3() {
    // US3.d T-M1: pin the Stage 2 ↔ Stage 3 boundary.
    // A double-pass design (Stage 1+2 first, then Stage 3) where Stage 2's
    // output is fed into Stage 3's regex would have a TOME_ENV_* value
    // containing `$0` text get substituted. Single-sweep architecture
    // makes this structurally impossible; this test pins the invariant.
    let _lock = lock_env();
    let _guard = EnvVarGuard::set("TOME_ENV_INJECT", "$0");
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_with_args(
        tmp.path(),
        ArgumentValues::Single("first-positional".to_string()),
        vec![],
    );
    let body = "env=${TOME_ENV_INJECT}";
    let out = substitution::render(body, &ctx).unwrap();
    // The injected $0 from Stage 2 must appear LITERALLY in the body
    // section — if Stage 3 had re-scanned it, we'd see "first-positional"
    // there instead. Stage 4 append-fallback fires (body has no Stage 3
    // references) — the footer is expected and unrelated to the invariant.
    assert_eq!(
        out, "env=$0\n\nARGUMENTS: first-positional",
        "Stage 2 output `$0` from TOME_ENV_INJECT must NOT be rescanned by Stage 3",
    );
}

// --- Stage 4: append-fallback footer --------------------------------------

#[test]
fn append_fallback_triggers_when_no_body_references() {
    // Body has zero Stage-3 references AND caller supplied args →
    // footer is appended per FR-044.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), ArgumentValues::Single("world".into()), vec![]);
    let body = "Hello";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Hello\n\nARGUMENTS: world");
}

#[test]
fn append_fallback_does_not_trigger_when_any_reference_matched() {
    // Even a single matched Stage-3 reference suppresses the fallback
    // — research §R-13 ("argument-substitution stage records whether
    // it performed any replacements").
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), ArgumentValues::Single("world".into()), vec![]);
    let body = "Hello $0";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Hello world", "fallback must NOT be appended");
    assert!(!out.contains("ARGUMENTS:"));
}

#[test]
fn append_fallback_format_matches_contract_single() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), ArgumentValues::Single("world".into()), vec![]);
    let body = "Hello";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Hello\n\nARGUMENTS: world");
}

#[test]
fn append_fallback_body_ends_with_newline_single_separator() {
    // Body already ends with `\n` → only ONE additional `\n` is
    // needed for the blank line before `ARGUMENTS:`. Total: blank
    // line + footer.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), ArgumentValues::Single("world".into()), vec![]);
    let body = "Hello\n";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Hello\n\nARGUMENTS: world");
}

#[test]
fn append_fallback_object_args_joins_positional() {
    // Object args coercion: positional values joined by single space.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "X"), ("b", "Y")], &["a", "b"]),
        vec!["a".into(), "b".into()],
    );
    let body = "Run a deploy.";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Run a deploy.\n\nARGUMENTS: X Y");
}

#[test]
fn append_fallback_partial_object_includes_empty_for_missing_declared() {
    // Object with partial named: missing declared bind to empty
    // string; the footer reflects that as a space-only gap.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(
        tmp.path(),
        obj(&[("a", "X")], &["a", "b"]),
        vec!["a".into(), "b".into()],
    );
    let body = "Body";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "Body\n\nARGUMENTS: X ");
}

#[test]
fn append_fallback_does_not_trigger_with_no_caller_args() {
    // No caller args → no Stage 3 → no Stage 4. Body unchanged.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_builder(tmp.path()).build().unwrap();
    let body = "Just a static body.";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, body);
}

#[test]
fn append_fallback_does_not_trigger_when_only_stage_1_substituted() {
    // Body has a Stage-1 built-in (which substitutes) but no Stage-3
    // reference. The Stage-1 sub does NOT count as a Stage-3
    // replacement — fallback SHOULD trigger.
    let tmp = tempfile::tempdir().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));

    let ctx = ctx_with_args(tmp.path(), ArgumentValues::Single("world".into()), vec![]);
    let body = "name=${TOME_PLUGIN_NAME}";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "name=test-plugin\n\nARGUMENTS: world");
}

#[test]
fn append_fallback_bare_arguments_counts_as_replacement_even_when_positional_empty() {
    // Object with declared but no values → positional all empty.
    // Bare `$ARGUMENTS` still resolves to empty string, which counts
    // as a Stage-3 replacement — fallback MUST NOT trigger.
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_with_args(tmp.path(), obj(&[], &["a"]), vec!["a".into()]);
    let body = "got=$ARGUMENTS";
    let out = substitution::render(body, &ctx).unwrap();
    assert_eq!(out, "got=");
    assert!(!out.contains("ARGUMENTS:"));
}
