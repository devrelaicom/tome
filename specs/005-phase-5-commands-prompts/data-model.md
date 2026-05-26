# Phase 5 — Data Model

Concrete Rust-level type definitions for Phase 5. Implementation-shape pinned where the eventual implementation has freedom; contract-shape pinned where downstream code (tests, MCP responses, CLI JSON envelopes) consumes the type.

## 1. Schema v3 (the central SQLite index)

The Phase 4 schema (`skills` table + `skill_embeddings` virtual table + `workspaces`, `workspace_catalogs`, `workspace_skills` junctions) is extended for Phase 5. The migration is the second registered migration (Phase 4's was the first).

### 1.1 `skills` table — extended

```sql
-- Phase 5 migration: ALTER TABLE add columns + widen unique constraint
ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN when_to_use TEXT;
DROP INDEX IF EXISTS skills_unique;
CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);
```

Existing columns preserved verbatim: `catalog`, `plugin`, `name`, `description`, `path`, `content_hash`, `enabled`, `indexed_at`, etc.

Type mapping (Rust):
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryRow {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    pub description: String,
    pub when_to_use: Option<String>,
    pub path: PathBuf,
    pub content_hash: String,
    pub searchable: bool,
    pub user_invocable: bool,
    pub indexed_at: time::OffsetDateTime,
    // ... existing fields ...
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Skill,
    Command,
}
```

Notes:
- `EntryKind` serialises as `"skill"` or `"command"` (matching the SQL string values).
- `searchable` defaults true; flipped false by frontmatter `disable-model-invocation: true`.
- `user_invocable` default depends on entry kind (false for skill, true for command) — set at insert time in `index::skills::upsert_skill` based on parsed frontmatter.
- `when_to_use` is nullable in SQL, `Option<String>` in Rust.

### 1.2 `skill_embeddings` virtual table — unchanged

Vector store (`sqlite-vec`) unchanged. The Phase 5 change to the `embedding_text` composer (R-12) regenerates the embeddings via the existing content-hash diffing path; no schema change to the virtual table.

### 1.3 `workspace_skills` junction — unchanged structure, widened FK semantics

Schema unchanged. The FK `(catalog, plugin, name)` reference in Phase 4 widens implicitly to `(catalog, plugin, kind, name)` via the new unique constraint. Existing rows reference `kind='skill'` rows by the migration's backfill (FR-111a). New `command`-kind rows added at plugin enable time by `lifecycle::enable_plugin`.

### 1.4 Migration shape

```rust
// src/index/migrations.rs (existing module; Phase 5 registers a new Migration)
pub const MIGRATIONS: &[Migration] = &[
    // Phase 4 first migration (v1 -> v2) ...
    Migration {
        from: 2,
        to: 3,
        name: "phase5_entry_kind_unification",
        apply: phase5_v3_apply,
    },
];

fn phase5_v3_apply(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch(
        "ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
         ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
         ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE skills ADD COLUMN when_to_use TEXT;
         DROP INDEX IF EXISTS skills_unique;
         CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);"
    )?;
    Ok(())
}
```

Tested via the existing `MIGRATIONS_OVERRIDE` thread-local seam (`tests/schema_migration_v3.rs`).

---

## 2. Frontmatter (parsed lenient, per third-party strictness boundary)

`src/plugin/frontmatter.rs` extended.

```rust
#[derive(Debug, Default, Clone, Deserialize)]
// NOTE: NOT `deny_unknown_fields` — lenient per third-party boundary
#[serde(rename_all = "kebab-case")]
pub struct EntryFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub when_to_use: Option<String>,  // note: snake_case in YAML per Claude Code convention
    #[serde(default, deserialize_with = "deserialize_arguments")]
    pub arguments: Vec<String>,
    pub argument_hint: Option<String>,
    pub disable_model_invocation: Option<bool>,
    pub user_invocable: Option<bool>,
    pub prompt_name: Option<String>,
}

