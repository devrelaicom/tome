# Phase 1 — Data Model

This document describes the in-memory and on-disk data shapes for Tome Phase 1. Types are presented as Rust signatures because that's how the implementation will materialise them; the *shapes* are what matter — the field names, types, validation rules, and the on-disk serialisation are part of the spec.

The schema for the user-facing catalog manifest is also published as a TOML file at [`contracts/catalog-manifest.schema.toml`](./contracts/catalog-manifest.schema.toml).

---

## Entities

### 1. `CatalogEntry` — a registered catalog

The unit of registration. One entry per catalog in `config.toml`.

```rust
// src/config.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    /// The catalog's display name. Defaults to the manifest's `name` field;
    /// can be overridden at `tome catalog add` time with `--name`.
    pub name: String,
    /// The Git source the catalog was added with — e.g.
    /// `https://github.com/owner/repo`, `git@github.com:owner/repo`,
    /// or `file:///abs/path`.
    pub url: String,
    /// The ref tracked by this catalog. `"main"` by default; can be any
    /// branch, tag, or full/short SHA.
    pub ref_: String, // serialised as `ref` (see #[serde(rename)])
    /// Absolute path to the local cache directory for this catalog.
    /// Always inside the XDG data dir for Tome.
    pub path: std::path::PathBuf,
    /// ISO-8601 UTC timestamp of the most recent successful sync.
    pub last_synced: chrono::DateTime<chrono::Utc>, // RFC 3339 in TOML
}
```

On-disk TOML representation, inside `${XDG_CONFIG_HOME}/tome/config.toml`:

```toml
[catalogs.midnight-experts]
url = "https://github.com/midnight/midnight-experts"
ref = "main"
path = "/Users/alice/.local/share/tome/catalogs/a3f9c1b2…"
last_synced = "2026-05-11T14:23:00Z"
```

**Invariants**:
- `name` is the TOML table key under `[catalogs.<name>]`, used as the registry primary key. It is unique within a registry; duplicate registration fails with `CatalogAlreadyExists` (exit code 4).
- `path` is always the canonical absolute path; never `~`-prefixed.
- `url` is preserved verbatim from the `add` call (after shorthand expansion); never normalised in a way that loses information.

**Note on the `chrono` dependency**: this introduces one additional crate not listed in STACK.md. The alternative is to use `time` (also good) or to roll our own RFC 3339 formatter on top of `std::time::SystemTime` (cheap, ~30 lines). Decision deferred to `/sdd:tasks` — both options stay within the binary-size budget; `time` is currently preferred for being smaller and free of the legacy `chrono` API surface.

---

### 2. `Config` — the top-level config document

```rust
// src/config.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Registered catalogs, keyed by display name.
    #[serde(default)]
    pub catalogs: std::collections::BTreeMap<String, CatalogEntry>,
}
```

**Invariants**:
- An empty config is valid (no catalogs registered).
- Unknown top-level keys are rejected (FR-016).
- The map is a `BTreeMap` so the `list` command's default ordering is deterministic.

---

### 3. `CatalogManifest` — the `tome-catalog.toml` schema

The user-facing manifest at the root of every catalog repository.

```rust
// src/catalog/manifest.rs
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogManifest {
    pub name: String,
    pub description: String,
    pub version: String, // semver string — validated at parse time

    pub owner: Owner,

    #[serde(default)]
    pub plugins: Vec<PluginDeclaration>,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginDeclaration {
    pub name: String,
    pub source: String, // a relative path within the catalog repo — validated below
}
```

**Validation, in this order, performed by `CatalogManifest::parse_and_validate(path: &Path, bytes: &[u8])`**:

1. **TOML parse** — `toml::from_str` with `deny_unknown_fields`. Any unknown key fails with `ManifestInvalid::UnknownField { file, key, expected_schema_uri }` (exit code 5).
2. **Required fields** — `name`, `description`, `version`, `owner.name`, `owner.email` non-empty (TOML enforces presence via `deny_unknown_fields` + `Deserialize` requiredness).
3. **`version` is valid semver** — `semver::Version::parse`. Failure → `ManifestInvalid::InvalidVersion`.
4. **`owner.email` is a syntactically plausible address** — a single `@`, non-empty local part, non-empty domain with at least one `.`. We do not validate against a deliverability service.
5. **`plugins[].name` is unique within the manifest** — duplicates → `ManifestInvalid::DuplicatePluginName`.
6. **`plugins[].source` is a valid relative path** — per the algorithm below.

**`source` path validation algorithm** (FR-012, FR-013):

```rust
fn validate_source(catalog_root: &Path, source: &str) -> Result<PathBuf, ManifestInvalid> {
    // 1. reject URL schemes (https://, file://, git@, etc.)
    if source.contains("://") || source.starts_with("git@") {
        return Err(ManifestInvalid::SourceLooksLikeUrl);
    }
    // 2. reject Windows-style drive prefixes
    if source.len() >= 2 && matches!(source.as_bytes()[1], b':') {
        return Err(ManifestInvalid::SourceAbsolute);
    }
    let p = Path::new(source);
    // 3. reject absolute paths
    if p.is_absolute() {
        return Err(ManifestInvalid::SourceAbsolute);
    }
    // 4. reject `..` components syntactically
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ManifestInvalid::SourceParentTraversal);
    }
    // 5. resolve symlinks. canonicalize() requires the path to exist —
    //    so we join + canonicalize + check ancestry.
    let joined = catalog_root.join(p);
    let resolved = joined.canonicalize()
        .map_err(ManifestInvalid::SourceUnresolvable)?;
    let root_resolved = catalog_root.canonicalize()
        .map_err(ManifestInvalid::CatalogRootUnresolvable)?;
    if !resolved.starts_with(&root_resolved) {
        return Err(ManifestInvalid::SourceEscapesRoot);
    }
    Ok(resolved)
}
```

**Invariants**:
- Every error variant names the offending field, the value, and the manifest file path (FR-023).
- All variants of `ManifestInvalid` map to exit code 5.

---

### 4. `TomeError` — closed error enum, source of truth for exit codes

```rust
// src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum TomeError {
    // 2 — usage error (handled by clap; this variant covers post-parse
    //     errors that clap didn't catch).
    #[error("invalid usage: {0}")]
    Usage(String),

    // 3 — catalog not found
    #[error("catalog `{0}` is not registered")]
    CatalogNotFound(String),

    // 4 — catalog already registered
    #[error("catalog `{0}` is already registered")]
    CatalogAlreadyExists(String),

    // 5 — manifest invalid (with sub-variants below)
    #[error("manifest invalid: {0}")]
    ManifestInvalid(#[from] ManifestInvalid),

    // 6 — git operation failed (carries the catalog name and the scrubbed
    //     stderr from the upstream git invocation)
    #[error("git failed for `{catalog}`: {detail}")]
    GitFailed { catalog: String, detail: String },

    // 7 — filesystem / I/O error
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    // 8 — interrupted by user (SIGINT)
    #[error("interrupted by user")]
    Interrupted,

    // 1 — last-resort internal error. NO `Other`/`Unknown` arms.
    //     This variant exists for genuine programmer-facing surprises
    //     (panics caught at top level, etc.). Every named failure above
    //     MUST use its named arm, never this one.
    #[error("internal error: {0:#}")]
    Internal(anyhow::Error),
}

