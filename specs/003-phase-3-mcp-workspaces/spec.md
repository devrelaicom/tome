# Feature Specification: Phase 3 — MCP Server, Workspaces, and Doctor

**Feature Branch**: `003-phase-3-mcp-workspaces`
**Created**: 2026-05-14
**Status**: Draft
**Input**: User description: "time for phase 3 — PRDs/phase-3.md; read review/ and .sdd/codebase/ for context and write the spec"

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details.

**Source PRD**: [PRDs/phase-3.md](../../PRDs/phase-3.md) — the implementation-level brief is authoritative for HOW; this spec is authoritative for WHAT.

## Overview

Phase 1 gave Tome catalogs. Phase 2 made the inventory inside those catalogs queryable through a local semantic index that the developer drives from the CLI. Phase 3 turns Tome from a developer-only CLI into something **agents can consume**, and gives the same machine room for **multiple independent installations** of state — one per project, with a global one as fallback.

Three new surfaces ship together because they unblock each other:

- **`tome mcp`** — a stdio Model Context Protocol server, backed by the Phase 2 index, that exposes `search_skills` and `get_skill` tools. Any compliant harness (Claude Code, Codex, Cursor, Gemini CLI, OpenCode) can register it as a child process and call the two tools to find and load skill content. This is the entire point of Phase 3 for end users.
- **Workspaces** — a `.tome/` directory in any project folder makes Tome operate against that project's own catalog list, its own enabled set, and its own index database. The global state from Phase 1 and Phase 2 remains; workspaces sit alongside it. Discovery is by CWD walk with explicit overrides; no automatic migration between global and workspace state.
- **`tome doctor`** — a read-only diagnostic command that reports model state, index state, catalog caches, workspace context, and which harnesses are detected on the local machine. `tome status` (Phase 2) remains the lock-free pre-flight; `tome doctor` is its diagnostic-superset sibling and is the place developers look first when something is wrong. `--fix` performs the obvious automatic repairs (re-download missing models, re-clone broken catalog caches, run forward DB migrations); destructive fixes are never automatic.

Schema-migration plumbing also lands in Phase 3 even though no schema actually changes between Phase 2 and Phase 3. Each workspace has its own database; each database must carry its own schema-version row and refuse to open under a newer Tome version it does not understand. The machinery is added now so Phase 4+ DB bumps run forward migrations cleanly inside the same atomic-write discipline that protects ordinary writes.

This phase is deliberately bounded. Cross-harness file installation, hooks/commands/agents translation, HTTP/SSE transport for MCP, multi-tenant MCP, and authentication are all explicitly deferred. The MCP server is one process per agent session; stdio only; one workspace (or global) per process lifetime.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Drive Tome from inside an agentic coding harness via MCP (Priority: P1)

A developer has registered one or more catalogs and enabled some plugins. They now want their agentic coding assistant — Claude Code, Codex, Cursor, Gemini CLI, or any other MCP-compliant client — to use Tome to find and load skill content automatically, without the developer typing CLI commands inside their editor.

**Why this priority**: This is the entire visible user value of Phase 3 for end users. Phase 2 produced a queryable index; Phase 3 makes it consumable by agents. Without this story, the workspace and doctor surfaces have nothing important to be scoped over.

**Independent Test**: From a working Phase 2 install with at least one enabled plugin, the developer registers `tome mcp` as an MCP server in any compliant harness's configuration. Inside an agent session in that harness, the agent calls `search_skills` with a natural-language query and receives a ranked list of matches. The agent then calls `get_skill` with one of the returned identifier triples and receives the full skill body plus a list of resource paths for every other file in the skill's directory. Closing and re-opening the agent session reuses the same MCP server process model with no developer intervention.

**Acceptance Scenarios**:

1. **Given** a working Phase 2 install with at least one enabled plugin and both models present, **When** a compliant MCP client launches the Tome MCP server as a child process and performs the handshake, **Then** the server advertises exactly two tools — one for searching skills by natural language, one for fetching a single skill by identifier — and is ready to serve calls.
2. **Given** the MCP server is running, **When** the client calls the search tool with a natural-language query and an optional result cap, **Then** the server returns a ranked list of matching skills, each carrying its catalog and plugin identifiers, its skill name, its description, its plugin version, the absolute on-disk path of its file, and a score.
3. **Given** the search tool is called with a catalog filter or a catalog-and-plugin filter, **When** the server processes the call, **Then** results are restricted to that scope before ranking, so the reranker only ever sees candidates that passed the filter.
4. **Given** the client calls the get-skill tool with a triple of catalog, plugin, and skill name that resolves to an enabled skill, **When** the server processes the call, **Then** it returns the full body of the skill declaration file with its metadata header stripped and the body otherwise verbatim, the absolute path of that file, and a list of absolute paths to every other file in the skill's directory.
5. **Given** the client calls the get-skill tool with a triple that does not resolve to any enabled skill, **When** the server processes the call, **Then** the tool returns a structured error identifying which part of the triple did not match.
6. **Given** the server starts, **When** the local installation is in a state where queries cannot serve correctly (missing database, missing required embedder model, schema-version mismatch, embedder drift, or model checksum mismatch), **Then** the server refuses to start, writes the diagnosis to its file log and to its standard error, and exits with a dedicated non-zero status; the harness sees the child process die and surfaces the failure to the developer.
7. **Given** the server is running and the developer signals it to terminate, **When** the signal arrives, **Then** any in-flight tool call completes within a bounded number of seconds, any partial state is rolled back, and the process exits without corrupting the index database the CLI also reads from.
8. **Given** the agent description text for each MCP tool, **When** the harness lists the available tools, **Then** the descriptions invite proactive use of the search tool before approaching a substantial task, identify the get-skill tool as the natural follow-up to a search result, and do not enumerate any specific catalog, plugin, or skill names.