/// Accepts EITHER a space-separated string OR a YAML list of strings per Claude Code docs.
/// Matches Claude Code's documented behavior verbatim.
fn deserialize_arguments<'de, D>(d: D) -> Result<Vec<String>, D::Error> where D: Deserializer<'de> { ... }
```

**Note**: `when_to_use` deserialises from YAML key `when_to_use` (snake_case), NOT `when-to-use`. This matches Claude Code's documented frontmatter naming and the Tome spec. All other recognised fields use kebab-case in YAML (`disable-model-invocation`, `user-invocable`, `argument-hint`, `prompt-name`). The `#[serde(rename_all = "kebab-case")]` handles those; `when_to_use` is the one exception per Claude Code's convention.

Default resolution helper:
```rust
impl EntryFrontmatter {
    pub fn resolved_searchable(&self) -> bool {
        !self.disable_model_invocation.unwrap_or(false)
    }
    pub fn resolved_user_invocable(&self, kind: EntryKind) -> bool {
        self.user_invocable.unwrap_or(match kind {
            EntryKind::Skill => false,
            EntryKind::Command => true,
        })
    }
}
```

---

## 3. Substitution layer (the new `src/substitution/` module)

### 3.1 Public API surface

```rust
// src/substitution/mod.rs
pub use context::SubstitutionContext;
pub use context::SubstitutionContextBuilder;

/// Render an entry body through the four-stage substitution pipeline.
/// Returns the rendered body or a SubstitutionError on failure.
pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError>;

#[derive(Debug)]
pub enum SubstitutionError {
    PluginDataDirCreationFailed { path: PathBuf, source: std::io::Error },
    WorkspaceDataDirCreationFailed { path: PathBuf, source: std::io::Error },
    InvalidArgumentFrontmatter { reason: String, file: PathBuf },
    PromptArgumentMismatch { expected: usize, supplied: usize },
}
```

Each `SubstitutionError` variant maps to a closed-enum `TomeError` variant via `From` (see §6).

### 3.2 `SubstitutionContext`

```rust
// src/substitution/context.rs
pub struct SubstitutionContext {
    // Built-in values (R-9 paths anchored under <home>/.tome/)
    pub catalog_name: String,
    pub plugin_name: String,
    pub plugin_version: String,
    pub entry_name: String,
    pub entry_path: PathBuf,         // absolute
    pub entry_dir: PathBuf,          // absolute (parent of entry_path)
    pub plugin_root_dir: PathBuf,    // absolute
    pub plugin_data_dir: PathBuf,    // absolute, lazy-created
    pub workspace_name: String,
    pub workspace_data_dir: PathBuf, // absolute, lazy-created
    pub clock: time::OffsetDateTime, // injected per R-16

    // Argument values
    pub args: Option<ArgumentValues>,

    // Lazy-init handles for data dir creation
    pub paths: Paths,  // existing Paths struct; for plugin_data_dir / workspace_data_dir resolution
}

pub enum ArgumentValues {
    /// Single string from caller; named-vs-positional disambiguation per R-10.
    Single(String),
    /// Structured object from caller; named values keyed, positional derived in declaration order.
    Object {
        named: HashMap<String, String>,
        declared_order: Vec<String>,
    },
}

impl SubstitutionContext {
    pub fn builder() -> SubstitutionContextBuilder { ... }
}
```

`SubstitutionContextBuilder` is the public construction surface; required fields fail at `.build()` if absent (returns `Result<SubstitutionContext, SubstitutionError>`).

### 3.3 Internal stage shape

```rust
// src/substitution/builtins.rs
pub(super) fn apply_builtins(body: &str, ctx: &SubstitutionContext) -> Result<String, SubstitutionError>;

// src/substitution/env.rs
pub(super) fn apply_env(body: &str) -> Cow<'_, str>;

// src/substitution/arguments.rs
pub(super) fn apply_arguments(body: &str, args: &ArgumentValues, declared: &[String]) -> (String, /* any_replaced */ bool);

// src/substitution/mod.rs internal:
pub fn render(...) -> Result<String, ...> {
    let s = apply_builtins(body, ctx)?;
    let s = apply_env(&s).into_owned();
    let (s, replaced) = match &ctx.args {
        Some(args) => apply_arguments(&s, args, &ctx.declared_args),
        None => (s, false),
    };
    let s = if ctx.args.is_some() && !replaced {
        format!("{s}\n\nARGUMENTS: {value}", value = args.append_value())
    } else {
        s
    };
    Ok(s)
}
```

### 3.4 Test injection seams (R-16)