impl TomeError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Internal(_)              => 1,
            Self::Usage(_)                 => 2,
            Self::CatalogNotFound(_)       => 3,
            Self::CatalogAlreadyExists(_)  => 4,
            Self::ManifestInvalid(_)       => 5,
            Self::GitFailed { .. }         => 6,
            Self::Io(_)                    => 7,
            Self::Interrupted              => 8,
        }
    }
}
```

**Closed-set guarantee**: `TomeError` has no `Other`/`Unknown` arm. Adding a new error category requires editing this enum, which requires editing `tests/exit_codes.rs` (every variant is exhaustively matched there to assert its code), which requires editing the spec's FR-022, which requires editing the PRD's exit-code table. The compiler enforces the chain.

**`ManifestInvalid` sub-enum**:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ManifestInvalid {
    #[error("unknown field `{key}` in {file}: see {expected_schema_uri}")]
    UnknownField { file: PathBuf, key: String, expected_schema_uri: String },

    #[error("missing required field `{key}` in {file}")]
    MissingField { file: PathBuf, key: String },

    #[error("`version` in {file} is not a valid semver: {got}")]
    InvalidVersion { file: PathBuf, got: String },

    #[error("`owner.email` in {file} is not a valid email: {got}")]
    InvalidEmail { file: PathBuf, got: String },

    #[error("duplicate plugin name `{name}` in {file}")]
    DuplicatePluginName { file: PathBuf, name: String },

    #[error("`plugins[].source = \"{value}\"` in {file} looks like a URL — Phase 1 supports relative paths only")]
    SourceLooksLikeUrl { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {file} is an absolute path — must be a relative path within the catalog repo")]
    SourceAbsolute { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {file} contains `..` — must be a normalised relative path")]
    SourceParentTraversal { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {file} resolves outside the catalog repo")]
    SourceEscapesRoot { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {file} does not exist or is unreachable: {cause}")]
    SourceUnresolvable { file: PathBuf, value: String, cause: std::io::Error },

    #[error("could not canonicalise catalog root {root}: {cause}")]
    CatalogRootUnresolvable { root: PathBuf, cause: std::io::Error },

    #[error("toml parse error in {file}: {message}")]
    TomlParse { file: PathBuf, message: String },
}
```

---

### 5. `Paths` — XDG-aware path resolver

```rust
// src/paths.rs
pub struct Paths {
    pub config_dir: PathBuf,   // ${XDG_CONFIG_HOME}/tome
    pub config_file: PathBuf,  // ${XDG_CONFIG_HOME}/tome/config.toml
    pub data_dir: PathBuf,     // ${XDG_DATA_HOME}/tome
    pub catalogs_dir: PathBuf, // ${XDG_DATA_HOME}/tome/catalogs
}

impl Paths {
    pub fn resolve() -> Result<Self, TomeError> { /* uses `directories` */ }
    pub fn cache_dir_for(&self, url: &str) -> PathBuf {
        // sha256(url) hex, in catalogs_dir
        let mut h = sha2::Sha256::new();
        h.update(url.as_bytes());
        self.catalogs_dir.join(hex::encode(h.finalize()))
    }
}
```

**Invariants**:
- `cache_dir_for(url)` is deterministic; two registrations with the same URL produce the same cache path. This is the basis for the FR-015 collision-avoidance guarantee.

---

### 6. Output records — the `--json` schema

When `--json` is set, every command emits one or more JSON records on stdout (success) or stderr (errors). The schemas are stable:

```jsonc
// catalog list — one record per registered catalog
{
  "name": "midnight-experts",
  "url": "https://github.com/midnight/midnight-experts",
  "ref": "main",
  "plugin_count": 2,
  "last_synced": "2026-05-11T14:23:00Z"
}

// catalog show — a single record
{
  "name": "midnight-experts",
  "description": "Expert plugins for working with the Midnight privacy chain",
  "version": "0.1.0",
  "owner": { "name": "Midnight Labs", "email": "plugins@midnight.network" },
  "plugins": [
    { "name": "midnight-compact-expert", "source": "./plugins/midnight-compact-expert" }
  ]
}

// any error — stderr
{
  "error": {
    "category": "manifest_invalid",   // matches FR-022 categories
    "exit_code": 5,
    "message": "`plugins[].source = \"../etc/passwd\"` in /tmp/cat/tome-catalog.toml contains `..` — must be a normalised relative path",
    "context": {                        // optional, category-dependent
      "file": "/tmp/cat/tome-catalog.toml",
      "field": "plugins[0].source",
      "value": "../etc/passwd"
    }
  }
}
```

The `category` field uses snake_case identifiers that map 1:1 to `TomeError` variants. Adding a new category requires updating the spec, the enum, and the contract document for the command (see `contracts/`).

---

## State transitions

### Registry lifecycle

```text
(no entry)
   │
   │ tome catalog add → clone → parse manifest → atomic write of config.toml
   ▼
Registered(name, url, ref, last_synced=now)
   │
   │ tome catalog update → fetch → reset --hard → re-parse → atomic write
   ▼
Registered(name, url, ref, last_synced=now')
   │
   │ tome catalog remove [--force] → atomic write removing entry → rm -rf cache
   ▼
(no entry)
```

**Failure transitions**:
- `add` fails → registry **unchanged**, partially cloned cache directory removed (FR-017a).
- `update` fails → that catalog's entry **unchanged**, partially fetched cache reverted to pre-fetch state via tempdir swap (FR-017a).
- `remove` fails after registry update → cache removal retries on next invocation (idempotent). Registry is the source of truth.
- SIGINT during any of the above → `TomeError::Interrupted` (exit code 8); registry and per-catalog cache are atomic (FR-026a).

### `--ref` semantics

| `--ref` value | Type | Update behaviour |
|---|---|---|
| (none) | tracking | `git fetch && git reset --hard origin/HEAD` — follows upstream default branch |
| `main`, `develop`, `feature/x` | branch tracking | `git fetch && git reset --hard origin/<ref>` |
| `v1.0.0`, any tag | tag pinning | `git fetch --tags && git reset --hard <ref>` |
| `[0-9a-f]{7,40}` | SHA pinning | `tome catalog update` no-ops with informational message and exits 0 (FR-008) |

SHA detection is structural: any value matching `^[0-9a-f]{7,40}$` is treated as a SHA. Tags with names matching that regex are pathological and would be mis-detected; documented as a known edge case.

---

## What's not in the data model (Phase 1)

These are explicit non-entities in Phase 1 — flagged so that future PRs don't quietly add them:

- **`Plugin` (installable)** — the manifest declares `PluginDeclaration`s, but the tool does not yet have a "plugin" concept beyond "a name + a path inside a catalog". The installable form arrives in Phase 2.
- **`Harness`** — no notion of which AI-coding-assistant is installed locally. Phase 2.
- **`Skill`, `Command`, `Agent`, `Hook`** — the components-of-a-plugin entities. Not parsed, not enumerated, not stored. Phase 2+.
- **`EmbeddingIndex`, `VectorRecord`** — semantic search. Phase 2.
- **`Session`, `Telemetry`** — none. Tome does not phone home.
