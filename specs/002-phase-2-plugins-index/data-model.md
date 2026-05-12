# Phase 2 — Data Model

This document describes the in-memory and on-disk data shapes added by Phase 2. Types are shown as Rust signatures because that is how the implementation materialises them; the shapes are what matter. The SQLite schema is also published as a separate, executable file at [`contracts/index-schema.sql`](./contracts/index-schema.sql).

---

## Persistent surfaces introduced in Phase 2

| Surface | Path | Owner | Strictness |
|---|---|---|---|
| Index database | `${XDG_DATA_HOME}/tome/index.db` | Tome | n/a (binary) |
| Index advisory lock | `${XDG_DATA_HOME}/tome/index.lock` | Tome | n/a (empty file) |
| Models root | `${XDG_DATA_HOME}/tome/models/` | Tome | n/a (directory) |
| Per-model manifest | `${XDG_DATA_HOME}/tome/models/<name>/manifest.json` | Tome | strict |
| Per-model ONNX artefacts | `${XDG_DATA_HOME}/tome/models/<name>/<files>` | Tome | content-addressed |
| Plugin manifest (read-only) | `${catalog cache}/<plugin>/.claude-plugin/plugin.json` | Third-party | lenient |
| Skill metadata header (read-only) | `${catalog cache}/<plugin>/skills/<skill>/SKILL.md` (frontmatter) | Third-party | lenient |

The Phase 1 surfaces (`config.toml`, the catalog registry, the catalog cache directories) are unchanged.

---

## Entities

### 1. `PluginId` — canonical plugin address

```rust
// src/plugin/identity.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PluginId {
    pub catalog: String,   // catalog registry key
    pub plugin: String,    // plugin directory name within the catalog
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.catalog, self.plugin)
    }
}

impl std::str::FromStr for PluginId {
    type Err = PluginIdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { /* "<catalog>/<plugin>"; rejects empty parts and embedded slashes */ }
}
```

**Invariants**:
- `catalog` matches a registered catalog name; lookup happens at command boundary.
- `plugin` matches a directory in that catalog's cache root that contains a `.claude-plugin/plugin.json`.
- No `..`, no absolute paths, no leading `.`.

---

### 2. `PluginRecord` — what `tome plugin list/show` returns (in-memory only)

Built on demand by walking the catalog cache. Not persisted in Phase 2; can be cached in a future phase if walks get expensive.

```rust
// src/plugin/mod.rs
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginRecord {
    pub id: PluginId,
    pub version: String,                 // from plugin.json#version, "0.0.0" if missing
    pub author: Option<String>,          // from plugin.json#author, joined "Name <email>"
    pub description: Option<String>,     // from plugin.json#description
    pub last_upstream_change: Option<time::OffsetDateTime>,   // `git log -1 --format=%cI -- <plugin-path>`
    pub status: PluginStatus,
    pub component_counts: ComponentCounts,
    pub last_indexed_at: Option<time::OffsetDateTime>,        // None if never indexed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginStatus {
    Enabled,
    Disabled,
    Unindexable,    // plugin.json missing or malformed
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ComponentCounts {
    pub skills: u32,
    pub agents: u32,
    pub commands: u32,
    pub hooks: u32,
    pub mcp_servers: u32,
}
```

---

### 3. `PluginManifest` — `plugin.json` schema (third-party; lenient)

We mirror only the fields we use. Everything else is ignored without warning per FR-013a.

```rust
// src/plugin/manifest.rs
#[derive(Debug, Clone, serde::Deserialize)]
// NOTE: NOT #[serde(deny_unknown_fields)] — this is third-party input.
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<PluginAuthor>,
    // Other fields (commands, hooks declarations, mcpServers, etc.) are ignored.
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginAuthor {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}
```

**Invariants**:
- `name` MUST be present and non-empty; absence is a parse failure (FR-013b → `PluginManifestParseError` exit code).
- All other fields are optional with sensible defaults.

---

### 4. `SkillFrontmatter` — YAML header inside `SKILL.md` (third-party; lenient)

```rust
// src/plugin/frontmatter.rs
#[derive(Debug, Clone, serde::Deserialize)]
// NOTE: NOT deny_unknown_fields — third-party input.
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: Option<String>,           // fallback in FR-011: skill directory name
    #[serde(default)]
    pub description: Option<String>,    // fallback in FR-012: first 500 chars of body
    // Other fields (when_to_use, allowed-tools, etc.) are ignored in Phase 2.
}
```

