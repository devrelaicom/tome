# Harness Modules — Contract

**Spec source**: [spec.md FR-460 through FR-462](../spec.md)
**Research**: [research.md R-8](../research.md)

## Trait

```rust
pub trait HarnessModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn detect(&self, home: &Path) -> bool;

    fn rules_file_target(&self, project_root: &Path) -> PathBuf;
    fn rules_file_strategy(&self) -> RulesFileStrategy;
    fn block_body_style(&self) -> BlockBodyStyle;     // consulted only for BlockInExistingFile

    fn mcp_config_path(&self, project_root: &Path, home: &Path) -> PathBuf;
    fn mcp_config_format(&self) -> McpConfigFormat;
    fn mcp_parent_key(&self) -> &'static str;          // "mcpServers" or "mcp_servers"
}
```

See [data-model.md §10](../data-model.md) for the enum definitions (`RulesFileStrategy`, `BlockBodyStyle`, `McpConfigFormat`).

## Static registry

```rust
static SUPPORTED_HARNESSES: &[&'static dyn HarnessModule] = &[
    &ClaudeCodeHarness,
    &CodexHarness,
    &GeminiHarness,
    &CursorHarness,
    &OpenCodeHarness,
];

pub fn lookup(name: &str) -> Option<&'static dyn HarnessModule>;
```

Lookup is case-sensitive against the harness's `name()`. An unknown name returns `None`; the caller (composition resolver, harness commands) maps `None` to `HarnessNotSupported` (exit 18).

## Per-harness specifics

### `claude-code`

```rust
impl HarnessModule for ClaudeCodeHarness {
    fn name(&self) -> &'static str { "claude-code" }
    fn description(&self) -> &'static str { "Anthropic's Claude Code CLI" }
    fn detect(&self, home: &Path) -> bool { home.join(".claude").is_dir() }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // Precedence: AGENTS.md > CLAUDE.md > .claude/CLAUDE.md
        for candidate in &["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"] {
            let p = project_root.join(candidate);
            if p.exists() { return p; }
        }
        project_root.join("AGENTS.md") // default if no existing file
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy { RulesFileStrategy::BlockInExistingFile }
    fn block_body_style(&self) -> BlockBodyStyle { BlockBodyStyle::AtInclude }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".claude/settings.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat { McpConfigFormat::Json }
}
```

The MCP config entry lives under the top-level `mcpServers` object: `{"mcpServers": {"tome": {"command": "tome", "args": ["mcp", "--workspace", "<name>"]}}}`.

### `codex`

```rust
fn name -> "codex"
fn description -> "OpenAI Codex CLI"
fn detect -> home.join(".codex").is_dir()
fn rules_file_target -> project_root.join("AGENTS.md")    // Codex CLI only reads AGENTS.md
fn rules_file_strategy -> BlockInExistingFile
fn block_body_style -> AtInclude
fn mcp_config_path -> home.join(".codex/config.toml")     // global; no per-project support
fn mcp_config_format -> Toml
```

The TOML entry lives under `[mcp_servers.tome]`: `[mcp_servers.tome]\ncommand = "tome"\nargs = ["mcp", "--workspace", "<name>"]`.

### `gemini`

```rust
fn name -> "gemini"
fn description -> "Google Gemini CLI"
fn detect -> home.join(".gemini").is_dir()
fn rules_file_target -> {
    for candidate in &["AGENTS.md", "GEMINI.md", ".gemini/GEMINI.md"] {
        let p = project_root.join(candidate);
        if p.exists() { return p; }
    }
    project_root.join("AGENTS.md")
}
fn rules_file_strategy -> BlockInExistingFile
fn block_body_style -> AtInclude
fn mcp_config_path -> home.join(".gemini/settings.json")  // global
fn mcp_config_format -> Json
```

Top-level `mcpServers` object pattern (same shape as Claude Code).

### `cursor`

```rust
fn name -> "cursor"
fn description -> "Cursor IDE"
fn detect -> home.join(".cursor").is_dir()
fn rules_file_target -> project_root.join(".cursor/rules/TOME_SKILLS.md")
fn rules_file_strategy -> StandaloneFile
fn block_body_style -> /* never consulted */ Inline
fn mcp_config_path -> project_root.join(".cursor/mcp.json")    // per-project
fn mcp_config_format -> Json
```

For Cursor, the rules file is fully Tome-owned; there is no block, no markers, just a complete file. Removal deletes the file. The MCP config object structure is `{"mcpServers": {"tome": ...}}`.

### `opencode`

```rust
fn name -> "opencode"
fn description -> "OpenCode CLI"
fn detect -> home.join(".opencode").is_dir()
fn rules_file_target -> project_root.join("AGENTS.md")
fn rules_file_strategy -> BlockInExistingFile
fn block_body_style -> Inline       // OpenCode does not document @-include support
fn mcp_config_path -> project_root.join("opencode.json")  // per-project, no dot prefix
fn mcp_config_format -> Json
```

OpenCode's MCP config shape: top-level `mcpServers` object. Block body is inline content (full rules text inside the `<!-- tome:begin -->` / `<!-- tome:end -->` markers).

## Verification at implementation time

Each per-harness `mcp_config_path`, `rules_file_target`, and `block_body_style` decision is locked here based on PRD-time research. The harness module contract makes specifics swappable behind one trait — if a harness changes its conventions between PRD and implementation, only that module's file changes; nothing in `sync`, the settings parser, or other commands is affected.

The first Phase 4 PR that touches a harness module records (in the PR body) a re-verification step: "verified against [harness docs URL] dated YYYY-MM-DD; no conflict found."

## Detection-only invariant

Per FR-167, `detect()` MUST be filesystem existence-only — no reading of the harness's own configuration files, no parsing of its rules, no inspection of its plugins. This invariant is enforced in tests by mocking `home` as a `TempDir` containing only the expected directory.