---

### User Story 2 — Create a workspace and have Tome operate on it automatically (Priority: P2)

A developer is working in a project that should have its own catalog set and its own enabled plugins, separate from whatever they have configured globally. They want Tome to detect they are in that project and operate against the project's state — without typing flags every time.

**Why this priority**: Phase 3 has two big ideas: the MCP server and workspaces. Workspaces are what lets a developer run an MCP server scoped to "this project" and get answers about this project's enabled plugins, not whatever they happened to enable globally six months ago. Without workspaces, the MCP server is useful but not project-aware.

**Independent Test**: From an existing Phase 2 install with some global state, the developer runs the workspace initialization command in a fresh directory. The command creates a `.tome/` subdirectory with the workspace's own configuration and an empty index database. Running any existing Tome command from inside that directory or any subdirectory of it operates on workspace state, not global. Asking Tome to describe the current workspace context reports the resolved workspace path and how it was resolved. Moving out of the directory and running the same commands operates on global state again, with no developer action.

**Acceptance Scenarios**:

1. **Given** a directory with no Tome workspace, **When** the developer initializes a workspace there, **Then** a workspace marker directory is created at that path containing an empty workspace configuration file and an empty workspace index database; the global state is unchanged.
2. **Given** the developer initializes a workspace with the explicit "inherit from global" option, **When** the workspace is created, **Then** the workspace configuration is seeded with the catalog list currently known globally, but no plugin is marked enabled in the new workspace — the developer must enable plugins per workspace explicitly.
3. **Given** a workspace exists at a path, **When** the developer runs any existing Tome command from that path or any subdirectory of it without overrides, **Then** the command operates on workspace state: catalog list, enabled set, and index database all come from the workspace.
4. **Given** the developer is inside a workspace, **When** they pass the explicit "use global state" flag on any command, **Then** that single command operates on global state regardless of the surrounding workspace.
5. **Given** a workspace exists at a path, **When** the developer is outside that path and any other workspace path, **Then** Tome operates on global state.
6. **Given** the developer sets a workspace path environment variable, **When** any command runs, **Then** that path is used as the workspace regardless of the current directory.
7. **Given** the developer passes an explicit workspace path flag on the command line, **When** the command runs, **Then** the flag wins over the environment variable, the CWD walk, and the global fallback.
8. **Given** a directory has a workspace marker that is malformed or refers to a missing index database, **When** any command runs there, **Then** the command exits with a dedicated error pointing the developer at the doctor command rather than silently falling back to global state.
9. **Given** the developer asks Tome to describe the current workspace context, **When** the command runs, **Then** the output reports the resolved workspace path (or "global"), which resolution method picked it (flag, environment variable, CWD walk, or global fallback), how many catalogs and plugins the workspace knows about, how many plugins are enabled, how many skills are indexed, and the workspace database's schema version and embedder identity.

---

### User Story 3 — Have every existing command honour the current workspace (Priority: P2)

A developer who has been driving Tome from the global Phase 2 install now wants their existing muscle memory to continue to work after they introduce a workspace. Catalog management, plugin enable/disable, query, reindex, status — all of it should "just work" against the workspace when they are inside one.

**Why this priority**: Without this story, workspaces are a new sub-tool that requires its own commands and is essentially a parallel universe. With it, workspaces are a property of the environment that every existing command silently honours. The first behaviour ages badly; the second is the only sustainable shape.

**Independent Test**: From a workspace with one catalog added, the developer enables a plugin and verifies that the global install's enabled set is unchanged. They run a query and see only workspace-enabled plugins in the results. They run the reindex command and only workspace-enabled plugins are touched. They run the status command and see workspace state, not global. Removing a catalog from the workspace does not affect the global registry. Conversely, with an explicit "use global" flag on any of the same commands, the global state is the one that mutates and the workspace is untouched.

**Acceptance Scenarios**:

1. **Given** the developer is inside a workspace, **When** they add a catalog without overrides, **Then** the catalog is recorded in the workspace's catalog list, the global catalog list is unchanged, and the workspace records its own catalog-pin reference.
2. **Given** the developer is inside a workspace and the catalog they are adding has already been cloned globally or by another workspace, **When** the workspace records that catalog, **Then** Tome reuses the existing on-disk clone (one clone on disk per unique upstream URL) rather than cloning a duplicate copy.
3. **Given** the developer is inside a workspace, **When** they enable a plugin, **Then** the plugin is marked enabled in the workspace's index database and is not marked enabled in any other workspace or in the global database.
4. **Given** the developer is inside a workspace with some enabled plugins, **When** they run a query, **Then** the results only reflect skills indexed in this workspace's database; skills enabled globally or in other workspaces never appear.
5. **Given** the developer is inside a workspace and runs reindex without arguments, **When** the command runs, **Then** only this workspace's enabled plugins are reindexed.
6. **Given** the developer is inside a workspace and runs status, **When** the command runs, **Then** the report reflects this workspace's database, this workspace's catalogs, and the same shared embedder and reranker installation that the global install uses.
7. **Given** the developer is inside a workspace, **When** they pass the explicit "use global state" flag on any command, **Then** the command operates against the global state exactly as it would have outside any workspace.
8. **Given** the last reference to a particular upstream catalog URL is removed (the only workspace that referenced it removed the catalog), **When** the cleanup completes, **Then** the shared on-disk clone of that URL is removed; if any other workspace or the global registry still references the URL, the on-disk clone remains.