```rust
// Tests-only, but pub-via-doc-hidden so integration tests can reach:
#[doc(hidden)]
pub static SUBSTITUTION_CLOCK_OVERRIDE: OnceLock<Mutex<Option<time::OffsetDateTime>>> = OnceLock::new();

#[doc(hidden)]
pub static PLUGIN_DATA_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

#[doc(hidden)]
pub static WORKSPACE_DATA_DIR_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
```

Each comes with an RAII guard (`ClockOverrideGuard`, `PluginDataDirGuard`, `WorkspaceDataDirGuard`) in `tests/common/mod.rs` that installs on `new()` + clears on `Drop`.

---

## 4. MCP types (extending `src/mcp/`)

### 4.1 Updated tool: `search_skills`

Input (extended):
```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchSkillsInput {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    pub catalog: Option<String>,
    pub plugin: Option<String>,
    #[serde(default = "default_description_max_chars")]
    pub description_max_chars: u32,  // NEW; default 150
}

fn default_description_max_chars() -> u32 { 150 }
```

Output element (extended):
```rust
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,  // NEW
    pub description: String,  // NOW: truncated to description_max_chars
    pub path: PathBuf,
    pub score: f32,
}
```

Searchable filter applied in the DB query: `WHERE searchable = 1 AND enabled = 1`.

### 4.2 New tool: `get_skill_info`

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSkillInfoInput {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: EntryKind,  // default Skill
}