A malformed YAML header is a per-skill error (FR-013c): we skip that skill, warn, and continue with the rest of the plugin.

---

### 5. `SkillRecord` — the `skills` table row

```rust
// src/index/skills.rs
#[derive(Debug, Clone)]
pub struct SkillRecord {
    pub id: i64,                         // SQLite ROWID
    pub catalog: String,
    pub plugin: String,
    pub name: String,                    // canonical skill name (frontmatter `name` or directory fallback)
    pub description: String,             // the text that was embedded
    pub plugin_version: String,          // version of the plugin at index time
    pub path: String,                    // on-disk path to the SKILL.md
    pub content_hash: String,            // sha256(hex) of the embedded text composition
    pub enabled: bool,
    pub indexed_at: time::OffsetDateTime,
}
```

Identity: `UNIQUE(catalog, plugin, name)`. Plugin version is recorded, not part of identity (FR-013).

---

### 6. `SkillEmbedding` — the `skill_embeddings` virtual-table row

```rust
// src/index/vec_ext.rs
#[derive(Debug, Clone)]
pub struct SkillEmbedding {
    pub skill_id: i64,                   // foreign key to skills.id
    pub embedding: Vec<f32>,             // length 384
}
```

The vector is stored in a `sqlite-vec` virtual table; we read/write through SQL using the extension's KNN syntax.

---

### 7. `ModelManifest` — `models/<name>/manifest.json` (Tome-owned; strict)

```rust
// src/embedding/registry.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub name: String,
    pub version: String,
    pub kind: ModelKind,
    pub source_url: String,
    pub sha256: String,                  // lowercase hex, 64 chars
    pub size_bytes: u64,
    pub licence: String,                 // e.g. "MIT"
    pub files: Vec<String>,              // relative paths inside the model directory
    pub installed_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ModelKind {
    Embedder,
    Reranker,
}
```

On-disk JSON example:

```json
{
  "name": "bge-small-en-v1.5",
  "version": "1.5",
  "kind": "embedder",
  "source_url": "https://huggingface.co/qdrant/bge-small-en-v1.5-onnx-Q/resolve/main/model_quantized.onnx",
  "sha256": "<64-hex>",
  "size_bytes": 47185920,
  "licence": "MIT",
  "files": ["model.onnx", "tokenizer.json"],
  "installed_at": "2026-05-12T11:34:00Z"
}
```

**Invariants**:
- `sha256` is the digest of the primary artefact file (model.onnx). Tokeniser files are content-addressed via their own pinned upstream — verified at install time, recorded by listing rather than by separate hash. (Decision in research R5.)
- Unknown fields cause parse failure (Tome-owned input).

---

### 8. `IndexMeta` — `meta` table rows (Tome-owned; strict at read time)

The `meta` table is a key/value store with a closed set of valid keys.

```rust
// src/index/meta.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKey {
    SchemaVersion,
    EmbedderName,
    EmbedderVersion,
    RerankerName,
    RerankerVersion,
    CreatedAt,
    LastWriterPid,           // optional; informational for `tome status`
}
```

Unknown keys observed on read are logged as warnings (forward-compat) but never written. Reads expect a known set; missing required keys (`SchemaVersion`) are a `IndexCorrupt` failure.

---

### 9. `ContentHash` — the diff-detection primitive

```rust
// src/index/skills.rs
pub fn content_hash(name: &str, description: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\n\n");
    hasher.update(description.as_bytes());
    hex::encode(hasher.finalize())
}
```

This is exactly the embedding-text composition (research R8), hashed. By construction, two skills with the same `(name, description)` produce the same hash, which is the condition under which FR-006 / FR-032 perform a no-op re-enable / no-op refresh.

---

### 10. `QueryResult` — output of `tome query`

```rust
// src/index/query.rs
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryResult {
    pub catalog: String,
    pub plugin: String,
    pub skill: String,
    pub plugin_version: String,
    pub score: f32,
    pub path: String,
    pub scoring: ScoringStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScoringStage {
    Reranked,
    EmbeddingSimilarity,
}
```

The JSON output of `tome query --json` is `{"results": [QueryResult ...], "scoring": "reranked", "threshold_passed": true}`. Threshold_passed reflects `--strict` mode; without `--strict` it is always `true`.

---