---

### User Story 4 — Diagnose a broken local installation with one command (Priority: P3)

A developer is debugging odd behaviour. Maybe queries are returning nothing, maybe a catalog refresh failed last week, maybe the MCP server keeps dying on startup, maybe they don't remember which workspace they're in. They want a single command that paints a complete picture of what's installed, what's missing, what's stale, and what they should run to fix it.

**Why this priority**: Phase 2 shipped `tome status` as the lock-free pre-flight. `tome doctor` extends that surface: same read-only-by-default discipline, but also reports workspace context, the harness directories the developer has installed locally, and per-catalog cache health, and offers an opt-in `--fix` flag that performs the obvious non-destructive repairs. The two commands coexist by design: `status` is fast and narrow, `doctor` is broad and slower.

**Independent Test**: On a clean install, the doctor command reports every subsystem as healthy and exits zero. Removing one model file makes the report flag the model as missing, the command exits non-zero, and the suggested-fixes section names the exact command to re-download. Running with the fix flag re-downloads it and re-runs the same diagnosis automatically. Deleting a catalog cache directory by hand makes the report flag the cache as broken and offers to re-clone it. From inside a workspace, the report identifies the workspace context; from outside any workspace, it reports the global fallback. The report also lists which agentic coding harnesses are detected on the local machine.

**Acceptance Scenarios**:

1. **Given** a clean and healthy install (global or workspace) with both models present, a valid index database, and every registered catalog cache intact, **When** the developer runs the doctor command, **Then** every subsystem is reported as healthy with explicit per-subsystem markers and the command exits zero.
2. **Given** the developer is inside a workspace, **When** the doctor command runs, **Then** the report includes a workspace-context section identifying the resolved workspace path, the resolution method, and the workspace's database schema and embedder identity, distinct from the global ones.
3. **Given** one of the model files is missing or its on-disk checksum disagrees with the recorded checksum, **When** the doctor command runs, **Then** the models subsystem is reported as unhealthy with a specific diagnosis, the suggested-fixes section names the command to re-download, the command exits non-zero, and the report on other subsystems is independently complete.
4. **Given** the same situation, **When** the developer re-runs the doctor command with the fix flag, **Then** the model is re-downloaded with progress, the diagnosis is re-run automatically, and the command exits zero if everything is now healthy.
5. **Given** a catalog cache directory has been deleted or is no longer a valid Git repository, **When** the doctor command runs, **Then** that catalog's cache is reported as broken with a specific diagnosis and the suggested fix points at re-cloning from the recorded URL.
6. **Given** the same situation, **When** the developer re-runs with the fix flag, **Then** the catalog cache is re-cloned at the same pinned reference and the diagnosis is re-run.
7. **Given** the workspace database schema is older than what the running tool expects, **When** the doctor command runs, **Then** the schema subsystem is reported as needing a forward migration; with the fix flag, the migration runs inside the same atomic-write discipline as ordinary writes and the diagnosis re-runs.
8. **Given** the workspace database schema is newer than what the running tool understands, **When** the doctor command runs, **Then** the report identifies the version mismatch and the suggested action is to upgrade Tome — never to mutate the database. The fix flag MUST NOT attempt any kind of backward migration in this case.
9. **Given** the developer has one or more agentic coding harnesses installed on the local machine, **When** the doctor command runs, **Then** the report names each detected harness by reading well-known per-user directories (the same directories future cross-harness installation will write into), without attempting to read those harnesses' own configuration files.
10. **Given** the developer pipes the doctor output to a file, **When** the command runs, **Then** colours, glyphs, and progress are suppressed and structured output is available with all the same fields.
11. **Given** the fix flag is passed, **When** any required repair would touch developer data in an unrecoverable way (for example, dropping a database known to be in use), **Then** the fix flag MUST NOT perform that repair automatically; the report MUST surface the suggested command and require the developer to run it explicitly.

---

### User Story 5 — Migrate index database schemas forward as Tome evolves (Priority: P3)

A developer upgrades Tome to a version that has bumped the index schema. They expect the upgrade to be silent in the common case: the older database on disk is brought to the new schema atomically the next time it is opened, and the developer keeps using their plugins as if nothing happened. They also expect the opposite direction — running an older Tome against a database produced by a newer Tome — to be refused with a clear error rather than risking corruption.

**Why this priority**: No schema actually changes between Phase 2 and Phase 3. But every Phase 4+ feature that touches the index will need this machinery and the discipline that comes with it. The cost of building it now is small; the cost of building it later for the first time is a real risk of an avoidable data-loss incident.

**Independent Test**: A database that records an older schema version, opened by a newer Tome, has its forward migration applied inside one atomic-write boundary and the schema version on disk is updated to the new value. A database that records a schema version newer than the running Tome's expected value is refused with a dedicated error and is never opened for writing. The migration machinery applies equally to the global database and to every workspace database — both carry their own schema version row.