fn default_kind() -> EntryKind { EntryKind::Skill }

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillInfo {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    pub path: PathBuf,
    pub description: String,  // FULL, untruncated
    pub when_to_use: Option<String>,
    pub plugin_version: String,
    pub user_invocable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceEnumeration>,  // None for command-kind
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResourceEnumeration {
    pub files: Vec<String>,  // absolute paths; may end with "and N more" sentinel
    pub directories: BTreeMap<String, Vec<String>>,  // BTreeMap for deterministic alphabetical key order
}

const PER_DIRECTORY_CAP: usize = 5;
const SENTINEL_PREFIX: &str = "and ";
```

### 4.3 Updated tool: `get_skill`

Input (extended):
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSkillInput {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: EntryKind,
    pub args: Option<GetSkillArgs>,  // NEW
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum GetSkillArgs {
    Single(String),
    Object(HashMap<String, String>),
}
```

Output:
```rust
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetSkillResponse {
    pub content: String,  // rendered through substitution layer per FR-101
    pub path: PathBuf,
}
```

### 4.4 MCP `prompts` capability

```rust
// src/mcp/prompts.rs

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptDescriptor {
    pub name: String,  // sanitised + truncated; collision-resolved
    pub description: String,  // entry's description (possibly truncated for prompt-list display)
    pub arguments: Vec<PromptArgument>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptListResponse {
    pub prompts: Vec<PromptDescriptor>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptGetResponse {
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptMessage {
    pub role: String,  // "user"
    pub content: PromptContent,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptContent {
    #[serde(rename = "type")]
    pub content_type: String,  // "text"
    pub text: String,  // the rendered body
}
```

### 4.5 Prompt name derivation

```rust
// src/mcp/prompt_name.rs

const PLUGIN_PORTION_MAX: usize = 16;
const ENTRY_PORTION_MAX: usize = 32;
const SEPARATOR: &str = "__";

pub fn derive_name(entry: &EntryRow, prompt_name_override: Option<&str>) -> String {
    let raw = match prompt_name_override {
        Some(override_) => sanitise(override_),
        None => format!("{}{}{}", sanitise_trunc(&entry.plugin, PLUGIN_PORTION_MAX), SEPARATOR, sanitise_trunc(&entry.name, ENTRY_PORTION_MAX)),
    };
    // No further truncation beyond per-portion caps; per NFR-003 the combined name fits under MCP budget.
    raw
}

fn sanitise(s: &str) -> String {
    s.to_lowercase()
     .chars()
     .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
     .collect::<String>()
     // Collapse runs of '_'
     .replace("__", "_")
     // ... iterate until idempotent (typically one pass)
}

fn sanitise_trunc(s: &str, max: usize) -> String {
    let sanitised = sanitise(s);
    if sanitised.len() <= max { sanitised } else { sanitised[..max].to_string() }
}
```

### 4.6 Prompt collision handling

```rust
// src/mcp/prompt_collision.rs

#[derive(Debug, Clone, Serialize)]
pub struct CollisionRecord {
    pub generated_name: String,
    pub entries: Vec<EntryIdentity>,
    pub final_names: Vec<String>,  // one per entry; first is unsuffixed, rest counter-suffixed
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryIdentity {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    pub kind: EntryKind,
    pub indexed_at: time::OffsetDateTime,
}

/// Resolve collisions deterministically per FR-062:
/// - Sort by (indexed_at ASC, catalog ASC, plugin ASC, kind ASC, name ASC).
/// - First entry gets unsuffixed name; subsequent get name + counter starting at 2.
/// - Each collision logged at warn! level with CollisionRecord.
pub fn resolve_collisions(entries: &[EntryRow]) -> (Vec<PromptDescriptor>, Vec<CollisionRecord>);
```

### 4.7 `McpState` extension

```rust
// src/mcp/state.rs
pub struct McpState {
    // ... existing fields ...
    pub prompt_registry: Arc<PromptRegistry>,  // NEW
}

pub struct PromptRegistry {
    pub by_name: HashMap<String, EntryRow>,  // resolved-name → entry
    pub collisions: Vec<CollisionRecord>,    // for doctor's collision report
}
```

Populated at MCP startup from the active workspace's enabled-entry set; immutable for the session (per NFR-008 — workspace switches require server restart).

---

## 5. Doctor extension types

```rust
// src/doctor/report.rs — Phase 5 additions

/// Phase 5 doctor surface — added to existing DoctorReport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsReport {
    pub prompts: Vec<PromptDescriptor>,   // grouped-by-plugin downstream; flat for serialisation
    pub collisions: Vec<CollisionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanDataDirReport {
    pub plugin_data: Vec<PathBuf>,        // orphaned per-plugin dirs
    pub workspace_data: Vec<PathBuf>,     // orphaned per-workspace-per-plugin dirs
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryCountsByKind {
    pub skills: u32,
    pub commands: u32,
    pub pending_re_embedding: u32,
}

// Extension to DoctorReport (existing):
pub struct DoctorReport {
    // ... existing fields ...
    pub prompts: Option<PromptsReport>,           // Phase 5 NEW
    pub orphan_data_dirs: Option<OrphanDataDirReport>,  // Phase 5 NEW
    pub entry_counts: Option<EntryCountsByKind>,  // Phase 5 NEW
}
```

Orphan detection: walk `<home>/.tome/plugin-data/*/` and `<home>/.tome/workspaces/*/plugin-data/*/`, check each against current workspace_skills + workspace_catalogs membership. Informational only in Phase 5 (no `--fix` repair handler).

Doctor's Phase 5 surface is read-only by default (FR-124). The new report sections do NOT trigger lazy directory creation; they enumerate existing paths only.

---

## 6. Error types and exit codes

New variants for `TomeError` (extending the closed enum in `src/error.rs`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum TomeError {
    // ... existing variants ...

    // Phase 5 additions:

    #[error("entry not found: {catalog}/{plugin}/{name} (kind: {kind:?})")]
    EntryNotFound { catalog: String, plugin: String, name: String, kind: EntryKind },

    #[error("substitution failed: {reason}")]
    SubstitutionFailed { reason: String },

    #[error("invalid argument frontmatter in {file}: {reason}")]
    InvalidArgumentFrontmatter { file: PathBuf, reason: String },

    #[error("prompt argument mismatch: expected {expected}, supplied {supplied}")]
    PromptArgumentMismatch { expected: usize, supplied: usize },

    #[error("workspace data directory write failed at {path}: {source}")]
    WorkspaceDataDirWriteFailed { path: PathBuf, source: std::io::Error },

    /// Plugin data dir create_dir_all failure. Split from
    /// `WorkspaceDataDirWriteFailed` in US1.d (R-M1) so the variant
    /// name + exit code disambiguate the directory class instead of
    /// burying it in the inner path.
    #[error("plugin data directory write failed at {path}: {source}")]
    PluginDataDirWriteFailed { path: PathBuf, source: std::io::Error },
}
```

Exit code mapping:

| Variant | Exit code | Notes |
|---|---|---|
| `PluginDataDirWriteFailed` | 9 | US1.d reviewer pass (R-M1) split this from `WorkspaceDataDirWriteFailed` (25). Mirrors the substitution engine's matching split (`SubstitutionError::PluginDataDirCreationFailed` vs `WorkspaceDataDirCreationFailed`). Code 9 is the lowest free slot in Phase 1's I/O cluster (1–8). |
| `WorkspaceDataDirWriteFailed` | 25 | New; covers workspace-data-dir `create_dir_all` failures only after the R-M1 split. |
| `PromptArgumentMismatch` | 26 | New; assigned 26 (NOT 24) because Phase 4 already ships `SummariserFailure → 24`. Final assignment per `contracts/exit-codes-p5.md`. |
| `EntryNotFound` | 27 | New; reassigned from contract-proposed 21 (Phase 2 ships `PluginAlreadyInState → 21`). See `contracts/exit-codes-p5.md` § Reassigned slots. |
| `SubstitutionFailed` | 28 | New; reassigned from contract-proposed 22 (Phase 2 ships `PluginManifestParseError → 22`). See `contracts/exit-codes-p5.md` § Reassigned slots. |
| `InvalidArgumentFrontmatter` | 29 | New; reassigned from contract-proposed 23 (Phase 2 ships `SkillFrontmatterParseError → 23`). See `contracts/exit-codes-p5.md` § Reassigned slots. |

**Note**: Phase 4 ships `SummariserFailure → 24`. The PRD's pre-allocation of exit code 24 for Phase 5's `PromptArgumentMismatch` is superseded by `contracts/exit-codes-p5.md` reassigning to **26**, preserving Phase 4's already-shipped semantics per constitution principle II (Predictable Exit Codes — NON-NEGOTIABLE). The same principle drove the F1-time reassignment of codes 21/22/23 → 27/28/29 to dodge Phase 2's plugin lifecycle cluster (20–23). Phase 5 occupies a clean contiguous cluster at **25–29** after these amendments.

---

## 7. Cross-module type ownership summary

| Type | Module | Visibility | Purpose |
|---|---|---|---|
| `EntryKind` enum | `src/plugin/identity.rs` (or `src/index/mod.rs`) | `pub` | Discriminator across all Phase 5 surfaces |
| `EntryRow` struct | `src/index/skills.rs` | `pub` | Schema row representation |
| `EntryFrontmatter` struct | `src/plugin/frontmatter.rs` | `pub(crate)` extended | Lenient parser output |
| `SubstitutionContext` | `src/substitution/context.rs` | `pub` | Public API of substitution layer |
| `SubstitutionContextBuilder` | `src/substitution/context.rs` | `pub` | Builder |
| `SubstitutionError` | `src/substitution/mod.rs` | `pub` | Returned from `render()` |
| `ArgumentValues` enum | `src/substitution/context.rs` | `pub` | Argument input shape |
| `SearchSkillsInput` / `SearchResult` | `src/mcp/tools/search_skills.rs` | `pub` | Tool I/O |
| `GetSkillInfoInput` / `SkillInfo` / `ResourceEnumeration` | `src/mcp/tools/get_skill_info.rs` | `pub` | Tool I/O |
| `GetSkillInput` / `GetSkillResponse` | `src/mcp/tools/get_skill.rs` | `pub` extended | Tool I/O |
| `PromptDescriptor` / `PromptArgument` | `src/mcp/prompts.rs` | `pub` | Prompts capability |
| `PromptListResponse` / `PromptGetResponse` / `PromptMessage` / `PromptContent` | `src/mcp/prompts.rs` | `pub` | Prompts capability response shapes |
| `CollisionRecord` / `EntryIdentity` | `src/mcp/prompt_collision.rs` | `pub` | Diagnostic record |
| `PromptRegistry` | `src/mcp/state.rs` | `pub` | Session-wide name registry |
| `PromptsReport` / `OrphanDataDirReport` / `EntryCountsByKind` | `src/doctor/report.rs` | `pub` | Doctor extensions |
| `TomeError` (Phase 5 variants) | `src/error.rs` | `pub` | Closed enum extension |
