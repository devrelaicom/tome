# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Git

| Service | Type | Purpose | Configuration Location |
|---------|------|---------|------------------------|
| System `git` (shell-out) | VCS | Cloning, fetching, checking out, and resetting remote catalog repositories | System PATH; referenced in `src/catalog/git.rs` |

### Connection Details

- **Method**: Synchronous shell-outs via `std::process::Command`
- **Operations**: 
  - `git clone <url>` — fetch catalog source
  - `git fetch` — refresh cached catalogs
  - `git checkout <ref>` — pin to specific commit
  - `git reset --hard` — discard local changes
- **Signal handling**: SIGINT (Ctrl+C) kills in-flight child processes; sets exit code 8
- **Credential scrubbing**: All captured stderr/stdout passes through `scrub_credentials()` before logging or error display (4 regex patterns in `src/catalog/git.rs` lines 50-72):
  - URL login patterns (`https://user@host`)
  - SSH login patterns (`git@host:`)
  - Key-value secrets (`token=`, `password=`, `api_key=`, etc.)
  - Long hex strings (40+ chars) in unsafe context
- **Error detail**: Captured `git` stderr included in `TomeError::GitFailed.detail` after scrubbing

### Configuration

No explicit Git config file is used. `git` inherits system configuration (`~/.gitconfig`, `/etc/gitconfig`).

### Failure Modes

- `git` not found in PATH: `TomeError::GitNotFound` (exit code 4)
- Network failure / SSH key unavailable: `TomeError::GitFailed` with scrubbed stderr (exit code 3)
- Invalid Git repository: `TomeError::GitFailed` (exit code 3)
- Interrupted by Ctrl+C: `TomeError::Interrupted` (exit code 8)

---

## Future Integrations (Phase 2+)

The following are planned but **not implemented in Phase 1**:

| Service | Type | Planned Purpose |
|---------|------|-----------------|
| SQLite | Local DB | Catalog index and plugin metadata |
| `sqlite-vec` | Vector store | Embedding-based search (optional Phase 2) |
| `fastembed-rs` | Embedding model | Local, CPU-based embeddings (optional Phase 2) |
| HTTP/HTTPS APIs | External APIs | Remote catalog sources and plugin resolution |

---

## Environment Variables

| Variable | Required | Purpose | Example |
|----------|----------|---------|---------|
| `HOME` | Yes | Base directory for XDG path resolution | `/Users/aaronbassett` |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory | `/opt/var` |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) |
| `CLICOLOR` | No | Disable coloured output (alternate) | `0` to disable |

---

## Databases & Data Stores

### Phase 1: File-Based Storage Only

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| Local filesystem (XDG) | TOML config | User configuration | `${XDG_CONFIG_HOME:-~/.config}/tome/config.toml` |
| Local filesystem (XDG) | Catalog cache | Cloned catalog repositories (git working trees) | `${XDG_DATA_HOME:-~/.local/share}/tome/catalogs/<sha256>` |

### Cache Structure

- Each remote catalog source is content-addressed by `sha256(url)`
- One directory per unique catalog URL
- Contents: bare or normal Git working tree, refreshed on `tome catalog update`
- Atomic writes via `tempfile` crate (rename-based) to prevent corruption on interrupt

---

## Authentication & Authorization

Phase 1 has no explicit authentication.

- Git operations inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`)
- No API keys or OAuth tokens required
- `tomecatalog.toml` ownership validated by file system permissions (owner email field is metadata only)

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and failure modes. It will be updated when Phase 2 adds database and embedding integrations.*