**Acceptance Scenarios**:

1. **Given** an index database on disk recording an older schema version than the running Tome expects, **When** any read-or-write operation opens that database, **Then** every pending forward migration runs in order inside one atomic-write boundary; if any migration fails, the database is left at the original schema version with no intermediate state; on success, the recorded schema version equals the new value.
2. **Given** an index database on disk recording a schema version newer than the running Tome expects, **When** any operation opens that database, **Then** the operation exits with a dedicated non-zero status and a message pointing the developer at upgrading Tome; no attempt is made to convert the schema downward.
3. **Given** a workspace database and the global database recording different schema versions, **When** the running Tome opens each in turn, **Then** each is migrated forward (or refused, per the same rules) independently of the other.
4. **Given** the developer runs the doctor command with the fix flag against a database whose schema needs forward migration, **When** the migration runs, **Then** the developer sees a clear notification that a migration occurred and against which database; status and doctor agree on the post-migration schema version on every subsequent invocation.

---

### Edge Cases

- **Workspace inside a workspace**: a developer initializes a workspace inside a subdirectory of an existing workspace. The closer (more deeply nested) workspace wins. The outer workspace is unaffected and continues to be visible to commands run outside the inner workspace's subtree. No "merging" of state across the two.
- **Workspace marker present but no database**: the `.tome/` directory exists but the workspace database is absent (e.g. the developer initialized but never enabled anything). Commands that read the index see an empty workspace, not an error. Status and doctor explicitly report "not yet bootstrapped" as an informational state, not an unhealthy one.
- **Workspace database present but no marker file content**: the marker exists, the database exists, but the workspace configuration file is empty or malformed. The command exits with a dedicated error rather than treating the workspace as empty — this is a corruption signal.
- **MCP server registered with both `--workspace` and `--global` flags**: the server refuses to start with a usage error rather than picking one silently.
- **MCP server running while a CLI writer holds the index lock**: per Phase 2's concurrency model, the read paths (search, get-skill) do not block on the writer. If the writer holds the lock past the documented timeout, the search may surface stale results from the WAL view but never corrupted ones. The server never exits because a CLI writer is slow.
- **MCP server running against a workspace that the user deletes mid-session**: the server has an open connection to the workspace database file. Operations continue to serve from the open handle until the next reconnect. A subsequent re-launch of the server fails with a workspace-resolution error.
- **Two MCP server processes for two different workspaces**: each is its own process with its own connection to its own database. They share the same global model files. No coordination between them; the OS handles process isolation.
- **MCP `search_skills` call when no plugins are enabled in the resolved workspace**: the server returns an empty result array, not an error. Same shape as a query that legitimately finds nothing.
- **MCP `get_skill` call for an enabled skill whose file has been deleted from disk after enable**: the server returns a structured error identifying that the file is missing; the index is not silently updated by the read path.
- **Shared catalog cache reference-counting race**: two processes simultaneously remove the last reference to a catalog URL. The cleanup is best-effort and idempotent — at worst the directory has already been removed when the second process tries, which is success-equivalent.
- **`tome doctor --fix` cannot reach upstream to re-download a model**: the fix is reported as failed for that subsystem; other independent fixes (e.g. catalog re-clone) still proceed where they can.
- **`tome doctor --fix` mid-run interruption**: the same atomic-write discipline applies. Any in-flight repair is rolled back at its own boundary; the overall command exits with the dedicated interrupted status.
- **Pre-existing global lockfile when starting an MCP server against a workspace**: irrelevant — workspaces have their own lockfiles. The MCP server respects the workspace's lockfile, not the global one.
- **Workspace path containing characters that need escaping** in any environment variable form: Tome treats the resolved path as an opaque filesystem path; the developer's shell is responsible for quoting. The workspace-info command echoes the resolved absolute path so the developer can see what Tome parsed.
- **`--global` and `--workspace <path>` on the same command**: usage error, refuse to run with a clear message.
- **MCP tool descriptions encountered by an LLM client that aggressively summarizes**: descriptions are written to be short enough to survive aggressive summarization while still inviting proactive use. They do not enumerate any specific catalog, plugin, or skill by name.
- **MCP server stdout used for anything other than the protocol channel**: forbidden by the protocol. All Tome logs in MCP mode go to a file and, for fatal startup-only errors, to the server's standard error.
- **`tome doctor` against the global state from inside a workspace** (with the explicit "use global state" flag): reports global subsystems exclusively, omits the workspace-context section, and notes which workspace was overridden.
- **Schema-too-new on a workspace database while the global database is on-version**: the affected workspace's commands refuse; commands operating on the global database continue to function. Doctor reports the failure scoped to the affected workspace.
- **Schema migration races with another process**: forward migrations are taken under the same advisory lockfile that protects ordinary writes; only one process attempts the migration; the other waits up to the documented index-busy timeout, then either re-opens (now at the new schema) or exits with the index-busy status.

## Requirements *(mandatory)*

### Functional Requirements

**MCP server**

