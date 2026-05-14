# Paths and On-Disk Layout — Phase 4

**Spec source**: [spec.md FR-300 through FR-305](../spec.md)
**Research**: [research.md R-1, R-11](../research.md)

## Root

A single per-user root directory at `<home>/.tome/`. `<home>` is resolved via `std::env::home_dir()` (Rust 1.85+ un-deprecated). No XDG-style separation between config / data / cache / state.

## Layout

```text
<home>/.tome/
├── config.toml              # Tome's global config (Phase 1 schema; strict)
├── settings.toml            # Global harness settings (NEW in Phase 4; strict)
├── index.db                 # Single central SQLite database (schema v2)
├── index.lock               # Advisory lockfile (Phase 2 discipline)
├── catalogs/                # Shared catalog clones (Phase 1 layout)
│   └── <sha256-of-url>/
├── models/                  # Shared model artefacts
│   ├── bge-small-en-v1.5/
│   │   ├── model.onnx
│   │   ├── tokenizer.json
│   │   └── manifest.json
│   ├── bge-reranker-base/
│   │   ├── model.onnx
│   │   ├── tokenizer.json
│   │   └── manifest.json
│   └── qwen2.5-0.5b-instruct/  # NEW
│       ├── model.gguf
│       └── manifest.json
├── logs/
│   ├── mcp.log
│   └── mcp.log.1            # rotated previous log (size cap, Phase 3 policy)
└── workspaces/
    ├── global/              # Seeded on first bootstrap; cannot be removed
    │   ├── settings.toml
    │   └── RULES.md
    └── <user-workspace>/
        ├── settings.toml
        └── RULES.md
```

## Atomicity

Every file write under `<root>/` follows the Phase 1 atomic-write discipline: write to a sibling temp file on the same filesystem, fsync, atomic rename. On Unix, files containing developer-relevant configuration (`config.toml`, `settings.toml`, `manifest.json`) are created with mode 0600.

Multi-file directory landings (`<root>/workspaces/<name>/`, `<project>/.tome/`) use the `src/util/atomic_dir.rs::land_directory` helper documented in [research.md R-10](../research.md). The helper stages the directory in a sibling temp dir on the same filesystem and renames once.

## What's removed vs Phase 3

| Phase 3 path | Phase 4 disposition |
|--------------|---------------------|
| `${XDG_CONFIG_HOME}/tome/config.toml` | Moved to `<root>/config.toml` |
| `${XDG_DATA_HOME}/tome/index.db` | Moved to `<root>/index.db` |
| `${XDG_DATA_HOME}/tome/catalogs/` | Moved to `<root>/catalogs/` |
| `${XDG_DATA_HOME}/tome/models/` | Moved to `<root>/models/` |
| `${XDG_STATE_HOME}/tome/mcp.log` | Moved to `<root>/logs/mcp.log` |
| `${XDG_STATE_HOME}/tome/workspaces.txt` | Deleted; replaced by `workspace_catalogs` + `workspace_projects` tables |
| `<workspace-root>/.tome/index.db` | Deleted; consolidated into `<root>/index.db` |
| `<workspace-root>/.tome/config.toml` | Deleted in this exact shape; replaced by `<root>/workspaces/<name>/settings.toml` + a thin `<project>/.tome/config.toml` binding pointer |
| `<workspace-root>/.tome/index.lock` | Deleted; one global lockfile at `<root>/index.lock` |

**No backward-compat detection.** Phase 3 paths are not probed, recognised, or copied. Pre-release users wipe their old `~/.local/share/tome/` / `~/.config/tome/` / `~/.local/state/tome/` directories manually.

## Permissions

| Path | Unix mode | Created by |
|------|-----------|------------|
| `<root>/` | 0700 | First-run bootstrap |
| `<root>/config.toml` | 0600 | First-run bootstrap; `tome` config writes |
| `<root>/settings.toml` | 0600 | `tome harness use --scope global` |
| `<root>/index.db` | 0600 | First-run bootstrap; rusqlite writes |
| `<root>/index.lock` | 0600 | Advisory lock acquisition |
| `<root>/catalogs/<hash>/` | 0700 + Git's defaults | Catalog clone |
| `<root>/models/<name>/` | 0755 (default umask) | Model download |
| `<root>/logs/` | 0700 | MCP server first-write |
| `<root>/logs/mcp.log` | 0600 | MCP server log open |
| `<root>/workspaces/<name>/` | 0700 | `tome workspace init` |
| `<root>/workspaces/<name>/settings.toml` | 0600 | `tome workspace init` / subsequent writes |
| `<root>/workspaces/<name>/RULES.md` | 0644 | Summary regeneration |
| `<project>/.tome/` | 0700 | `tome workspace use` |
| `<project>/.tome/config.toml` | 0600 | `tome workspace use` |
| `<project>/.tome/RULES.md` | 0644 | `tome workspace use` / sync |

## `Paths` accessor module

All path joins happen exclusively in `src/paths.rs`. No other module constructs Tome-owned paths from string literals. See [data-model.md §1](../data-model.md) for the typed accessor signatures.