### 11. `StatusReport` — output of `tome status`

```rust
// src/commands/status.rs
#[derive(Debug, serde::Serialize)]
pub struct StatusReport {
    pub tome_version: String,
    pub embedder: ModelStatus,
    pub reranker: ModelStatus,
    pub index_db: IndexDbStatus,
    pub schema_version: u32,
    pub drift: DriftStatus,
    pub overall: OverallHealth,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelStatus {
    pub name: String,
    pub version: String,
    pub state: ModelState,        // Ok / Missing / Corrupt / ChecksumMismatch
}

#[derive(Debug, serde::Serialize)]
pub struct IndexDbStatus {
    pub state: IndexState,        // Ok / Missing / Corrupt / SchemaTooNew / Locked-by-other-pid
    pub size_bytes: u64,
    pub skill_count: u64,
    pub enabled_plugin_count: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DriftStatus {
    None,
    EmbedderNameDrift { stored: String, configured: String },
    EmbedderVersionDrift { stored: String, configured: String },
    RerankerDrift { stored: String, configured: String },     // name or version
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OverallHealth { Ok, Degraded, Unhealthy }
```

`Degraded` covers reranker-only drift (queries still serve); `Unhealthy` covers anything that prevents queries.

---

## SQLite schema (Phase 2 v1)

The canonical schema lives at [`contracts/index-schema.sql`](./contracts/index-schema.sql); summarised here for review:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE skills (
  id              INTEGER PRIMARY KEY,
  catalog         TEXT NOT NULL,
  plugin          TEXT NOT NULL,
  name            TEXT NOT NULL,
  description     TEXT NOT NULL,
  plugin_version  TEXT NOT NULL,
  path            TEXT NOT NULL,
  content_hash    TEXT NOT NULL,
  enabled         INTEGER NOT NULL DEFAULT 1,
  indexed_at      TEXT NOT NULL,                      -- RFC 3339
  UNIQUE (catalog, plugin, name)
) STRICT;

CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin);
CREATE INDEX idx_skills_enabled ON skills(enabled);
CREATE INDEX idx_skills_content_hash ON skills(content_hash);

CREATE VIRTUAL TABLE skill_embeddings USING vec0(
  skill_id   INTEGER PRIMARY KEY,
  embedding  FLOAT[384]
);
```

`STRICT` mode on the `skills` table enforces column types at insert time — defence in depth against insert paths that bypass the Rust type system.

---

## Lifecycle state machine — a single plugin

```text
                     +-------------------------+
   register catalog  |     UNTRACKED           |  (catalog not registered)
                     +-------------------------+
                                 |
                                 v
                     +-------------------------+
                     |     INSTALLED           |  (plugin files exist on disk, no skill rows)
                     +-------------------------+
                            |             ^
            plugin enable   |             | plugin disable
                            v             |
                     +-------------------------+
                     |     ENABLED             |  (skill rows present, enabled=1)
                     +-------------------------+
                            |             |
                catalog     |             | catalog update detects upstream removal
                refresh     |             v
                detects     |   +-------------------------+
                content     |   |  ORPHANED → DISABLED    |  (skill rows dropped, plugin row absent)
                change      |   +-------------------------+
                            v
                     +-------------------------+
                     | ENABLED (rev N+1)       |  (one or more skill rows re-embedded)
                     +-------------------------+
```

Transitions:

| From | To | Trigger | Index effect |
|---|---|---|---|
| UNTRACKED | INSTALLED | `catalog add` | none |
| INSTALLED | ENABLED | `plugin enable` | embed every skill, insert rows, `enabled=1` |
| ENABLED | ENABLED (rev N+1) | `catalog update` (changes) | re-embed changed skills only |
| ENABLED | DISABLED | `plugin disable` | `enabled=0` for plugin's rows |
| DISABLED | ENABLED | `plugin enable` (unchanged content) | `enabled=1` flip; no embedding |
| DISABLED | ENABLED | `plugin enable` (content changed) | re-embed changed skills, then `enabled=1` |
| ENABLED | DISABLED + rows dropped | `catalog update` (plugin removed upstream) | drop rows, log warning |
| ENABLED | DISABLED + rows dropped | `catalog remove --force` | drop rows, drop plugin, drop catalog |

The state machine is implemented in `src/plugin/lifecycle.rs`; each transition runs inside one SQLite transaction inside the advisory lockfile boundary.