- **FR-101**: The system MUST provide an MCP server entry point invoked as a single Tome subcommand. The server MUST speak the standard input / standard output transport defined by the Model Context Protocol; alternative transports MUST NOT be added in Phase 3.
- **FR-102**: The server MUST be designed as a long-lived child process launched once per agent session. Switching the resolved workspace MUST require terminating and re-launching the server; an in-process workspace switch MUST NOT be possible.
- **FR-103**: The server MUST advertise exactly two tools to its client: a search tool that performs ranked retrieval over enabled skills, and a fetch tool that loads one skill's body and resource paths by identifier.
- **FR-104**: The search tool MUST accept a free-form natural-language query and the following optional parameters: a result cap with a documented default, a catalog filter, and a plugin filter scoped to a catalog filter. Filters MUST be applied before reranking so the reranker only sees the filtered candidate pool.
- **FR-105**: The search tool MUST return, per result, the catalog and plugin identifiers, the skill name, the indexed description, the plugin version recorded at enable or last reindex time, the absolute on-disk path of the skill declaration file, and a score that reflects the reranker output when reranking is on or raw embedding similarity when it is off.
- **FR-106**: The fetch tool MUST accept exactly three required parameters — catalog, plugin, skill name — and MUST return the full body of the skill declaration file with its metadata header stripped and the remainder verbatim, the absolute on-disk path of that file, and a list of absolute on-disk paths to every other file in the skill's directory. The fetch tool MUST NOT inline the content of those resource files; the client is expected to load them with its own file-reading tools if it wants them.
- **FR-107**: If the identifier triple passed to the fetch tool does not resolve to a currently enabled skill, the tool MUST return a structured error identifying which part of the triple did not match and MUST NOT fall back to a partial match.
- **FR-108**: The descriptions registered for both tools MUST invite proactive use of the search tool before approaching a substantial task and MUST identify the fetch tool as the natural follow-up to a search result. The descriptions MUST NOT enumerate any specific catalog, plugin, or skill name. The exact wording is an implementation detail; the constraints are normative.
- **FR-109**: The server MUST eagerly load the embedder model at startup and MUST lazy-load the reranker model on the first search call. If either model is missing or fails its integrity check at the moment it is required, the server MUST exit before serving that call rather than silently degrading.
- **FR-110**: Before accepting any protocol messages, the server MUST verify that the resolved index database exists and is readable, that its recorded schema version matches what the running Tome expects, that the embedder identity recorded in the index matches the installed embedder identity, that the required model files exist on disk, and that those model files match their recorded cryptographic checksums. Any one of these checks failing MUST cause the server to exit with a dedicated non-zero status.
- **FR-111**: The server MUST write diagnostic logs to a file under the per-user state directory rather than the per-user data or cache directory; the path MUST follow the same XDG-aware resolution discipline as Phase 1 paths. Standard output is reserved for the protocol channel and MUST NOT carry log output. Standard error MUST carry only fatal startup-only errors that occur before the server can write to its log file. Log filtering MUST honour the same environment variable convention used elsewhere in Tome.
- **FR-112**: The server MUST handle the interruption signal cleanly: any in-flight tool call MUST complete or roll back within a bounded number of seconds; the index database MUST be left in a state that any other Tome process — CLI or another MCP server — can subsequently open and use.
- **FR-113**: The server MUST honour the workspace resolution rules defined below (FR-130 through FR-135) at startup; once resolved, the workspace identity is fixed for the process lifetime.

**Workspaces**

