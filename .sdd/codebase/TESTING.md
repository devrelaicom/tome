# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-29 (Phase 6 US5 — privilege governance + doctor extensions)

## Test Framework

| Type | Framework | Configuration | Commands |
|------|-----------|---------------|----------|
| Unit | Rust `#[test]` | None (built-in) | `cargo test --lib` |
| Integration | Rust `#[test]` in `tests/` | None (built-in) | `cargo test --test '*'` |
| All | Combined | `.cargo/config.toml` | `cargo test` |

### Running Tests

| Command | Purpose |
|---------|---------|
| `cargo test` | Run all unit + integration tests (uses stub embedder — fast) |
| `cargo test --test catalog_add` | Run one integration test file |
| `cargo test catalog_add::` | Run one test by path |
| `cargo test --test query` | Phase 2 query tests |
| `cargo test --test concurrency` | Two-process index contention |
| `cargo test --test atomicity` | Interrupt-injection tests |

**MSRV tested**: CI runs `cargo +1.93 build` to enforce `rust-version = "1.93"`.

**Pre-push hook** (Phase 5 Polish change): local `cargo fmt`, `cargo clippy`, `typos` checks only. Full `cargo test --workspace` runs in CI as the source of truth (deferred from local pre-push per Phase 5 Polish PR #126 to keep pre-push under ~1 minute on warm cache). Test discipline NOT relaxed — CI matrix is the enforcement surface.

## Test Organization

### Directory Structure

```
tests/
├── *.rs                         # Integration test files (175+ total as of Phase 6 US5)
├── common/
│   ├── mod.rs                   # Shared harness: ToolEnv, Fixture, guards
│   └── ...                      # (exported helpers)
└── fixtures/
    ├── sample-catalog/          # Catalog skeleton (git repo template)
    └── sample-plugin-catalog/   # Plugin skeleton (for lifecycle tests)
```

### Test File Location Strategy

**Separate directory** (`tests/` parallel to `src/`): all integration tests. No co-located unit tests (Rust convention discouraged here because the test binary needs to invoke the CLI and construct real environments).

**Unit tests** within `src/` modules: for pure functions that don't need I/O isolation. Example: `src/config.rs::tests` tests TOML round-tripping.

### Test Categories by File

| Category | Files | Example |
|----------|-------|---------|
| **Catalog commands** | `catalog_*.rs` | `catalog_add.rs`, `catalog_remove.rs` (12 files) |
| **Plugin commands** | `plugin_*.rs` | `plugin_enable.rs`, `plugin_disable.rs` (10 files) |
| **Query & search** | `query.rs`, `entry_*.rs` | Embedding + reranking tests (5 files) |
| **Models & embedding** | `models_*.rs`, `embedding_*.rs` | Download, list, remove (6 files) |
| **Workspace lifecycle** | `workspace_*.rs` | Init, rename, remove, sync (12 files) |
| **Harness integration** | `harness_*.rs` | Use, list, remove, sync (12 files) |
| **Index & schema** | `index_*.rs`, `schema_migration_*.rs` | Database, migrations (7 files) |
| **Doctor & diagnostics** | `doctor_*.rs` | Report, fixes, orphan cleanup (7 files) |
| **MCP server** | `mcp_*.rs` | Server lifecycle, tools, log format (10 files) |
| **Concurrency & atomicity** | `concurrency.rs`, `atomicity.rs` | Lock contention, interrupts (4 files) |
| **Frontmatter & manifests** | `frontmatter*.rs`, `manifest_*.rs` | YAML parsing, strictness (4 files) |
| **Security & hardening** | `security_hardening.rs` | File perms, symlink refusal (1 file) |
| **Error & exit codes** | `exit_codes*.rs`, `error_messages.rs` | Exit code coverage, Display impl (2 files) |
| **Substitution** (Phase 5) | `substitution_*.rs`, `entry_*.rs` | Variable expansion, argument coercion (8 files) |
| **Agent translation** (Phase 6 US1) | `agent_translate_*.rs`, `agent_*.rs` | Per-harness native agents (8 files) |
| **Hooks integration** (Phase 6 US2) | `hooks_rewrite.rs`, `hooks_merge.rs` | Path-variable rewriting, config merging (2 files) |
| **Guardrails & rules-file** (Phase 6 US3) | `guardrails_*.rs`, `rules_file_*.rs` | Guardrails regions, rules-file placement (6 files) |
| **Agent personas** (Phase 6 US4) | `personas.rs`, `personas_collision.rs`, `personas_startup_scope.rs` | Persona prompts, toggle, startup resolution (3 files) |
| **Settings** (Phase 6) | `settings_p6.rs`, `settings_*.rs` | Scalar resolution, layering, first-declarer-wins (15+ files) |
| **Doctor extensions** (Phase 6 US5) | `doctor_p6.rs`, `doctor_p6_json_shape.rs`, `doctor_json.rs` | Hooks/guardrails/agents/personas/privilege reports (3 files) |
| **Misc** | `path_validation.rs`, `atomic_dir.rs`, etc. | Phase 1 foundational (10 files) |

**Total**: 175+ test files across 175+ suites; 1250+ tests pass (Phase 6 US5 adds 3 new files + extensions).

**Phase 6 US5 additions**:
- `tests/doctor_p6.rs` — Full matrix of Phase 6 doctor surfaces: hooks report (contributed/missing per event/plugin), guardrails report (present/orphaned/suppressed), agents report (per-harness presence), personas report (toggle on/off), privilege-escalation report (grouped by plugin + field); `--fix` re-renders each surface idempotently; read-only creates no directories (exact-count proof per FR-124)
- `tests/doctor_p6_json_shape.rs` — Byte-stable JSON pin for doctor `HooksReport`, `GuardrailsReport`, `AgentsReport`, `PersonasReport`, `PrivilegeEscalationReport` emitted in `DoctorOutput` (wire-shape change contract: appended LAST, `skip_serializing_if` on optional fields)
- `tests/plugin_show_p6.rs` — Extended `plugin show` for agents: lists agent rows per plugin, shows `hooks.json`/`GUARDRAILS.md` presence, displays resolved persona name (clash-prefixed if applicable)
- `tests/plugin_show_p6_json_shape.rs` — Byte-stable JSON pin for agent entries in `plugin show` output (agent list includes name + display name + presence flags)
- `tests/agent_privilege.rs` — Privilege escalation audit unchanged when strip setting on; stripping only affects emission, not source; privilege report sees unstripped source
- Extensions to `tests/doctor_*.rs` — Confirm doctor surfaces created atomically, re-readable on re-run, hooks/guardrails/agents/personas all render when conditions met
- Extensions to `tests/exit_codes_e2e.rs` — Exit 45 (AgentTranslationFailed) via `workspace use` with agent `name: ../../../../tmp/evil`; exit 46 (GuardrailsWriteFailed) via symlinked guardrails target during sync
- Extensions to `tests/settings_*.rs` — `strip_plugin_agent_privileges` setting; first-declarer-wins resolution; default `false` when absent

## Test Patterns

### Test Structure: Arrange-Act-Assert

```rust
#[test]
fn happy_path_human_mode() {
    // Arrange: set up fixture and environment
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    // Act: invoke the command
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .expect("spawn");

    // Assert: verify exit code, stdout, state
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Added catalog"));
}
```

### Fixture Pattern: Git-backed Catalog

```rust
pub struct Fixture {
    pub tempdir: TempDir,
    pub repo_path: PathBuf,
    pub url: String,  // file:// URL for cloning
}

impl Fixture {
    pub fn build_sample() -> Self {
        // Copy tests/fixtures/sample-catalog/ into temp dir
        // Run git init && git commit
        // Return handle to the temp repo
    }
}
```

**Used by**: all catalog tests, plugin lifecycle tests, reindex tests.

### Test Environment: ToolEnv

```rust
pub struct ToolEnv {
    pub home: TempDir,
}

impl ToolEnv {
    pub fn new() -> Self {
        // Create isolated $HOME with fresh XDG layout
    }

    pub fn cmd(&self) -> Command {
        // Return a Command for the `tome` binary
        // Pre-populate HOME + XDG_* env vars
        // Suppress logging output
    }
}
```

**Key discipline**: Every test gets its own `ToolEnv`. The host's real `~/.tome/` is never touched because `HOME` is redirected to a `TempDir`.

### Library API Pattern: No CLI Binary

When a test needs to verify library logic without loading real ONNX models, use the library API directly:

```rust
#[test]
fn enable_sets_enabled_flag() {
    let root = TempDir::new().unwrap();
    let paths = lifecycle_paths(root.path());
    let catalog = copy_sample_plugin_catalog(&root, "sample");
    fabricate_models(&paths);

    let embedder = StubEmbedder::new();
    let _guard = EmbedderGuard::install(Arc::new(embedder));

    let id = PluginId::from_str("sample/hello").unwrap();
    let deps = LifecycleDeps { ... };
    let outcome = lifecycle::enable(&id, &deps, false).unwrap();

    assert_eq!(outcome.status, PluginStatus::Enabled);
}
```

**Used by**: plugin lifecycle, reindex, workspace tests (avoid CLI spawn when library API suffices).

### CLI Binary Pattern: Full Integration

When testing the CLI's complete stack (command parsing, output formatting, exit codes), spawn the binary:

```rust
#[test]
fn catalog_add_emits_json_on_flag() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url, "--json"])
        .output()
        .expect("spawn");

    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).expect("json parse");
    assert_eq!(v["added"]["name"], "sample-experts");
}
```

**Used by**: output formatting tests, exit code coverage, TTY-dependent features like prompts.

### Isolation: `HomeGuard` for `$HOME` Mutations

```rust
#[test]
fn test_reads_home_var() {
    let new_home = TempDir::new().unwrap();
    let _guard = HomeGuard::install(new_home.path());

    // Inside this scope, $HOME is redirected
    assert_eq!(std::env::var("HOME").unwrap(), new_home.path().to_str().unwrap());

    // Test code runs here
}
// _guard drops, HOME is restored, mutex is released
```

**Discipline**: `HomeGuard` holds `HOME_MUTEX` for its lifetime, serializing all tests that mutate `$HOME`. This prevents parallel-test races.

### Phase 5: Test Injection for Time-Dependent Features

When tests verify time-based behavior (e.g., substitution with `$now` variable), use a clock injection guard:

```rust
#[test]
fn substitution_now_returns_fixed_time() {
    let now = time::OffsetDateTime::from_unix_timestamp(1609459200).unwrap();
    let _guard = ClockOverrideGuard::install(now);

    let result = substitution::substitute("built at $now", &ctx);
    assert_eq!(result, "built at 2021-01-01T00:00:00Z");
}
```

**Pattern**: `ClockOverrideGuard` (in `tests/common/mod.rs`) injects via `SUBSTITUTION_CLOCK_OVERRIDE` slot. Drop guard restores real clock. Used in `tests/substitution_*.rs`.

### Phase 5: Test Injection for Data Directory Features

When tests verify plugin or workspace data directory isolation, use data-dir injection guards:

```rust
#[test]
fn plugin_data_dir_isolates_per_plugin() {
    let plugin_root = TempDir::new().unwrap();
    let _guard = PluginDataDirGuard::install(plugin_root.path());

    // Tests now write plugin data to isolated dir, not user's home
    let config = load_plugin_config("my-plugin");
    assert_eq!(config.data_root, plugin_root.path());
}
```

**Patterns**: `PluginDataDirGuard`, `WorkspaceDataDirGuard` (Phase 5 US2 data-model changes).

### Phase 6: Test-Configurable Test Double (StubHarness Builder)

When a test double needs to drive different capability combinations, use the builder pattern with `Default`:

```rust
#[test]
fn harness_with_native_agents_registers_directory() {
    let harness = StubHarness::default()
        .with_native_agents(AgentFormat::MarkdownYaml);
    
    // harness::supports_native_agents() returns true
    // harness::agent_dir() returns Some(<project>/.stub/agents)
    assert!(harness.supports_native_agents());
}

#[test]
fn harness_with_hook_settings_returns_path() {
    let harness = StubHarness::default().with_hook_settings();
    
    // harness::hook_settings_path() returns Some(<project>/.stub/settings.local.json)
    assert!(harness.hook_settings_path(Path::new("/project")).is_some());
}
```

**Pattern** (Phase 6 Foundational F3): `StubHarness` evolved from a unit struct to a `#[derive(Default)]` struct. The `Default` impl produces safe defaults (trait safe defaults for all methods). Builder setters (`with_*`) flip capabilities without spelling out the whole struct. Used in `tests/harness_trait_p6.rs` to exercise hook + agent dispatch paths.

### Phase 6: Direct Per-Harness `translate_agent` Unit Tests

When testing a harness's agent translation without the full CLI/sync stack, call the harness's `translate_agent` method directly:

```rust
// tests/agent_translate_codex.rs
#[test]
fn body_lands_in_triple_quoted_developer_instructions() {
    let agent = read_only_agent();
    let t = CODEX.translate_agent(&agent, false).expect("translate");

    // Parse the rendered TOML and read the value back
    let doc: toml_edit::DocumentMut = t.rendered.parse().expect("parse");
    assert_eq!(
        doc["developer_instructions"].as_str(),
        Some(agent.body.as_str()),
        "developer_instructions holds the body verbatim",
    );
}
```

**Pattern** (Phase 6 US1): Harness modules implement `HarnessModule::translate_agent`, which takes a `CanonicalAgent` and a clash-set boolean. Direct calls avoid spinning up CLI, project markers, sync orchestration — tests remain fast and narrowly focused. Supports quick iteration on format/field-mapping details. Used for per-harness contract coverage.

### Phase 6: Full-Stack Agent Sync Tests via `sync_project` + Override

When testing the complete agent pipeline (enable → index → sync), use the library API with harness override:

```rust
// tests/agent_naming_clash.rs
#[test]
fn clash_applies_plugin_prefix_to_display_name() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(ClaudeCode)]);
    let fx = Fixture::build("test-workspace");

    // Insert two agents with the same `name` from different plugins
    insert_enabled_agent_row(&paths, "test-workspace", "cat", "pluginA", "reviewer", ...);
    insert_enabled_agent_row(&paths, "test-workspace", "cat", "pluginB", "reviewer", ...);

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");
    
    // Both files exist on disk with plugin-prefixed display names
    let a_rules = std::fs::read_to_string(&fx.project.join("CLAUDE_CODE_RULES.md"))?;
    assert!(a_rules.contains("name: pluginA-reviewer"));
    assert!(a_rules.contains("name: pluginB-reviewer"));
}
```

**Pattern** (Phase 6 US1): Tests that verify end-to-end agent sync behavior must install a real harness module (not `StubHarness`, which lacks translation semantics), seed agent rows in the index, and call `sync_project`. The `OVERRIDE_MUTEX` serializes concurrent override access. Used for integration-layer tests like clash handling, orphan cleanup, removal.

### Phase 6: Byte-Stable JSON Pins for Agent Dropped-Fields

When testing agent translation, verify the `dropped_fields` vector is recorded correctly and byte-stable:

```rust
// tests/agent_translate_codex.rs
#[test]
fn model_is_dropped_and_recorded() {
    let t = CODEX
        .translate_agent(&read_only_agent(), false)
        .expect("translate");

    // Recorded for the doctor surface
    assert!(
        t.dropped_fields.contains(&"model".to_owned()),
        "dropped model must be recorded; got {:?}",
        t.dropped_fields,
    );
}
```

**Pattern** (Phase 6 US1 / T053 placeholder): Every translation result carries a `dropped_fields: Vec<String>` describing which frontmatter keys were dropped during the field-map. Byte-stable JSON pin tests verify field order and presence for `TranslatedAgent` serialization (when agents are stored in doctor diagnostics).

### Phase 6: Direct Guardrails Region Tests via Library API

When testing guardrails rendering and marker validation without the full CLI sync, call guardrails APIs directly with in-memory file targets:

```rust
// tests/guardrails_render.rs
#[test]
fn regions_rendered_in_lexicographic_order() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("CLAUDE.md");

    // Seed initial content
    std::fs::write(&target, "# my rules\n").unwrap();

    // Reconcile with regions in non-lex order (z before a)
    let mut desired = BTreeMap::new();
    desired.insert("cat:z-plugin".to_string(), "z rules\n".to_string());
    desired.insert("cat:a-plugin".to_string(), "a rules\n".to_string());

    guardrails::reconcile_in_file_region(&target, &desired).expect("reconcile");

    let result = std::fs::read_to_string(&target).unwrap();
    // Verify a-plugin region comes before z-plugin
    let a_pos = result.find("cat:a-plugin").expect("a present");
    let z_pos = result.find("cat:z-plugin").expect("z present");
    assert!(a_pos < z_pos, "regions rendered in lex order");
}
```

**Pattern** (Phase 6 US3 / T3-1): Library-API tests for `guardrails::reconcile_in_file_region` and `rules_file::compose_in_file`. Direct calls avoid spinning up CLI + harness modules — tests focus on reconciliation logic, idempotence, marker validation. Used for verifying region ordering, overwrite-in-place, new-append, orphan-removal, and idempotence. Covers `contracts/guardrails.md` (FR-011/014/015).

### Phase 6: Rules-File Strategy Tests via Direct Library Calls

When testing rules-file block or standalone strategies, call the appropriate writer directly:

```rust
// tests/rules_file_block_in_existing.rs
#[test]
fn block_overwrites_in_place_preserves_surrounding_content() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("RULES.md");

    // Seed existing file with content outside the block
    let initial = "# Header\n\n<!-- tome:begin -->\n<!-- tome:end -->\n\nFooter\n";
    std::fs::write(&target, initial).unwrap();

    // Update the block content
    rules_file::write_block_in_file(&target, "new body\n").unwrap();

    let result = std::fs::read_to_string(&target).unwrap();
    assert!(result.contains("# Header"), "header preserved");
    assert!(result.contains("new body"), "body updated");
    assert!(result.contains("Footer"), "footer preserved");
    // Verify markers still present with newline discipline
    assert!(result.contains("<!-- tome:begin -->"), "begin marker present");
    assert!(result.contains("<!-- tome:end -->"), "end marker present");
}
```

**Pattern** (Phase 6 US3 / T081): Library-API tests for `rules_file::{write_block_in_file, write_standalone_file}`. Tests verify marker preservation, idempotence short-circuit (no rewrite if bytes match), symlink refusal (exit 7), atomic write semantics. Used for verifying `BlockInExistingFile` and `StandaloneFile` strategies per `contracts/rules-file-integration.md` (FR-525).

### Phase 6 US4: Agent Personas via MCP Prompts

When testing agent personas without full CLI integration, build the prompt registry directly with `expose_personas` enabled:

```rust
// tests/personas.rs
#[test]
fn persona_toggle_off_excludes_personas() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    
    // Enable plugins with agents, expose_personas = false (default)
    lifecycle::enable(&plugin_id, &deps, false).unwrap();
    
    // Open index and build registry with personas OFF
    let conn = index::open(&paths, ...);
    let registry = PromptRegistry::build_for_workspace(&conn, &paths, false).unwrap();
    
    // No persona prompts in the list
    assert!(!registry.prompts.iter().any(|p| p.name.ends_with("-persona")));
}

#[test]
fn persona_toggle_on_includes_personas() {
    let fix = Fixture::build_sample();
    
    // Enable plugins with agents, expose_personas = true
    lifecycle::enable(&plugin_id, &deps, false).unwrap();
    
    // Open index and build registry with personas ON
    let conn = index::open(&paths, ...);
    let registry = PromptRegistry::build_for_workspace(&conn, &paths, true).unwrap();
    
    // Agent personas + drop-persona in the list
    assert!(registry.prompts.iter().any(|p| p.name == "reviewer-persona"));
    assert!(registry.prompts.iter().any(|p| p.name == "drop-persona"));
}

#[test]
fn persona_body_is_template_wrapped_with_substitution() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    
    lifecycle::enable(&plugin_id, &deps, false).unwrap();
    let conn = index::open(&paths, ...);
    let registry = PromptRegistry::build_for_workspace(&conn, &paths, true).unwrap();
    
    // Get persona prompt response
    let resp = registry.get_prompt("reviewer-persona", &[], &conn, ...).unwrap();
    
    // Body is wrapped in role-assumption template
    assert!(resp.messages[0].content[0].text.contains("Assume the `reviewer` agent persona"));
    // Frontmatter is stripped (no YAML delimiters)
    assert!(!resp.messages[0].content[0].text.contains("---"));
    // Phase 5 substitution applied (e.g., $now → timestamp)
    assert!(resp.messages[0].content[0].text.contains("2026-05-29"));
}

#[test]
fn persona_prompts_join_collision_namespace() {
    let fix = Fixture::build_sample_with_clash();  // Two plugins, agent name clash
    let env = ToolEnv::new();
    
    lifecycle::enable(&plugin_a_id, &deps, false).unwrap();
    lifecycle::enable(&plugin_b_id, &deps, false).unwrap();
    
    let conn = index::open(&paths, ...);
    let registry = PromptRegistry::build_for_workspace(&conn, &paths, true).unwrap();
    
    // Clashing personas are plugin-prefixed
    assert!(registry.prompts.iter().any(|p| p.name == "pluginA-reviewer-persona"));
    assert!(registry.prompts.iter().any(|p| p.name == "pluginB-reviewer-persona"));
    // Non-clashing persona keeps clean name
    assert!(registry.prompts.iter().any(|p| p.name == "other-persona"));
    // drop-persona is reserved and unique (once, unnamespaced)
    assert_eq!(
        registry.prompts.iter().filter(|p| p.name == "drop-persona").count(),
        1,
        "drop-persona appears exactly once"
    );
}
```

**Pattern** (Phase 6 US4): Library-API tests for persona prompts. Tests verify toggle on/off, template-wrapping + substitution + arguments, frontmatter stripping, collision clash-prefix, drop-persona uniqueness + reservation. Registry is built in-process with fixture agents; the persona path is tested separately from the command/skill path (no folding). Covers `contracts/agent-personas.md` (FR-060–FR-065).

### Phase 6 US4: Startup Scope Resolution for Settings

When testing settings resolution during MCP startup, call the scope-loaders directly and apply the resolver:

```rust
// tests/personas_startup_scope.rs
#[test]
fn expose_personas_resolved_from_project_workspace_global() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::new(&dir.path());
    
    // Seed on-disk settings: global = false, workspace = true, project = <absent>
    write_global_settings(&paths, Some(false)).unwrap();
    write_workspace_settings(&paths, "test-ws", Some(true)).unwrap();
    
    // Load and resolve (project absent falls through to workspace)
    let project = scopes::load_project_marker(None).unwrap();
    let workspace = scopes::load_workspace_settings(&paths, &WorkspaceName::parse("test-ws")?).unwrap();
    let global = scopes::load_global_settings(&paths).unwrap();
    
    let resolved = resolve_scalar_with(
        project.as_ref(),
        workspace.as_ref(),
        &global,
        |p| p.expose_agents_as_personas,
        |w| w.expose_agents_as_personas,
        |g| g.expose_agents_as_personas,
    );
    
    assert!(resolved, "workspace true overrides global false when project absent");
}

#[test]
fn expose_personas_project_overrides_global() {
    // Project = false, global = true → resolved = false
    let resolved = resolve_scalar_with(..., Some(false), Some(true), Some(true));
    assert!(!resolved, "project false overrides global true");
}
```

**Pattern** (Phase 6 US4 / R-4-2): Direct tests of `settings::scopes` loaders + `resolve_scalar_with` resolver. Tests verify first-declarer-wins walk, fall-through to next scope, default `false` when all absent. Scope-loader errors (parse failure → `WorkspaceMalformed`, NotFound → `Ok(None)` or default) tested via boundary conditions. Covers `contracts/settings-p6.md` (FR-053, FR-067).

### Phase 6 US4: Settings Struct Parse + Strictness

When testing Phase 6 scalar settings fields on the three Tome-owned settings structs, verify parse behavior + strictness:

```rust
// tests/settings_p6.rs
#[test]
fn expose_agents_as_personas_parses_and_defaults() {
    let toml_str = r#"
        name = "test-workspace"
        expose_agents_as_personas = true
    "#;
    let ws: WorkspaceSettings = toml::from_str(toml_str).unwrap();
    assert_eq!(ws.expose_agents_as_personas, Some(true));
}

#[test]
fn expose_agents_as_personas_absent_parses_as_none() {
    let toml_str = r#"
        name = "test-workspace"
    "#;
    let ws: WorkspaceSettings = toml::from_str(toml_str).unwrap();
    assert_eq!(ws.expose_agents_as_personas, None);
}

#[test]
fn unknown_key_rejected_deny_unknown_fields() {
    // Phase 6 adds `expose_agents_as_personas` but does NOT loosen strictness
    let toml_str = r#"
        name = "test-workspace"
        expose_agents_as_personas = true
        unknown_future_key = "should fail"
    "#;
    let result: Result<WorkspaceSettings, _> = toml::from_str(toml_str);
    assert!(result.is_err(), "unknown keys rejected (deny_unknown_fields)");
}
```

**Pattern** (Phase 6 US4): Unit tests for settings struct parse + layering. Verify `Option<bool>` field presence/absence, strict strictness enforcement (NFR-010), correct resolver wiring for both the value and the absence case. Used for first-declarer-wins + default-false pinning.

### Phase 6 US5: Doctor Read-Only Projection Tests

When testing doctor surfaces, verify they re-read state without writing directories (FR-124 read-only invariant):

```rust
// tests/doctor_p6.rs
#[test]
fn doctor_p6_surface_creates_no_dirs() {
    // Pre-stage on-disk state: enabled plugins, hooks/guardrails/agents/personas configured
    let fix = Fixture::build("test-workspace");
    env.cmd().args(["plugin", "enable", &plugin_id]).output().unwrap();
    
    // Count files before doctor
    let before_count = count_all_files(&fix.home);
    
    // Run doctor (read-only projection)
    let out = env.cmd().args(["doctor", "--json"]).output().unwrap();
    assert!(out.status.success());
    
    // Count files after — exact match proves read-only (no .tome/data dir creation)
    let after_count = count_all_files(&fix.home);
    assert_eq!(before_count, after_count, "doctor must not create any directories");
}

#[test]
fn doctor_hooks_report_shows_contributed_and_missing() {
    // Plugin has hooks, enable it
    let fix = Fixture::with_hooks();
    env.cmd().args(["plugin", "enable", &fix.plugin_id]).output().unwrap();
    
    // Run doctor
    let out = env.cmd().args(["doctor", "--json"]).output().unwrap();
    let report: DoctorOutput = serde_json::from_slice(&out.stdout).unwrap();
    
    // Hooks report present
    assert!(report.hooks.is_some(), "hooks report created");
    let hooks = report.hooks.unwrap();
    
    // Plugin entry shows contributed count (hooks in file)
    // and missing count (hooks not in file)
    assert_eq!(hooks.plugins.len(), 1);
    assert_eq!(hooks.plugins[0].plugin, fix.plugin_id);
    assert!(hooks.plugins[0].contributed.len() > 0 || hooks.plugins[0].missing.len() > 0);
}

#[test]
fn doctor_privilege_report_groups_by_plugin() {
    // Plugin with privileged agent fields
    let fix = Fixture::with_privileged_agent();
    env.cmd().args(["plugin", "enable", &fix.plugin_id]).output().unwrap();
    
    // Run doctor
    let out = env.cmd().args(["doctor", "--json"]).output().unwrap();
    let report: DoctorOutput = serde_json::from_slice(&out.stdout).unwrap();
    
    // Privilege report present and grouped
    assert!(report.privilege_escalation.is_some());
    let priv_report = report.privilege_escalation.unwrap();
    assert_eq!(priv_report.plugins.len(), 1);
    let plugin_entry = &priv_report.plugins[0];
    
    // Agent entry includes fields (hooks, mcpServers, permissionMode)
    assert!(plugin_entry.agents[0].fields.contains(&"hooks".to_owned()));
}

#[test]
fn doctor_fix_rerenders_hooks_idempotently() {
    let fix = Fixture::with_hooks();
    env.cmd().args(["plugin", "enable", &fix.plugin_id]).output().unwrap();
    
    // Run doctor --fix hooks
    let out = env.cmd().args(["doctor", "--fix"]).output().unwrap();
    assert!(out.status.success());
    
    // Re-run doctor (no changes)
    let out2 = env.cmd().args(["doctor", "--json"]).output().unwrap();
    let report2: DoctorOutput = serde_json::from_slice(&out2.stdout).unwrap();
    
    // Hooks report shows same counts as before (idempotent)
    assert_eq!(report2.hooks.as_ref().unwrap().plugins[0].missing.len(), 0);
}
```

**Pattern** (Phase 6 US5 / FR-124): Library-API tests for doctor check functions. Each check function is read-only: re-reads the state the sync path produced, compares against actual on-disk, records observations in a report. Tests verify: hooks/guardrails/agents/personas reports render when conditions met, privilege-escalation groups correctly, `--fix` re-renders idempotently, and exact-count file-count assertion proves no directory creation. Covered in `tests/doctor_p6.rs`.

### Phase 6 US5: Doctor Byte-Stable JSON Pins

When testing doctor output, pin byte-stable JSON shapes for every emitted report type:

```rust
// tests/doctor_p6_json_shape.rs
#[test]
fn hooks_report_wire_shape_byte_stable() {
    // Deterministic fixture: one plugin, two hooks events
    let fix = Fixture::with_deterministic_hooks();
    
    let report = build_hooks_report(&paths, &project, &workspace, &conn).unwrap();
    let json = serde_json::to_string(&report).unwrap();
    
    // Pin the exact byte sequence (alphabetical field order, compact formatting)
    assert_eq!(json, r#"{"plugins":[{"catalog":"test-cat","plugin":"test-plugin","contributed":[{"count":2,"event":"onCreateFile"}],"missing":[]}]}"#);
}

#[test]
fn privilege_escalation_report_appended_last_in_doctor_output() {
    // Privilege report is a Phase 6 US5 addition; pin its position LAST
    let output = DoctorOutput { ... };
    let json = serde_json::to_string(&output).unwrap();
    
    // Extract the JSON and verify field order: hooks, guardrails, agents, personas, privilege_escalation
    let obj = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    let keys: Vec<&str> = obj.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    let priv_index = keys.iter().position(|&k| k == "privilege_escalation");
    let persona_index = keys.iter().position(|&k| k == "personas");
    
    assert!(priv_index > persona_index, "privilege_escalation appended after personas");
}
```

**Pattern** (Phase 6 US5 / wire-shape pins): Byte-stable JSON pins for every new Phase 6 doctor report type. The doctor `HooksReport`, `GuardrailsReport`, `AgentsReport`, `PersonasReport`, and `PrivilegeEscalationReport` are appended LAST to `DoctorOutput` to preserve existing bytes. Tests verify field order, serialization format, and `skip_serializing_if` behavior. Covered in `tests/doctor_p6_json_shape.rs`.

### Phase 6 US5: Privilege Strip + Audit Separation Tests

When testing privilege escalation, verify that stripping affects emission only, not the audit source:

```rust
// tests/agent_privilege.rs
#[test]
fn privilege_strip_only_affects_emission_not_audit() {
    let fix = Fixture::with_privileged_agent();
    let paths = setup_paths(&fix);
    let conn = index::open(&paths)?;
    
    // Enable agent with privileged fields
    lifecycle::enable(&plugin_id, &deps, false)?;
    
    // Audit reads unstripped source
    let priv_report = build_privilege_escalation_report(&paths, &workspace, &conn)?;
    assert_eq!(priv_report.plugins[0].agents[0].fields.len(), 3);  // hooks + mcpServers + permissionMode
    
    // Emit with strip = true
    let canonical = CanonicalAgent::parse(...)?;
    let emitted_yaml = emit_claude_code_agent(&canonical, true)?;  // strip = true
    
    // Emitted YAML has no privileged fields
    assert!(!emitted_yaml.contains("hooks:"));
    assert!(!emitted_yaml.contains("mcpServers:"));
    assert!(!emitted_yaml.contains("permissionMode:"));
    
    // Re-run audit (source unchanged, still reports privileged fields)
    let priv_report2 = build_privilege_escalation_report(&paths, &workspace, &conn)?;
    assert_eq!(priv_report2.plugins[0].agents[0].fields.len(), 3);
}
```

**Pattern** (Phase 6 US5 / FR-051): Privilege audit and strip are decoupled. The audit path reads the unstripped source (canonical agent frontmatter as-is) and reports which fields are present. The emission path works on a clone that may be stripped before rendering. Tests verify the two paths stay independent: stripping the emission doesn't affect the audit, and the audit always sees the true source. Covered in `tests/agent_privilege.rs`.

## Test Data

### Fixtures

**Catalog fixture** (`tests/fixtures/sample-catalog/`):
- Git repo skeleton with `tome-catalog.toml` manifest
- Two sample plugins (`hello`, `goodbye`) with plugin.json manifests
- Copied into temp dir by `Fixture::build_sample()` for each test

**Plugin catalog fixture** (`tests/fixtures/sample-plugin-catalog/`):
- Same structure; used by workspace/lifecycle tests via `copy_sample_plugin_catalog()`

### Fabrication Helpers

| Helper | Purpose | Output |
|--------|---------|--------|
| `fabricate_models(paths)` | Create manifest.json for every model | `~/.tome/models/{name}/manifest.json` |
| `fabricate_installed_models(paths, entries)` | Fabricate model artefact files (sparse) | Manifest + sparse artefact files |
| `fabricate_all_registry_models(paths)` | Fabricate every entry in `MODEL_REGISTRY` | All 3 models (embedder, reranker, summariser) |
| `write_index_db_with_schema_version(path, v)` | Synthetic DB with minimal schema | `/path/index.db` at version `v` |
| `write_config_for_cli(paths, config)` | Seed catalog config + enrol in DB | `config.toml` + `workspace_catalogs` rows |
| `seed_workspace(paths, name)` | Inject workspace row into DB | `workspaces` table entry |

**Sparse file pattern**: `File::set_len(size)` creates zero-filled files that take ~no disk space. Embedder fixture is 66 MB but occupies 0 bytes on disk. SHA-256 mismatch is intentional for `--verify` tests.

## Mocking Strategy

### Stub Embedder (`src/embedding/stub.rs`)

Deterministic embedder that produces fixed vectors based on input. Used in all tests that don't need real inference.

```rust
pub struct StubEmbedder {
    // Produces consistent vectors for the same input
}

#[test]
fn plugin_enable_uses_embedder() {
    let _guard = EmbedderGuard::install(Arc::new(StubEmbedder::new()));
    // Test proceeds with stub instead of loading ONNX models
}
```

**Override mechanism**: `EMBEDDER_OVERRIDE` slot at `src/embedding/mod.rs`, installed via `EmbedderGuard::install()` in `tests/common/mod.rs`.

### Stub Reranker

Similar pattern to embedder; deterministic ranking by vector sum.

### Stub Summariser (Phase 4)

Deterministic text summarization (returns fixed text) instead of loading Qwen2.5 model. Override via `SUMMARISER_OVERRIDE` slot.

### Stub Harness (Phase 6)

Test-only deterministic harness implementation in `src/harness/stub.rs`, configurable via builder pattern. Override via `HARNESS_MODULES_OVERRIDE` slot.

### Test-Only Injection Points

| Slot | Override Guard | Used For |
|------|----------------|----------|
| `EMBEDDER_OVERRIDE` | `EmbedderGuard` | Stub embedder in tests |
| `RERANKER_OVERRIDE` | `RerankerGuard` | Stub reranker in tests |
| `SUMMARISER_OVERRIDE` | `SummariserOverrideGuard` | Stub summariser (Phase 4) |
| `HARNESS_MODULES_OVERRIDE` | `HarnessModulesGuard` | Synthetic harness registry (Phase 6) |
| `MIGRATIONS_OVERRIDE` | `MigrationsGuard` | Synthetic schema migrations |
| `SUBSTITUTION_CLOCK_OVERRIDE` | `ClockOverrideGuard` | Fixed system clock (Phase 5) |
| `PLUGIN_DATA_DIR_OVERRIDE` | `PluginDataDirGuard` | Plugin data directory (Phase 5) |
| `WORKSPACE_DATA_DIR_OVERRIDE` | `WorkspaceDataDirGuard` | Workspace data directory (Phase 5) |

All defined in `tests/common/mod.rs` with RAII drop guards.

## Coverage Requirements

| Metric | Target | Current | Notes |
|--------|--------|---------|-------|
| Exit codes | All enumerated variants | ✓ | `tests/exit_codes.rs` grep guard |
| CLI binary paths | Representative sampling | ✓ | Exit codes + output format tested |
| Library API | 100% on public surface | ✓ | Unit tests in modules |
| Error Display | All variants | ✓ | `tests/error_messages.rs` |
| JSON wire shapes | Byte-stable pins | ✓ | `tests/*_json_shape.rs` (Phase 4+) |
| Agent translation | Per-harness contract | ✓ | `tests/agent_translate_*.rs` (Phase 6 US1) |
| Hooks rewrite & merge | Boundary + integration | ✓ | `tests/hooks_*.rs` (Phase 6 US2) |
| Guardrails regions | Rendering, validation, atomicity | ✓ | `tests/guardrails_*.rs` (Phase 6 US3) |
| Agent personas | Toggle, rendering, substitution, collision | ✓ | `tests/personas*.rs` (Phase 6 US4) |
| Settings scalar resolution | First-declarer-wins, layering, defaults | ✓ | `tests/settings_p6.rs` + `tests/personas_startup_scope.rs` (Phase 6 US4) |
| Doctor extensions | Hooks/guardrails/agents/personas/privilege reports; --fix idempotence | ✓ | `tests/doctor_p6.rs`, `tests/doctor_p6_json_shape.rs` (Phase 6 US5) |

**Exclusions**: ONNX inference (real model load excluded; library `fastembed` tests own path), real model downloads (fabricated fixtures instead), MCP protocol purity (deferred T093–T095).

**Phase 6 US5**: Doctor read-only projection via library API (no CLI spin-up for report tests, matching `doctor_p5.rs` pattern). Direct calls to `build_hooks_report`, `build_guardrails_report`, `build_agents_report`, `build_personas_report`, `build_privilege_escalation_report`. JSON shapes byte-stable via pins. `--fix` idempotence via re-invocation of `sync_project`. Privilege strip/audit separation verified. All tested via library API (no CLI spawn for report detail tests, matching established patterns).

## Test Categories by Purpose

### Smoke Tests

Critical path tests that must pass before deploy:

| Test | Purpose |
|------|---------|
| `catalog_add.rs::happy_path_human_mode` | Core catalog registration flow |
| `plugin_enable.rs::happy_path_json_mode` | Core plugin enable flow |
| `query.rs::happy_path` | Core search + ranking flow |
| `workspace_use.rs::happy_path` | Core project binding flow |
| `doctor.rs::assemble_report_happy_path` | Core diagnostic flow (Phase 5 US5) |

### Regression Tests

Tests for previously fixed bugs, linked to phase retros:

| Category | Retro | Example |
|----------|-------|---------|
| Phase 4 US1 | `retro/P3.md` | `sync_idempotence.rs` (Sync twice → no changes) |
| Phase 4 US3 | `retro/P5.md` | `workspace_commands.rs` (Scope isolation) |
| Phase 5 US3 | `retro/P5.md` | `entry_kind_indexing.rs` (Entry kind + collision handling) |
| Phase 5 US5 | (current) | `doctor_phase5_surface_creates_no_dirs` (FR-124 read-only invariant) |
| Phase 6 Foundational F2 | (current) | `entry_kind_agent_indexing.rs` (Agent row integration; schema drift prevention) |
| Phase 6 US1 S-1 | (current) | `agent_path_traversal.rs` (Index-time gate blocks ../../../../tmp/evil) |
| Phase 6 US3 B-1 | (current) | `guardrails_marker_injection.rs` (Fail-closed marker validation) |
| Phase 6 US4 R-4-2 | (current) | `personas_startup_scope.rs` (Single-source-of-truth scope-loaders) |
| Phase 6 US5 FR-124 | (current) | `doctor_p6.rs::surface_creates_no_dirs` (Read-only invariant proof) |
| Phase 6 US5 FR-051 | (current) | `agent_privilege.rs` (Audit/strip separation) |

### Invariant Tests

Tests that verify core properties hold:

| Property | Test File | Checks |
|----------|-----------|--------|
| Manifest strictness | `manifest_strictness.rs` | All Tome-owned types have `#[serde(deny_unknown_fields)]` |
| Exit code completeness | `exit_codes.rs` | All `TomeError` variants are covered |
| Syncability | `sync_idempotence.rs` | Harness sync is idempotent |
| Atomicity | `atomicity.rs` | Partial failures leave committed state |
| JSON wire shape | `*_json_shape.rs` | Serialization is deterministic + byte-stable |
| Read-only invariant | `doctor_p5.rs` | `doctor assemble_report` creates no directories (Phase 5 US5.a) |
| Exact-count pins | `plugin_show_p5.rs`, `doctor_p5.rs`, `doctor_json.rs` | Deterministic fixture counts stay exact (Phase 5 Polish + Phase 6) |
| Canonical enum dispatch | `entry_kind_agent_indexing.rs` | Exhaustive match on `EntryKind` surfaces schema drift (Phase 6 F2) |
| Marker migration | `schema_migration_p6.rs` | Version bump advances without DDL (Phase 6 Foundational) |
| Filename provenance | `agent_removal.rs` | `<plugin>__<name>` is the sole provenance rule (Phase 6 US1) |
| Agent embedding skip | `entry_kind_agent_indexing.rs` | Agent rows are never embedded; queries filter on `embedding IS NOT NULL` (Phase 6 US1) |
| Path-traversal defence | `agent_path_traversal.rs` | Attacker-controlled `name: ../../../../tmp/evil` rejected at index time (S-1) |
| Display name clash | `agent_naming_clash.rs` | Two agents with same `<name>` show plugin-prefixed display names (FR-041) |
| Guardrails region ordering | `guardrails_render.rs` | Regions rendered in lex order; overwrite-in-place deterministic (FR-014) |
| Guardrails marker validation | `guardrails_marker_injection.rs` | Any managed-marker line in body rejected (B-1 fail-closed) |
| Rules-file idempotence | `rules_file_block_in_existing.rs`, `rules_file_standalone.rs` | Re-write with no change rewrites nothing (FR-525) |
| Persona toggle + rendering | `personas.rs` | Toggle on/off includes/excludes personas; body wrapped + substituted (FR-060/062/064) |
| Persona collision | `personas_collision.rs` | Clashing persona names are prefixed; drop-persona reserved (FR-061/063) |
| Settings scalar resolution | `settings_p6.rs` | First-declarer-wins walk; project/workspace/global layering (FR-053) |
| Doctor read-only | `doctor_p6.rs` | Report generation creates no directories (exact-count file count proof) (FR-124) |
| Doctor privilege report | `doctor_p6.rs` | Privilege report groups by plugin, lists privileged fields (FR-051) |
| Doctor byte-stable output | `doctor_*_json_shape.rs` | Wire-shape pins for all Phase 6 reports (appended LAST, `skip_serializing_if`) (Phase 6 US5) |
| Privilege strip idempotence | `agent_privilege.rs` | Strip setting only affects emission, not audit source (FR-051) |

### Phase 5: Truncation Boundary Tests

Tests for string truncation edge cases (US4.d + Polish M-1 pattern):

| Test | Checks |
|------|--------|
| `mcp_tool_description.rs::truncate_respects_char_boundaries_with_emoji()` | Multi-byte UTF-8 char slicing |
| `mcp_search_skills_truncation.rs::truncation_at_multibyte_char_boundary_does_not_split_codepoint()` | Emoji boundaries (Polish M-1) |
| `entry_kind_*.rs::search_skills_description_truncation_*()` | Description max-length enforcement |
| `substitution_*.rs::argument_value_truncation_boundary()` | Argument coercion with limits |

### Phase 5: Exact-Count + Empty-Section Invariant Tests

Tests that verify deterministic entity counts and collection states (US5.b + Polish patterns):

| Test | Checks | Pattern |
|------|--------|---------|
| `plugin_show_p5.rs::dormant_entry_annotated()` | Dormant bit set correctly | Positive assertion |
| `plugin_show_p5.rs::dormant_not_annotated_when_searchable_true()` | Boolean-logic negative case (T-G1) | Explicit "NOT" test |
| `doctor_p5.rs::empty_section_arrays_present_not_omitted()` | Empty arrays serialize; not omitted (T-G2) | Presence invariant |
| `doctor_json.rs::entry_counts_by_kind_exact_match()` | Exact skill/command/agent counts match fixture (T-W1) | Exact-count discipline |
| `doctor_p5.rs::pending_re_embedding_zero_when_no_files_touched()` | Zero re-embeds when nothing changed (GAP-2, Polish) | Zero-state assertion |

**Rationale** (Polish phase learnings): The zero-state and empty-section invariant tests catch "off-by-one forgot to reset" bugs. Phase 5 Polish T-W1 introduced the pattern; now applied to pending counts and empty arrays. Together with positive tests, this three-case coverage (positive/negative/zero/empty) becomes the canonical pattern for deterministic fixtures.

### Phase 6: Agent Translation Contract Tests

Tests that verify each harness's agent format and field-mapping contract (US1):

| Test | Harness | Checks |
|------|---------|--------|
| `agent_translate_claude_code.rs::body_lands_in_frontmatter()` | Claude Code | MarkdownYaml format; frontmatter keys; model pass-through |
| `agent_translate_codex.rs::body_lands_in_triple_quoted_developer_instructions()` | Codex | TOML format; developer_instructions triple-quote; model DROP |
| `agent_translate_cursor.rs::format_and_filename_match_contract()` | Cursor | MarkdownYaml format; model DROP (no alias) |
| `agent_translate_opencode.rs::model_maps_to_qualified_anthropic_id()` | OpenCode | MarkdownYaml format; `opus` → `anthropic/claude-opus-4.7`; display name override |

**Pattern** (Phase 6 US1): Each harness has a `translate_agent` test file verifying the contract (`contracts/agent-translation.md` SC-001 row). Direct calls to `HarnessModule::translate_agent` with hand-crafted `CanonicalAgent` fixtures. Tests verify:
- Format (MarkdownYaml or Toml)
- Filename (always `<plugin>__<name>.<ext>`)
- Body placement (frontmatter vs triple-quoted)
- Model mapping (same-vendor-only)
- Dropped-fields vector
- Read-only inference (tools → sandbox_mode or not)

### Phase 6: Hooks Rewrite and Merge Tests (US2)

Tests that verify real Claude Code hooks path-variable rewriting and config file merging (US2):

#### Hooks Rewrite Tests (`tests/hooks_rewrite.rs`)

| Test | Checks |
|------|--------|
| `resolves_plugin_root_and_data_leaves_others_verbatim()` | Two tokens resolved to absolute paths; `${CLAUDE_PROJECT_DIR}` / `${CLAUDE_SESSION_ID}` left verbatim |
| `only_string_values_rewritten_keys_untouched()` | Non-string scalars (numbers) untouched; token-looking keys preserved verbatim; only VALUES rewritten |
| `absent_hooks_file_is_none()` | Plugin with no `hooks/hooks.json` yields `Ok(None)` (benign fall-through) |
| `symlinked_hook_source_is_refused_exit_7()` | Symlinked source path rejected → exit 7 (Unix only) |
| `malformed_hooks_file_is_exit_43()` | Invalid JSON in `hooks/hooks.json` → exit 43, error names the file |

**Pattern** (Phase 6 US2 / T069): Library-API tests for `harness::hooks::read_rewritten_entries`. Fixed-needle `str::replace` (not regex, not Phase 5 substitution pipeline) rewrites exactly two tokens in string VALUES only. Keys and non-string scalars survive. Contract: `contracts/hooks-integration.md` § "Path-variable rewriting" (FR-003, R-4).

#### Hooks Merge and Removal Tests (`tests/hooks_merge.rs`)

| Test | Checks |
|------|--------|
| `create_if_absent_never_touches_committed_settings()` | Creates `settings.local.json` + `.claude/` (0700); committed `settings.json` never written |
| `idempotent_re_add_is_deep_equal_skip_no_rewrite()` | Deep-equal entry already present → skip, mtime stable |
| `user_authored_identical_entry_not_duplicated()` | User-authored entry matching deep-equal entry not duplicated (FR-004) |
| `user_edited_entry_preserved_on_removal()` | Non-matching (user-edited) entry left in place on removal (NFR-003) |
| `removal_prunes_empty_event_but_keeps_hooks_object()` | Empty event array pruned (FR-006); empty `hooks` object kept |
| `removal_against_absent_file_is_noop()` | Removing from absent file is a no-op; no file created |
| `multi_plugin_merge_in_one_pass_then_idempotent()` | Two plugins merge in separate calls; re-merge is deep-equal skip, mtime stable |
| `multi_event_removal_prunes_only_the_target_event()` | Multi-event settings: remove event A, event B + user entries survive |
| `merge_into_wrong_type_settings_is_exit_44_original_intact()` | Malformed (wrong-type) settings → exit 44, original file byte-for-byte intact |
| `merge_into_wrong_type_hooks_value_is_exit_44_original_intact()` | `hooks` value wrong type → exit 44, original intact |
| `merge_through_symlinked_settings_is_refused_exit_7()` | Symlink target for merge → exit 7, decoy untouched (Unix only) |
| `remove_through_symlinked_settings_is_refused_exit_7()` | Symlink target for remove → exit 7, decoy untouched (Unix only) |
| `merge_preserves_unrelated_user_keys_and_appends()` | User keys (`"model": "opus"`) preserved; distinct hook appended to event |

**Pattern** (Phase 6 US2 / T070): Library-API tests for `harness::hooks::{merge_into_settings, remove_from_settings}`. Structural-match (deep-equal) ownership: add only if absent; remove only if present (no sidecar provenance, NFR-003). Create-if-absent on merge (settings.local.json, never settings.json). Mtime stability on idempotent operations. Contract: `contracts/hooks-integration.md` § "Merge semantics" / "Removal semantics" (FR-002/004/005/006).

#### Hooks Sync Tests (extensions to `tests/harness_sync_stub.rs`)

| Test | Checks |
|------|--------|
| `hooks_forward_progress_one_malformed_one_good()` | One malformed plugin skipped; good plugin's hooks merge continues; first error recorded |
| `merge_through_symlinked_settings_is_refused_exit_7()` | Symlink refusal during sync hooks merge → exit 7 |
| `multi_plugin_hooks_merge_in_one_pass()` | Multi-plugin hooks from enabled plugins all merge into one `settings.local.json` in a single pass |

**Pattern** (Phase 6 US2 / T2-2): Full-stack sync tests via `sync_project` with `StubHarness` override. Forward-progress: one plugin with malformed `hooks.json` is skipped (error recorded in `first_error`); sibling plugins' hooks continue to merge. Symlink refusal on settings write. Multi-plugin merge via `reconcile_hooks` passes all enabled plugins' rewritten hooks in one call, idempotent on re-sync. Mirrors agent forward-progress pattern (T-4).

#### Hooks Exit Code Tests (extensions to `tests/exit_codes_e2e.rs`)

| Test | Checks |
|------|--------|
| `workspace_use_malformed_hooks_exits_43()` | Malformed `hooks/hooks.json` during `workspace use` → exit 43 |

**Pattern** (Phase 6 US2 / T-3): E2E exit code test for exit 43 (HookSpecParseError). Exit 44 (HookSettingsWriteFailed) tested library-API-only in `hooks_merge.rs` (IO failures not cheaply forced through CLI). Contract: `contracts/exit-codes-p6.md` § "Discipline" (codes 43/44 split between e2e and library).

### Phase 6: Guardrails Rendering and Validation Tests (US3)

Tests that verify guardrails region rendering, marker validation, and atomicity (US3):

#### Guardrails Render Tests (`tests/guardrails_render.rs`)

| Test | Checks |
|------|--------|
| `regions_rendered_in_lexicographic_order()` | Multiple guardrails regions rendered in lex order of key (`<catalog>:<plugin>`) |
| `new_region_appended_in_lex_order()` | New desired keys appended AFTER existing regions, in lex order |
| `existing_region_overwritten_in_place()` | Changed body for existing key: region between markers rewritten in place (no move) |
| `orphaned_region_removed()` | Key in file but not desired: region removed entirely |
| `idempotent_re_sync_rewrites_nothing()` | Re-run with same desired map: file bytes unchanged (short-circuit compare) |
| `multiple_harnesses_regions_on_same_file()` | `AGENTS.md` holds regions for both codex and opencode; ordering deterministic |

**Pattern** (Phase 6 US3 / T3-1): Library-API tests for `guardrails::reconcile_in_file_region`. Direct calls to the reconciler with `BTreeMap<String, String>` of desired regions. Tests verify idempotence (FR-525), deterministic lexicographic ordering (FR-014), in-place overwrite (no shuffling), new-append, orphan-removal. Verified in `contracts/guardrails.md` (FR-011/014/015).

#### Guardrails Suppression Tests (`tests/guardrails_suppression.rs`)

| Test | Checks |
|------|--------|
| `suppressed_harness_skips_guardrails_region()` | Harness key in suppression map → `reconcile_in_file_region` not called for that harness |
| `non_suppressed_writes_region()` | Harness key not in suppression map → `reconcile_in_file_region` called; region written |
| `agents_md_not_corrupted_when_claude_code_suppressed()` | Claude Code suppressed but codex not: `AGENTS.md` holds codex region only; `CLAUDE.md` holds claude-code hooks-via-@include; cross-file invariant holds |

**Pattern** (Phase 6 US3 / T3-3): Full-stack tests via `sync_project` with two harnesses (claude-code + codex), suppression applied to claude-code (because it shipped hooks JSON). Verify codex guardrails region lands in `AGENTS.md` while claude-code skips its region (FR-013). Harness suppression computed by the sync orchestrator based on the hooks set (a per-plugin per-harness calculation).

#### Guardrails Marker Injection Tests (`tests/guardrails_marker_injection.rs`)

| Test | Checks |
|------|--------|
| `body_with_guardrails_start_marker_rejected()` | GUARDRAILS.md body contains `<!-- START GUARDRAILS: … -->` line → rejected (exit 46) |
| `body_with_guardrails_end_marker_rejected()` | GUARDRAILS.md body contains `<!-- END GUARDRAILS: … -->` line → rejected (exit 46) |
| `body_with_tome_block_marker_rejected()` | GUARDRAILS.md body contains `<!-- tome:begin -->` or `<!-- tome:end -->` line → rejected (exit 46) |
| `sibling_plugin_region_renders_despite_crafted_body()` | One plugin has crafted (rejected) GUARDRAILS.md; sibling plugin with valid body still renders; re-sync convergent |

**Pattern** (Phase 6 US3 / T3-2 B-1): Library-API tests for fail-closed marker validation. Fixture seeds three plugins: one with each marker type in the body, plus a valid sibling. `sync_project` via `reconcile_guardrails` reads all three sources; the three with markers surface exit 46 and record error in forward-progress slot; the valid plugin's region renders. Re-sync on the same fixture returns cleanly (idempotent). Verified in `contracts/guardrails.md` (FR-084, fail-closed refusal on body-escape attempt).

#### Guardrails Atomicity Tests (extensions to `tests/atomicity.rs`)

| Test | Checks |
|------|--------|
| `guardrails_in_file_write_failure_leaves_target_byte_unchanged()` | Mid-write failure (read-only parent prevents sibling tempfile creation) on in-file guardrails target leaves file byte-for-byte unchanged |

**Pattern** (Phase 6 US3 / T3-2): Interrupt-injection test. Fixture seeds an existing guardrails region in a target file, then attempts a `reconcile_in_file_region` call with a changed body. The parent directory is made read-only so the atomic write (sibling tempfile creation) fails after the desired body is computed. Verifies the failure surfaces exit 46 + the file is byte-for-byte intact (old region in place, no partial update). Restored to read-write before cleanup.

### Phase 6: Rules-File Strategy Tests (US3)

Tests that verify rules-file block and standalone strategies, Phase 4 correction (US3):

#### Rules-File Block Tests (`tests/rules_file_block_in_existing.rs`)

| Test | Checks |
|------|--------|
| `block_overwrites_in_place_preserves_surrounding_content()` | `BlockInExistingFile`: block updated, content outside markers unchanged |
| `block_inserted_into_empty_file()` | Empty target file: block inserted with correct markers and newline discipline |
| `idempotent_write_rewrites_nothing()` | Body unchanged: file bytes match desired; write short-circuits (no rewrite) |
| `symlinked_target_is_refused_exit_7()` | Symlinked target path rejected before write → exit 7 (Unix only) |
| `malformed_block_markers_detected_and_rejected()` | Nested `begin` or unmatched `end` in file → rejected (IO variant, not TomeError) |

**Pattern** (Phase 6 US3 / T081): Library-API tests for `rules_file::write_block_in_file`. Direct calls avoid CLI spin-up. Tests verify marker preservation, content outside markers stays intact, symlink refusal (exit 7), atomic write semantics, idempotence. Covers `BlockInExistingFile` strategy per `contracts/rules-file-integration.md` (FR-525).

#### Rules-File Standalone Tests (`tests/rules_file_standalone.rs`)

| Test | Checks |
|------|--------|
| `standalone_file_created_if_absent()` | Target absent: file created with marker-wrapped body |
| `standalone_file_replaced_entirely()` | Target exists: file replaced (not merged) |
| `standalone_file_removal_deletes_file()` | `RulesFileStrategy::StandaloneFile`: removal deletes the file entirely |
| `idempotent_write_rewrites_nothing()` | Body unchanged: write short-circuits |
| `symlinked_target_is_refused_exit_7()` | Symlinked target path rejected before write → exit 7 (Unix only) |

**Pattern** (Phase 6 US3 / T082): Library-API tests for `rules_file::write_standalone_file` and removal. Direct calls verify file creation/replacement, symlink refusal (exit 7), atomic semantics, idempotence, removal deletion. Covers `StandaloneFile` strategy per `contracts/rules-file-integration.md` (FR-525).

#### Phase 4 Rules-File Correction Tests (`tests/rules_file_claude_correction.rs`)

| Test | Checks |
|------|--------|
| `rules_block_lands_in_claude_md_not_agents_md()` | Claude Code harness candidate set: `CLAUDE.md` first; `AGENTS.md` skipped entirely (Phase 4 correction FR-020) |
| `agents_md_used_for_codex_blocks_claude_md_for_claude_code()` | Multi-harness project: codex writes `AGENTS.md`; claude-code writes `CLAUDE.md` (not both to AGENTS) |
| `both_blocks_resolve_same_project_rules_via_include()` | Both `CLAUDE.md` (claude-code) and `AGENTS.md` (codex) use `@`-includes to the same `.tome/RULES.md` |

**Pattern** (Phase 6 US3 / T083): Full-stack tests via `sync_project` driving real `claude-code` and `codex` harness modules. Fixtures create multi-harness projects with `.tome/RULES.md` and verify the correct target files are written per harness. Tests the Phase 4 correction that moves claude-code from `AGENTS.md` → `CLAUDE.md` (FR-020/021/022).

#### Rules-File Exit Code Tests (extensions to `tests/exit_codes_e2e.rs`)

| Test | Checks |
|------|--------|
| `guardrails_write_through_symlink_exits_46()` | Symlinked in-file guardrails target during sync → exit 46 (library-API, Unix only) |

**Pattern** (Phase 6 US3 / T-4): E2E exit code coverage for exit 46 (GuardrailsWriteFailed). Symlink refusal path exercises the fail-closed pattern cheaply. IO/render failures on rules-file targets tested library-API-only (not cheaply forced through CLI).

## CI Integration

### Test Pipeline (`.github/workflows/*`)

- Unit tests (parallel)
- Integration tests (parallel, with stub embedder)
- Binary size check (`target/release/tome` <= 50 MB)
- Clippy strict linting
- rustfmt check
- typos check

### Required Checks

| Check | Blocking | Runs On |
|-------|----------|---------|
| `cargo test` | Yes (main) | Every PR |
| `cargo clippy` | Yes | Every PR |
| `cargo fmt --check` | Yes | Every commit hook |
| `typos` | Yes | Every commit hook |
| Binary size | Yes (main) | Linux x86_64 |
| MSRV | Yes | CI only |

### Pre-Commit Hook

`.githooks/pre-commit` runs `cargo fmt --check`, `typos`, and `cargo clippy` sequentially. All three must pass before commit succeeds (no `--no-verify` bypasses without documented reason).

### Pre-Push Hook (Phase 5 Polish Change)

**Phase 5 Polish PR #126**: Pre-push hook now runs **local fmt/clippy/typos checks ONLY** (no full `cargo test --workspace`). Rationale: pre-push completes under ~1 minute on warm cache, staying responsive for local iteration. Full test suite runs in CI (GitHub matrix across Linux/macOS) as the source of truth. Test discipline is NOT relaxed — CI is the enforcement surface.

## Test Discipline

### One Assertion Per Test

Each test verifies one behavior. Related assertions on the same outcome are grouped, but independent checks get separate tests.

```rust
// Good: one concept per test
#[test]
fn catalog_add_success_updates_config() { ... }

#[test]
fn catalog_add_duplicate_exits_4() { ... }

// Bad: mixing multiple concerns
#[test]
fn catalog_add_works() {
    // Assert success
    // Assert config updated
    // Assert cache cloned
    // Assert manifest parsed
}
```

### Test Names

Descriptive, underscore-separated. Format: `{subject}_{action}_{expectation}`.

```rust
#[test]
fn catalog_add_duplicate_registration_exits_4() { ... }

#[test]
fn plugin_enable_missing_models_prompts_download() { ... }

#[test]
fn harness_use_composition_error_exits_17() { ... }

#[test]
fn doctor_p5_surface_creates_no_dirs() { ... }

#[test]
fn entry_kind_agent_injected_rows_counted_correctly() { ... }

#[test]
fn agent_path_traversal_rejected_at_index_time() { ... }

#[test]
fn guardrails_marker_injection_stray_end_rejected() { ... }

#[test]
fn persona_toggle_off_excludes_personas() { ... }

#[test]
fn expose_personas_project_overrides_global() { ... }

#[test]
fn doctor_p6_surface_creates_no_dirs() { ... }

#[test]
fn privilege_strip_only_affects_emission_not_audit() { ... }
```

### Minimal External I/O

- **Git**: real repo fabrication via `Fixture` (necessary for catalog tests).
- **HTTP**: none (no real downloads; fixtures or error paths).
- **Filesystem**: all under TempDir (no host state pollution).
- **ONNX models**: stub inference only (no real model load in test suite).
- **Time**: fixed via `ClockOverrideGuard` when needed (Phase 5).

### Deterministic Execution

- No flaky sleeps or timeouts.
- Stub embedder produces fixed vectors for deterministic test assertions.
- Concurrent tests serialized via `HOME_MUTEX` + RAII guards.
- No real time dependencies (fixed clock via `ClockOverrideGuard`).

### Phase 5: 4-Reviewer Parallel Pass Pattern

**Phase 5 Polish introduces this pattern at PHASE-WIDE scope** (distinct from per-US passes). Every Phase 5+ closeout runs a parallel 4-reviewer pass **once at the end** rather than per-user-story:

| Reviewer | Focus | Deliverable |
|----------|-------|-------------|
| Contract audit | Spec alignment, cross-US drift, contract amendments | `review/findings.md` + `review/disposition.md` |
| Rust-lens | Code review, idioms, safety, cross-US patterns | Inline code comments, M-1/M-2/M-3/M-4 fixes |
| Test audit | Coverage gaps, edge cases, invariant tests | Test additions (GAP-2, Polish M-1 truncation) |
| Security audit | Hardening, boundary validation, no new vectors | Security findings, deferred items |

Findings + disposition committed **BEFORE** fixes land (Phase 5 Polish PR pattern). Exemplified in Polish Polish: "4-reviewer pass surfaced 0 BLOCKERS + 4 majors + 1 test gap + 5 minors; applied 2 majors + 1 test."

**Impact**: Phase-wide passes catch cross-US drift earlier than per-US passes can. Emerged at Phase 5 Polish as the structural-safety net for multi-user-story phases.

## What Does NOT Belong Here

- Code style rules → `CONVENTIONS.md`
- Security testing → `SECURITY.md`
- Architecture patterns → `ARCHITECTURE.md`

---

*This document describes HOW to test. Update when testing strategy changes or a new pattern emerges.*