- **FR-130**: The system MUST treat a directory containing a marker subdirectory (named for the tool) as a workspace. The marker subdirectory MUST hold the workspace's own catalog configuration file and its own index database file.
- **FR-131**: Workspace resolution MUST follow this priority order, deterministically: an explicit workspace flag on the command line, an explicit "use global state" flag on the command line, a workspace-path environment variable, a walk from the current working directory toward the filesystem root looking for a workspace marker (first hit wins), and finally the global fallback. The two explicit flags MUST be mutually exclusive on the same invocation.
- **FR-132**: The system MUST persist workspace state independently of global state: the workspace's catalog list and the workspace's enabled-plugin state MUST live in the workspace's own files. Enabling a plugin in one workspace MUST NOT make it enabled in another workspace or in the global state.
- **FR-133**: The system MUST share the on-disk catalog clone (the cached files cloned from each catalog's upstream Git URL) across every workspace and the global state. Only the workspace's record of which catalogs it knows about — and the per-workspace pinned reference — MUST be per-workspace. Reference-counting MUST cleanly remove the on-disk clone when the last referencing record is removed.
- **FR-134**: The system MUST share the embedder and reranker model artefacts globally; there MUST NOT be a per-workspace copy of model files. Each workspace's index database MUST record its own embedder and reranker identity so drift is detectable per workspace.
- **FR-135**: The system MUST provide an explicit "initialize a workspace" command that creates the workspace marker directory and its empty configuration file. The command MUST refuse to overwrite an existing workspace unless a force flag is passed. The command MUST accept an optional flag that seeds the workspace's catalog list from the global catalog list at the moment of creation; this option MUST NOT copy any enabled-plugin state — every workspace starts with no plugins enabled regardless of seeding.
- **FR-136**: The system MUST provide a workspace-information command that reports the resolved workspace path (or "global"), the resolution method that picked it, the workspace's catalog count and plugin count, the count of enabled plugins, the count of indexed skills, the workspace database's schema version, and the embedder identity recorded against that workspace's stored vectors. The command MUST be available in both human and structured forms.
- **FR-137**: Every existing Phase 1 and Phase 2 command MUST resolve a workspace using the same priority order and MUST operate on the resolved workspace's state. A command run from inside a workspace with no overrides MUST NOT mutate global state.
- **FR-138**: Every existing command MUST accept the explicit "use global state" flag and the explicit workspace-path flag as global flags at the top level. The flags MUST behave identically regardless of which subcommand they appear with.
- **FR-139**: The system MUST log the resolved workspace at debug verbosity on every invocation so developers debugging an unexpected workspace pick can see it without dedicated tooling.
- **FR-140**: The system MUST refuse to operate against a malformed workspace (marker present, configuration file unreadable, or index database file present but rejected by the integrity check) rather than silently falling back to global state. The error MUST point the developer at the doctor command.
- **FR-141**: The system MUST NOT provide tooling to migrate state between global and workspace; opting into a workspace is a deliberate act and there is no automatic conversion.

**Doctor**

- **FR-160**: The system MUST provide a doctor command, separate from the Phase 2 status command, that performs read-only diagnostic checks across every Phase 1, Phase 2, and Phase 3 subsystem and reports each one independently. The status command MUST continue to behave exactly as in Phase 2.
- **FR-161**: The doctor command MUST report, at minimum:
  - the resolved workspace context (path or "global", resolution method, schema version, embedder identity recorded in that workspace);
  - the presence, on-disk size, and checksum status of every required model artefact, in the same vocabulary as the Phase 2 model list command;
  - the index database: integrity, recorded schema version, embedder and reranker identity, total skill count, enabled skill count;
  - the catalog cache: for every catalog the resolved configuration knows about, whether the on-disk clone directory exists, whether it is a valid Git repository, and whether it contains a parseable catalog manifest;
  - the set of agentic coding harnesses detected on the local machine, identified by the presence of their well-known per-user directories.
- **FR-162**: The doctor command MUST classify each per-subsystem finding as healthy, non-fatal-degraded, or broken, using visually distinct markers. The command MUST exit non-zero if any finding is non-fatal-degraded or broken.
- **FR-163**: When any finding is not healthy, the doctor command MUST emit a separate "suggested fixes" section listing the specific Tome commands the developer should run to resolve each finding. The fixes MUST be expressed as actual commands the developer can copy and paste, not as prose.
- **FR-164**: The doctor command MUST accept an opt-in "fix" flag that performs every fix the report would have suggested for the following classes: a missing or corrupt model can be re-downloaded; a broken catalog cache can be re-cloned from the catalog's recorded URL; an older index schema can have its forward migrations applied. Fixes MUST run inside the same atomic-write discipline as ordinary writes. If a fix succeeds, the corresponding subsystem MUST be re-reported as healthy in the same invocation.
- **FR-165**: The doctor command MUST NOT, even with the fix flag, perform any repair that would touch developer data in an unrecoverable way (for example, dropping a database, removing skills, or downgrading a schema). Such repairs MUST be surfaced as suggested commands the developer must run explicitly.
- **FR-166**: The doctor command MUST be available in both human and structured forms. The structured form MUST expose the same per-subsystem classification and the same suggested fixes as the human form.
- **FR-167**: The doctor command MUST validate Tome's own state. It MUST NOT attempt to read or validate any agentic coding harness's own configuration files; harness presence is established only by the existence of a well-known per-user directory and is reported informationally.
- **FR-168**: The doctor command MUST respect the same workspace resolution rules as every other command (FR-131). Running doctor from inside a workspace MUST report workspace state by default; the explicit "use global state" flag MUST report global state instead and MUST note which workspace was overridden.

**Schema migration**

- **FR-180**: Every index database file (global and per-workspace) MUST record its schema version in a dedicated metadata row.
- **FR-181**: When opening an index database whose recorded schema version is older than what the running Tome expects, the system MUST run every registered forward migration in order, inside one atomic-write boundary, under the same advisory lockfile that protects ordinary writes. On success, the recorded schema version MUST equal the new value. On failure of any single migration, the database MUST remain at the original schema version with no intermediate state.
- **FR-182**: When opening an index database whose recorded schema version is newer than what the running Tome expects, the system MUST exit with a dedicated non-zero status and a message pointing the developer at upgrading Tome. The system MUST NOT attempt any kind of backward migration or schema rewrite under any circumstance.
- **FR-183**: Migrations MUST be registered as ordered, in-process declarations (from-version, to-version, transformation). External migration tooling MUST NOT be a developer-facing requirement.
- **FR-184**: The migration framework MUST be in place in Phase 3 even though no schema actually changes between Phase 2 and Phase 3. The framework MUST be exercised by tests against a synthetic older-version database to prove the forward-migration path runs end-to-end.

**Errors and exit codes**

- **FR-200**: All Phase 1 and Phase 2 exit codes and error types MUST continue to behave as in their respective phases.
- **FR-201**: The system MUST extend its closed set of error types and exit codes to cover, at minimum, the following new failure modes, each with its own enumerated case and its own dedicated exit status:
  - MCP server startup pre-condition failure (any one of: database missing, schema mismatch, embedder identity mismatch, model file missing, model checksum mismatch), distinct from each pre-condition's own existing code;
  - MCP protocol I/O failure (the server could not read from or write to standard input / standard output);
  - workspace marker malformed (marker present, configuration unreadable);
  - workspace not found at an explicit path (the path passed to the explicit workspace flag does not contain a workspace marker);
  - workspace conflict (mutually exclusive flags present on the same invocation);
  - schema-version-too-new (on-disk schema newer than the running tool understands);
  - schema migration failure (a registered forward migration returned an error during apply);
  - doctor fix not safe (an automatic fix was requested for a class that requires explicit developer action).
- **FR-202**: The closed-error-set principle MUST continue to hold: there MUST NOT be a generic "other" error variant.

**Presentation and CLI surface**

- **FR-220**: Every Phase 3 surface (the MCP server's own diagnostics, the workspace-information command, the doctor command) MUST honour the same human / structured-output discipline as Phase 1 and Phase 2.
- **FR-221**: The MCP server's standard output MUST be the protocol channel exclusively. No human text MUST appear on standard output in MCP mode.
- **FR-222**: The MCP server's standard error MUST be silent in the steady state. Only fatal startup-only errors that occur before the log file is open MUST appear on standard error.
- **FR-223**: The doctor command's human form MUST use the same colour and TTY-detection rules as Phase 2 commands. Structured form MUST be byte-stable regardless of terminal context.

### Non-Functional Requirements

- **NFR-101**: The stripped release binary MUST continue to stay within the binary-size cap defined in the constitution. The Phase 3 MCP SDK and any new direct dependencies MUST be measured against the cap. If the cap would be breached, the plan MUST revise its component choices rather than waive the cap.
- **NFR-102**: Each new third-party dependency added in Phase 3 MUST carry a written justification per the constitution. The plan MUST enumerate every new direct and significant transitive dependency together with its licence and confirm each licence is within the constitution's allowlist.
- **NFR-103**: The MCP server MUST start in under one second on a recent laptop, measured from process spawn to readiness to accept the first protocol message. The eagerly-loaded embedder is included in that budget; the lazily-loaded reranker is not.
- **NFR-104**: A single end-to-end `search_skills` call against an index of around 100 skills with the reranker active MUST complete in under 300 ms at the 50th percentile and under 600 ms at the 99th percentile on a recent laptop.
- **NFR-105**: The sensitive-material scrubbing discipline from Phase 1 (Git output, model download URLs, error chains derived from such operations) MUST continue to apply to every Phase 3 surface. No catalog URL, workspace path containing sensitive segments, or upstream Git stderr fragment MUST appear unscrubbed in any MCP log, doctor report, or workspace-information output.
- **NFR-106**: Writes to workspace state (catalog list, index database) MUST be atomic in the same sense as Phase 1 and Phase 2 writes: a crash or interrupt MUST NOT leave files in a state any subsequent invocation refuses to interpret.
- **NFR-107**: All Phase 1 and Phase 2 quality gates (formatting, lint, typo, security audit, dependency policy, conventional commits) MUST continue to apply to Phase 3 code.

### Key Entities

- **Workspace**: a directory containing a workspace marker subdirectory. Holds the workspace's own catalog configuration file and its own index database file. Identified by an absolute filesystem path. Resolved deterministically from a documented priority order on every command invocation.
- **Workspace configuration**: the per-workspace declarative file that records which catalogs the workspace knows about and the per-workspace pinned reference for each. Strict schema; unknown fields rejected.
- **Workspace index**: the per-workspace database file. Carries its own schema version, its own embedder and reranker identity, and its own skill rows. Independent of the global database and of every other workspace's database.
- **Shared catalog cache**: the on-disk clone of one upstream catalog URL. Lives in a single shared location regardless of how many workspaces (or the global state) reference it. Reference-counted by the set of catalog records that name its URL.
- **Shared model artefacts**: the on-disk embedder and reranker model files. Live globally; never duplicated per workspace.
- **MCP server process**: a single long-lived Tome child process spawned by an MCP-compliant client. Resolves exactly one workspace (or global) at startup and is fixed to that workspace for its lifetime. Owns the embedder eagerly and the reranker lazily.
- **MCP tool: search**: the natural-language search tool advertised by the server. Accepts a query string and optional filters; returns a ranked list of skills.
- **MCP tool: fetch**: the get-skill-by-identifier tool advertised by the server. Accepts the (catalog, plugin, name) triple; returns the skill body, the skill file path, and the list of resource paths.
- **Schema migration**: a registered forward transformation from one index schema version to the next. Applied in order under the workspace's (or global) advisory lockfile inside one atomic-write boundary.
- **Detected harness**: a per-user directory belonging to a known agentic coding harness, found by Tome on the local machine. Reported informationally by the doctor command; never written to or read from for configuration in Phase 3.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-101**: The MCP server registered in any compliant MCP harness produces working search-skills and get-skill calls from inside a session in that harness with no developer action beyond writing the server registration entry.
- **SC-102**: The MCP server starts and is ready to accept its first protocol message in under one second on a recent laptop, measured from process spawn.
- **SC-103**: A search-skills call against an index of around 100 skills with the reranker active completes in under 300 ms at the 50th percentile and under 600 ms at the 99th percentile on a recent laptop.
- **SC-104**: A workspace initialization in a fresh directory produces a working workspace; subsequent Tome commands in that directory or any subdirectory operate against the workspace's state, not the global state.
- **SC-105**: A workspace initialized with the explicit "inherit from global" option contains exactly the global catalog list at the moment of creation and zero enabled plugins, regardless of how many plugins are enabled globally.
- **SC-106**: The explicit "use global state" flag works from inside a workspace on every shipping Phase 1, Phase 2, and Phase 3 command; commands so invoked never mutate the workspace and always operate on the global state.
- **SC-107**: The doctor command on a healthy install reports every subsystem as healthy and exits zero; corrupting any one subsystem (removing a model, deleting a catalog cache, mutating the index schema-version row to a recognised-older value) causes that subsystem to be reported as not healthy and the command to exit non-zero, while leaving the report on every other subsystem complete and accurate.
- **SC-108**: The doctor command's fix flag correctly repairs the three supported repair classes (missing model re-downloaded, broken catalog cache re-cloned, older-schema database forward-migrated) and re-reports the affected subsystem as healthy in the same invocation; it never attempts a repair outside those three classes.
- **SC-109**: A schema-newer-than-expected database is refused with a dedicated error; no operation in Phase 3 (CLI, MCP server, doctor) ever writes to such a database.
- **SC-110**: All Phase 1 and Phase 2 success criteria continue to hold; the Phase 3 surface does not regress catalog management, plugin lifecycle, query results, status, or version output.
- **SC-111**: The MCP server's standard output carries only protocol messages; no Tome log, no human text, no progress glyph appears on standard output in any tested code path.
- **SC-112**: The workspace-information command, the doctor command, and the MCP server's startup diagnostics all agree on the resolved workspace path, the workspace's schema version, and the workspace's embedder identity for any given invocation.
- **SC-113**: Removing the last record referring to a particular upstream catalog URL (anywhere — global registry or any workspace) causes the shared on-disk clone to be removed; while any record still references that URL, the on-disk clone persists.
- **SC-114**: Two MCP server processes for two different workspaces, running simultaneously, each serve their own workspace's content correctly with no cross-contamination; both processes terminate cleanly on their respective interruption signals.
- **SC-115**: The forward schema-migration framework runs end-to-end against a synthetic older-version database fixture in tests, even though no schema actually changes between Phase 2 and Phase 3.

## Assumptions

- Phase 1 catalog management and Phase 2 plugin lifecycle, query, models, reindex, status, and version surfaces are in place and unchanged by Phase 3. Phase 3 reads through them; it does not modify their on-disk contracts beyond extending the workspace-resolution boundary.
- The Phase 2 closed `TomeError` enum, atomic-write discipline, advisory-lockfile discipline, and credential-scrubbing discipline carry forward unchanged. Phase 3 extends them; it does not relax them.
- The host has a working network connection for first-run model downloads and for doctor-fix re-downloads. Offline use of an already-initialized install continues to work as in Phase 2.
- The host has at least one directory writable as the per-user state directory for the MCP log file. If that directory is not writable, the MCP server falls back to fatal standard-error output and exits.
- The MCP-compliant client launches Tome's MCP server as a child process and is responsible for stopping it. Tome does not detect orphaned servers; the operating system handles process cleanup.
- A single global index database remains sufficient for users who do not opt into a workspace. Workspaces are an opt-in feature, not the default.
- Cross-harness file installation (writing into agentic coding harnesses' own plugin directories) remains explicitly deferred. Harness detection in doctor is informational only.
- The reranker remains on by default for both CLI queries and MCP search calls. The off-by-default debug flag established in Phase 2 continues to exist for CLI; the MCP search tool does not expose a no-rerank parameter in Phase 3.

## Dependencies on Phases 1 and 2

- The catalog registry, the catalog cache layout, the per-catalog Git operations, the credential-scrubbing boundary, and the closed `TomeError` enum are reused unchanged.
- The Phase 2 index schema (skills table, vector embeddings table, meta rows), the embedder and reranker trait interfaces, the model registry and download pipeline, the advisory lockfile discipline, the query KNN-plus-rerank pipeline, and the lifecycle orchestrator (enable, disable, reindex_plugin, cascade_disable_for_catalog, auto_disable_orphan) are reused unchanged.
- The Phase 2 status command (lock-free pre-flight, three-class health classification) coexists with the Phase 3 doctor command; status remains the fast narrow surface and doctor is the broad slower one.
- The Phase 2 extended `--version` output (Tome version, embedder identity, reranker identity) is unchanged by Phase 3.
- The Phase 1 atomic-write discipline is extended to cover workspace configuration files, workspace index databases, and the MCP log file's rotation boundary.

## Out of Scope (Phase 3)

- Cross-harness file installation: writing or symlinking enabled plugin content into the install directories of agentic coding harnesses.
- Translation of commands, agents, or hooks across harnesses.
- HTTP, SSE, or any non-stdio transport for the MCP server.
- Multi-tenant or shared MCP servers serving more than one client.
- Authentication, authorization, or remote access for the MCP server.
- MCP tool annotations beyond plain text descriptions (no caching hints, no permission scopes, no resource subscriptions).
- Tooling to migrate state between the global database and a workspace database (or vice versa). Workspaces are an explicit opt-in with no automatic conversion in either direction.
- Validation of a harness's own MCP configuration file as part of the doctor command. The doctor reports Tome's own state, not the harness's.
- Plugin authoring tools (scaffolding, lint, new).
- A second concrete schema migration. The migration framework lands in Phase 3 with synthetic-fixture tests; the first real migration ships with the Phase 4+ feature that needs it.
- A query command exposed to the MCP server beyond search-and-fetch. Reindex, enable, disable, catalog mutation, and status are all CLI-only in Phase 3.
