# Feature Specification: Phase 2 — Plugin Enable/Disable and Local Skill Index

**Feature Branch**: `002-phase-2-plugins-index`
**Created**: 2026-05-11
**Status**: Draft
**Input**: User description: "time to work on phase 2: PRDs/phase-2.md"

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details.

**Source PRD**: [PRDs/phase-2.md](../../PRDs/phase-2.md) — the implementation-level brief is authoritative for HOW; this spec is authoritative for WHAT.

## Overview

Phase 1 gave Tome catalogs: developers could register a remote source of plugins, inspect what it ships, and refresh it. Phase 2 makes that inventory useful at the command line. The developer opts into individual plugins (enable / disable), and Tome builds a local semantic search index over the skills declared inside those plugins so the developer can ask "do I have a skill for X?" and get a ranked answer in a single command.

Phase 2 separates two ideas that Phase 1 conflated:

- **Installed.** A plugin's files live on disk because the catalog that ships it has been registered. This happens automatically; there is no separate install verb.
- **Enabled.** The developer has explicitly opted into a plugin. Enabling it indexes its skills into a local search database. Disabling excludes them from query results.

Plugins are addressed everywhere by a two-part identifier `<catalog>/<plugin>`, so the same plugin name can appear in multiple catalogs without colliding.

This phase is deliberately self-contained: still no MCP server, still no workspaces, still no writing files into other agentic coding harnesses' install directories. The index is Tome-internal and is queried only through Tome itself.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Enable a plugin and find a skill by description (Priority: P1)

A developer has registered a catalog. They want to opt into one of its plugins and then, later, ask the tool a free-form question to discover which skill in that plugin is relevant.

**Why this priority**: This is the entire visible user value of Phase 2. Without it, none of the supporting machinery (model download, embedding pipeline, vector store) serves any purpose. P1 is the MVP slice — every other story in this spec exists to support, polish, or correct around this one.

**Independent Test**: From a fresh install on top of Phase 1, with one catalog registered that ships at least one plugin containing several skills, the developer enables that plugin by its `<catalog>/<plugin>` identifier, runs a query with a natural-language string, and sees a ranked, formatted list of matching skills. The same query in structured-output mode returns parseable structured records.

**Acceptance Scenarios**:

1. **Given** a registered catalog whose plugins are installed on disk but not enabled, **When** the developer enables a plugin by `<catalog>/<plugin>`, **Then** the plugin's skills are parsed from their declared metadata, their descriptions are converted into searchable vectors, and the developer is told how many skills were indexed.
2. **Given** at least one enabled plugin with indexed skills, **When** the developer runs a free-form query, **Then** the developer sees a ranked table whose columns identify each result by catalog, plugin, skill name, version, score, and on-disk location.
3. **Given** the same query with structured-output requested, **When** the command runs, **Then** the developer receives the same results in a structured format suitable for piping to other tools, with the same fields.
4. **Given** an enabled plugin, **When** the developer runs a query that has no obviously relevant skill, **Then** the developer still receives the top-ranked candidates ordered by relevance and is not shown an empty result by default.
5. **Given** the developer pipes the query output to a file or another command, **When** the command runs without a connected terminal, **Then** colours, spinners, and progress are suppressed automatically and the structured fields remain stable.
6. **Given** the developer requests the per-query result cap, **When** they pass a top-N flag, **Then** the result set is capped to that many records.
7. **Given** the developer wants to inspect raw embedding distance without the reranker, **When** they pass the no-rerank flag, **Then** the score column reflects pure embedding similarity and the result order may differ.

---

### User Story 2 — Browse catalogs interactively and pick plugins to enable (Priority: P2)

A developer who has just added one or more catalogs wants to see what is on offer and decide which plugins are worth enabling. They have no specific plugin name in mind; they want to browse.

**Why this priority**: Discovery is the path that gets a new user to P1. The non-interactive enable/disable commands are scriptable and sufficient for power users, but interactive browse is what turns "I added a catalog" into "I have something useful indexed."

**Independent Test**: From a registered catalog, the developer runs the plugin command with no subcommand, navigates a catalog list, drills into a plugin to see its metadata and component counts, and toggles its enabled state from within the same flow. Returning to the plugin browser reflects the updated state.

**Acceptance Scenarios**:

1. **Given** at least one registered catalog, **When** the developer enters interactive browse, **Then** they see a list of catalogs with each catalog's plugin count and how many of those are enabled.
2. **Given** a selected catalog in browse, **When** the developer enters its plugin list, **Then** every plugin is shown with its current enabled state and version, and the developer can pick any one to inspect.
3. **Given** a selected plugin in browse, **When** the plugin view is shown, **Then** the developer sees plugin metadata (name, version, last upstream change, author, description) and a component breakdown of how many skills, agents, commands, hooks, and embedded service declarations the plugin ships.
4. **Given** the plugin view of a disabled plugin, **When** the developer chooses to enable it, **Then** the embedding pipeline runs to completion with visible progress and the plugin view updates to show enabled state with the time of indexing.
5. **Given** the plugin view of an enabled plugin, **When** the developer chooses to disable it, **Then** the developer is asked to confirm before the change is applied.
6. **Given** any level of the interactive flow, **When** the developer chooses the escape option, **Then** they return to the previous level (plugin → catalog list → exit) until they leave the flow.
7. **Given** the command is invoked without a connected terminal, **When** the developer runs the plugin command with no subcommand, **Then** the command exits with a clear error pointing the developer at the non-interactive subcommands.

---

### User Story 3 — Disable a plugin without losing its index (Priority: P2)

A developer has previously enabled a plugin but no longer wants its skills to appear in query results. They expect re-enabling later to be cheap.

**Why this priority**: Symmetrical to enable. Required for the user to recover from a noisy enable decision. The "cheap re-enable" property keeps the developer from being punished for opting out.

**Independent Test**: An enabled plugin is disabled; a subsequent query never returns its skills. Re-enabling it without any change to its source files completes essentially instantly (no re-embedding) and the plugin's skills reappear in query results.

**Acceptance Scenarios**:

1. **Given** an enabled plugin, **When** the developer disables it without an explicit force flag, **Then** they are asked to confirm before the change applies.
2. **Given** an enabled plugin and a non-interactive context, **When** the developer attempts to disable it without the force flag, **Then** the command exits with a clear error rather than hanging on a prompt.
3. **Given** a disabled plugin whose source skills have not changed since it was last enabled, **When** the developer re-enables it, **Then** no embedding work is performed and the skills reappear in subsequent query results.
4. **Given** a disabled plugin, **When** the developer runs a query, **Then** none of that plugin's skills appear in results, even if their textual scores would have been high.

---

### User Story 4 — Manage the embedding and reranking models explicitly (Priority: P3)

The semantic search index depends on locally-installed models. The developer wants to download them, see whether they are present and intact, and remove them when needed.

**Why this priority**: First-run model download is essential to make the rest of the feature usable, but the developer-facing commands around models are infrequent. The download itself can be triggered on demand by P1/P2 flows; the explicit model commands exist for operability, repair, and disk-space hygiene.

**Independent Test**: The developer runs the model download command on a clean system; the embedder and the reranker land in the expected location, are listed as present, and report a healthy state. Running the same command again is a no-op. Removing one model causes the list to show it as missing and the next enable/query operation to require re-download.

**Acceptance Scenarios**:

1. **Given** a clean install, **When** the developer runs the explicit model download command, **Then** both required models (embedder and reranker) are downloaded with visible progress, verified against a published cryptographic checksum, and stored under the per-user data directory.
2. **Given** the models are already present and intact, **When** the developer runs the download command again, **Then** it is a no-op (no network activity) unless a force flag is passed.
3. **Given** the models are listed, **When** the developer asks for the model list, **Then** each model is shown with name, version, on-disk size, on-disk location, and status (ok / corrupt / missing).
4. **Given** a model is removed, **When** the developer next attempts an operation that needs it and stands at a connected terminal, **Then** they are prompted to download the missing model and the operation continues once download completes.
5. **Given** a model is missing in a non-interactive context, **When** the developer attempts an operation that requires it, **Then** the command exits with a dedicated non-zero status and a message pointing at the explicit download command.
6. **Given** an installed model whose recorded checksum does not match what is on disk, **When** the developer runs the list command, **Then** that model is marked as corrupt and the developer is told to re-download.

---

### User Story 5 — Keep the index in sync as upstream catalogs change (Priority: P3)

The developer refreshes a catalog whose plugins they have enabled. They expect the local index to reflect any upstream skill changes without their having to think about it.

**Why this priority**: Without this, the index quietly rots. Manual reindexing exists as an explicit escape hatch, but the catalog refresh path must do the right thing by default for the developer who never thinks about indexing.

**Independent Test**: A catalog refresh against a catalog with enabled plugins reports a summary of which skills were added, changed, or removed and re-indexes only the changed ones. An explicit reindex command is also available, scoped to all enabled content, a single catalog, or a single plugin.

**Acceptance Scenarios**:

1. **Given** a catalog with an enabled plugin and no upstream changes since last refresh, **When** the developer refreshes the catalog, **Then** no embedding work is performed and the developer is told nothing changed for the indexed plugin.
2. **Given** a catalog whose enabled plugin has upstream skill changes, **When** the developer refreshes the catalog, **Then** only skills whose declared content changed are re-embedded, and the developer is shown a summary of additions, modifications, and removals.
3. **Given** a catalog whose enabled plugin has been removed upstream, **When** the developer refreshes the catalog, **Then** that plugin is automatically disabled, its skill records are removed from the index, and the developer is loudly informed.
4. **Given** an enabled plugin, **When** the developer explicitly requests a reindex, **Then** by default only changed skills are re-embedded, and a force flag rebuilds every skill regardless.
5. **Given** an enabled plugin, **When** the developer requests a reindex scoped to a single catalog or plugin, **Then** only that scope is touched.
6. **Given** the developer changes which embedding model produced the stored vectors (for example, by upgrading Tome), **When** they next run a query, **Then** the command refuses to run with a clear message instructing them to re-run a force reindex; query never silently mixes vectors from different models.

---

### User Story 6 — Verify that the local installation is healthy (Priority: P3)

A developer is debugging odd query results, or has just upgraded Tome, or is preparing to file a bug report. They want a single command that says whether their local state is internally consistent and what the configured models are.

**Why this priority**: Operability of a tool that ships a binary database and downloaded models depends on being able to introspect them. The closed-error-set principle puts every failure mode behind a dedicated exit code; a status command makes the remediation surface explicit instead of leaving it implicit in error messages alone.

**Independent Test**: A clean install reports every subsystem as healthy. Removing one model file makes the next status report flag the model as missing and the command exit non-zero. The version output identifies the embedder and reranker names and versions in both clean and degraded states.

**Acceptance Scenarios**:

1. **Given** a clean install with both models present and an empty or healthy index, **When** the developer runs the status command, **Then** every subsystem (index database integrity, models, model-version drift relative to stored vectors, schema-version) is reported as healthy and the command exits zero.
2. **Given** one of the model files has been removed or corrupted, **When** the developer runs the status command, **Then** the models subsystem is reported as unhealthy with a specific diagnosis and the command exits non-zero, while the other subsystems are reported independently.
3. **Given** the configured embedder name or version has changed since the stored vectors were produced, **When** the developer runs the status command, **Then** the drift subsystem reports the specific kind of drift and the command exits non-zero.
4. **Given** the developer needs to attach version information to a bug report, **When** they run the version command, **Then** the output identifies the Tome version, the configured embedder name and version, and the configured reranker name and version on a single screen.

---

### User Story 7 — Remove a catalog safely even when its plugins are enabled (Priority: P3)

The developer wants to drop a catalog they no longer trust or care about. Some of its plugins are enabled.

**Why this priority**: This is a guardrail on a destructive Phase 1 verb, not a new feature. It must be impossible for `catalog remove` to silently destroy data the developer is relying on.

**Independent Test**: Removing a catalog with at least one enabled plugin without a force flag refuses with an error that names the enabled plugins and points at the disable command. The same command with a force flag disables the catalog's plugins, removes their skill records from the index, and removes the catalog cleanly.

**Acceptance Scenarios**:

1. **Given** a catalog with at least one enabled plugin, **When** the developer runs the catalog remove command without the force flag, **Then** the command refuses with an error listing each enabled plugin and pointing at the disable command.
2. **Given** a catalog with at least one enabled plugin, **When** the developer runs the catalog remove command with the force flag, **Then** every plugin in that catalog is disabled, its skill rows are removed from the index, and the catalog itself is removed; the developer sees a summary of what was cleaned up.
3. **Given** a catalog with no enabled plugins, **When** the developer runs the catalog remove command, **Then** the command behaves identically to its Phase 1 behaviour.

---

### Edge Cases

- **Missing skill metadata fields**: a skill declares only its directory but neither a name nor a description in its metadata header. The system substitutes the directory name and the first chunk of body text respectively, logs a warning for each substitution, and indexes the skill rather than refusing.
- **Cross-catalog skill name collision**: two enabled plugins from different catalogs both ship a skill with the same logical name. Both appear in query results as distinct rows; the catalog and plugin columns disambiguate. Neither is hidden.
- **First-run model download in a connected terminal**: the developer enables a plugin without having explicitly downloaded models. The system prompts to download (with a default of "yes"), shows a size estimate, and proceeds with progress on confirmation.
- **First-run model download in a non-interactive context**: same situation but with no terminal. The system exits with a dedicated non-zero status and a message pointing at the explicit download command. It does not silently download in the background.
- **Interrupted model download**: the developer presses the interrupt key part-way through a download. The partial file is discarded, the command exits with a clear status, and the next attempt starts cleanly.
- **Interrupted embedding pipeline**: the developer interrupts an enable mid-embedding. The plugin is left in a disabled state in the index. No half-indexed rows remain. A subsequent enable starts fresh.
- **Embedding model version drift**: the developer's stored vectors were produced by a different model version than the one currently loaded. Query refuses with a clear instruction to run a force reindex; no operation silently mixes vectors across model versions.
- **Schema version drift**: the developer is running a newer Tome against an index produced by an older Tome (or vice versa) with a different schema. The tool refuses to run with a clear error rather than corrupting the existing index.
- **Empty enable**: the developer enables a plugin that declares zero skills. The plugin is recorded as enabled, the indexed-skill count is zero, and a query that filters by that plugin returns an empty result without error.
- **Repeated enable**: the developer enables an already-enabled plugin. The command is a no-op and exits with a dedicated non-zero status to make automation idempotent.
- **Repeated disable**: same situation but for an already-disabled plugin. Same dedicated non-zero status.
- **Catalog refresh removes a plugin**: an enabled plugin disappears upstream. The plugin is auto-disabled, its index rows are dropped, and the developer is loudly notified.
- **Output redirected to a file**: the developer pipes a query to a file. Colour and progress are suppressed; tables remain readable plain text; structured output is byte-exact regardless of terminal.
- **Disable in a non-interactive context without the force flag**: the command exits with a clear error rather than hanging on a prompt.
- **Disk-full during embedding or download**: an operation that exhausts disk space mid-write is reported with a clear error; on the next invocation no half-written database state, no orphan vectors, and no partial model files are present.
- **Malformed plugin manifest**: a plugin's manifest exists but is syntactically invalid or missing its required identity fields. The plugin is reported as unindexable, the enable fails with a dedicated error naming the plugin and the manifest file, and the rest of the catalog continues to function unaffected.
- **Malformed skill metadata header**: a single skill's metadata header is present but syntactically invalid. That skill is skipped with a warning naming the file; the rest of the plugin's skills are indexed normally.
- **Reranker drift only**: the configured reranker has changed but the embedder has not. Queries continue to serve (stored vectors remain valid) but the developer is told that reranker drift was detected and asked to re-run a reindex when convenient.
- **Concurrent invocations**: two Tome processes run at the same time. A reader during a writer completes successfully; a second writer waits up to the documented timeout then exits with a dedicated database-busy status.
- **Schema-version drift**: the developer is running an older Tome against a database produced by a newer Tome. The tool refuses to operate with a clear error pointing the developer at upgrading Tome; it never tries to interpret an unknown schema.

## Requirements *(mandatory)*

### Functional Requirements

**Plugin identity and lifecycle**

- **FR-001**: The system MUST address every plugin by a two-part identifier of catalog and plugin name; the same plugin name in two different catalogs MUST produce two separately addressable plugins.
- **FR-002**: The system MUST distinguish *installed* (the plugin's files exist on disk, as a side-effect of registering its catalog) from *enabled* (the developer has opted in and the plugin's skills are present in the search index).
- **FR-003**: The system MUST persist each plugin's enabled state across invocations.
- **FR-004**: The system MUST treat a plugin enable as: parse the plugin's skill metadata, embed each skill's text, and insert or update one record per skill in the search index marked enabled. Enable MUST be atomic at the plugin granularity: either all of a plugin's skills are present in the index and marked enabled, or none of them are, regardless of when the operation was interrupted. If any single skill in a batch fails to embed (for example, the underlying inference engine refuses a pathological input), the entire enable for that plugin MUST roll back and the plugin MUST remain disabled.
- **FR-005**: The system MUST treat a plugin disable as: mark every one of its skill records as not enabled; the records and their embeddings remain on disk so re-enabling is cheap.
- **FR-006**: The system MUST treat re-enable of a plugin whose declared skill content has not changed since last enable as a state flip with no re-embedding.
- **FR-007**: The system MUST require explicit confirmation before disabling any enabled plugin, with a force flag to bypass for scripting and for non-interactive contexts.
- **FR-008**: The system MUST exit with a dedicated non-zero status when the developer asks to enable an already-enabled plugin, or disable an already-disabled plugin.

**Skill metadata extraction**

- **FR-009**: For each enabled plugin, the system MUST locate every skill declaration file the plugin ships and extract a structured metadata header from each.
- **FR-010**: From each skill metadata header the system MUST take the skill's name and description fields; other fields MUST be ignored in this phase.
- **FR-011**: If the skill metadata header omits the name, the system MUST substitute the skill directory name and MUST log a warning naming the file.
- **FR-012**: If the skill metadata header omits or empties the description, the system MUST substitute a bounded prefix of the skill body text (in particular, up to 500 characters following the metadata header) and MUST log a warning naming the file.
- **FR-013**: The system MUST identify each indexed skill by the triple (catalog, plugin, name); two enabled plugins from different catalogs MAY both ship a skill of the same logical name and both MUST appear as distinct, both-returnable index records. The plugin's version is recorded against each skill record but MUST NOT be part of the skill's identity — only one version of a given skill can be indexed at a time per (catalog, plugin) pair.
- **FR-013a**: Declarative inputs owned by Tome (model registration files, index metadata records, configuration) MUST be parsed strictly: unknown fields MUST cause the parse to fail with a clear error. Declarative inputs authored by third parties (plugin manifests, skill metadata headers) MUST be parsed leniently: unknown fields MUST be ignored without warning so that forward-compatible additions to the upstream plugin format do not break Tome.
- **FR-013b**: When a plugin's manifest exists but is malformed (invalid syntax, missing required identity fields), the system MUST treat that plugin as unindexable: enable MUST fail with a dedicated error naming the plugin and the manifest file; existing index records for that plugin (if any) MUST be left untouched. A subsequent catalog refresh that fixes the manifest MUST recover normally.
- **FR-013c**: When a skill's metadata header is present but malformed (invalid syntax inside the header, as distinct from missing fields covered by FR-011/FR-012), the system MUST skip that single skill, log a warning naming the file, and continue indexing the rest of the plugin's skills. A malformed header MUST NOT cause the entire plugin enable to fail.

**Embedding pipeline**

- **FR-014**: The system MUST compute a vector embedding for each indexed skill from a deterministic textual composition consisting of the skill's name and description, separated by a blank line; no other content MUST contribute to the embedding text.
- **FR-015**: The system MUST use a single, fixed embedding model for the entirety of an installation and MUST record both the embedder name and the embedder version against each stored vector's source of truth in index metadata. The reranker name and version MUST also be recorded.
- **FR-016**: The system MUST distinguish three drift cases against the index metadata: (a) embedder name change, (b) embedder version change, and (c) reranker name-or-version change. For (a) and (b) the system MUST refuse to serve queries until the developer rebuilds the index with a force-reindex; the refusal MUST identify which drift case was detected. For (c) the system MUST continue to serve queries (no stored data is invalidated by a reranker swap) but MUST report the drift on the next query and the next status command.
- **FR-017**: The system MUST NOT mix vectors from different embedding models in a single result set under any circumstance.
- **FR-018**: The system MUST run a reranking pass over the top embedding-similarity candidates by default, and MUST expose a flag to disable reranking for debugging or benchmarking. When reranking is disabled on a query, the human output MUST visibly indicate that the displayed scores are pre-rerank similarity, to avoid confusion against the production scoring scale.

**Models**

- **FR-019**: The system MUST provide an explicit command to download the embedding model and the reranking model.
- **FR-020**: The system MUST verify each downloaded model file against a recorded cryptographic checksum and MUST refuse to record a model as installed if verification fails. Checksum mismatch (download completed but the digest does not match the published value) MUST be reported as a distinct failure from "missing" (no file on disk) and from "corrupt" (file on disk but unreadable / truncated): each has a separate enumerated error and a separate exit status.
- **FR-020a**: A model MUST be considered installed only when its file content, checksum verification result, and registration entry are all consistent on disk. An interrupted download MUST leave the system in the same observable state as before the download started: no partial file MUST be visible as a model, and no orphaned registration entry MUST remain. The next invocation MUST be able to retry the download from a clean baseline.
- **FR-021**: The system MUST store models in a per-user data location chosen so that operating-system cache-cleanup processes do not silently delete them. The system MUST reuse Phase 1's path-resolution logic for per-user data locations rather than introducing a parallel path-resolution rule.
- **FR-022**: The system MUST provide a command to list installed models with their name, version, on-disk size, on-disk location, and a status of ok, missing, corrupt, or checksum-mismatched.
- **FR-023**: The system MUST provide a command to remove an installed model, with confirmation required by default and a force flag to bypass. Removing the embedder MUST be permitted even when stored vectors exist; the next query MUST then prompt to re-download (or refuse, per FR-025) rather than failing obscurely.
- **FR-024**: When an operation requires a missing model and the developer is at a connected terminal, the system MUST prompt to download it (defaulting to yes) and proceed on confirmation.
- **FR-025**: When an operation requires a missing model and the developer is not at a connected terminal, the system MUST exit with a dedicated non-zero status and a message pointing at the explicit download command.

**Querying**

- **FR-026**: The system MUST provide a query command that accepts a free-form text query and returns a ranked list of relevant skills.
- **FR-027**: The query command MUST accept a top-N cap with a documented default of ten results and MUST honour catalog and plugin filters.
- **FR-028**: The query command MUST only return records flagged as enabled.
- **FR-029**: The query command MUST present a table with columns for catalog, plugin, skill name, plugin version (the version of the plugin that the skill was indexed against), score, and skill on-disk path; in structured output, the same fields MUST appear as record fields.
- **FR-030**: When the reranker ran, the score MUST reflect the reranker output; when reranking was disabled, the score MUST reflect raw embedding similarity; the response MUST indicate which scoring was used.
- **FR-031**: By default the query command MUST return its best candidates even if their scores are low; an opt-in strict mode MUST exit non-zero when no result clears a configurable quality threshold. The threshold MUST be settable on the command line and MUST have a documented default; the threshold MUST be evaluated against the active scoring stage (the reranker if it ran, otherwise embedding similarity).

**Index maintenance**

- **FR-032**: The system MUST detect, on catalog refresh, which enabled plugins have had any of their skills' declared content change, and MUST re-embed only the changed skills.
- **FR-033**: When a catalog refresh discovers that an enabled plugin has been removed upstream, the system MUST automatically disable the plugin, drop its skill records, and notify the developer prominently.
- **FR-034**: The system MUST provide an explicit reindex command scoped to all enabled content, a single catalog, or a single plugin.
- **FR-035**: The explicit reindex MUST skip unchanged skills by default and MUST re-embed every skill in scope when a force flag is passed.
- **FR-036**: The catalog remove command MUST refuse to remove a catalog that has any enabled plugins, listing them and pointing at the disable command; a force flag MUST cascade — disabling every plugin in the catalog, dropping their skill records, and removing the catalog.

**Index storage and integrity**

- **FR-037**: The system MUST store all index state in a single per-user database file under the per-user data location.
- **FR-038**: The system MUST embed the index database engine and its vector-search capability into the Tome binary; the developer MUST NOT be required to install any system database or extension.
- **FR-039**: The system MUST record an index schema version in the database itself. When the running tool's schema version is newer than the on-disk index's schema version, the system MUST apply forward migrations automatically inside the same atomic-write discipline that protects ordinary writes, and MUST notify the developer that a migration occurred. When the on-disk schema version is newer than the running tool's, the system MUST refuse to operate with a clear error pointing the developer at upgrading Tome.
- **FR-040**: The system MUST define a concurrency model for the index database: read operations MUST be serve-able concurrently with an in-progress write; at most one writer MUST be permitted at a time; a second concurrent writer MUST wait for a bounded period (the wait timeout MUST be documented) and, if that period elapses, MUST exit with a dedicated non-zero status rather than block indefinitely. Concurrent invocations MUST NOT corrupt the database under any timing.

**Presentation**

- **FR-041**: Every output that has both a human form and a structured form MUST emit a structured form on request via the global flag inherited from Phase 1; structured output MUST be byte-stable across terminal and non-terminal contexts.
- **FR-042**: Long-running operations — defined as those expected to take more than approximately two seconds in the common case, including model download, batch embedding, and reindex — MUST show progress information to a connected terminal and MUST suppress that progress when output is redirected.
- **FR-043**: Operations whose duration is non-trivial but not measurable in advance (git operations, model loading, database initialisation) MUST show a working indicator to a connected terminal and MUST suppress it when output is redirected.
- **FR-044**: Listings (catalogs, plugins, models, query results) MUST render as readable tables on a connected terminal; the same data MUST be available in the structured form regardless.
- **FR-045**: The system MUST honour the standard environment variable that disables colour, and a global flag that disables colour, while keeping the rest of the rich presentation intact.
- **FR-046**: The system MUST suppress colour automatically when the standard output stream is not connected to a terminal, and MUST suppress progress and spinners automatically when the standard error stream is not connected to a terminal.

**Errors and exit codes**

- **FR-047**: All Phase 1 exit codes and error types MUST continue to behave as in Phase 1.
- **FR-048**: The system MUST extend its closed set of error types and exit codes to cover, at minimum, the following new failure modes, each with its own enumerated case and its own dedicated exit status:
  - plugin not found by identifier;
  - plugin already in the requested state (no-op enable / no-op disable);
  - model file missing;
  - model file corrupt (on disk but unreadable / truncated);
  - model checksum mismatch (downloaded successfully but digest disagrees with the published value);
  - inference-runtime initialisation failure (the embedder or reranker engine refuses to start);
  - vector-search engine initialisation failure (the database engine started but the vector-search capability could not be loaded);
  - embedding generation failure (the embedder ran but returned an error for a specific input);
  - reranking failure (the reranker ran but returned an error), distinct from embedding generation failure;
  - skill metadata header parse failure (the header is present but syntactically invalid), distinct from FR-011/FR-012 missing-field fallbacks;
  - plugin manifest parse failure (the plugin manifest is present but syntactically invalid or missing required identity fields);
  - model registration metadata parse failure (a model's recorded metadata is unreadable);
  - query strict-mode no-results (no candidate cleared the configured threshold);
  - index schema-version mismatch (on-disk schema is newer than the running tool's);
  - embedder name drift detected against stored vectors;
  - embedder version drift detected against stored vectors;
  - index database busy (a writer held the index past the configured wait timeout);
  - index database integrity-check failure (the database engine reports the file as corrupt).
- **FR-049**: The closed-error-set principle from Phase 1 MUST continue to hold: there MUST NOT be a generic "other" error variant; every new failure mode MUST have its own enumerated case and its own exit status. Future Phase-2 patches that introduce additional failure modes MUST extend the enumeration rather than collapse new cases into existing ones.

**Interactive flow**

- **FR-050**: The system MUST provide an interactive entry point reached by invoking the plugin command with no subcommand. The flow MUST step through catalog selection, plugin selection within a catalog, plugin inspection, and enable/disable action, with an escape option at every level.
- **FR-051**: The interactive flow MUST refuse to run without a connected terminal and MUST point the developer at the non-interactive subcommands instead.
- **FR-052**: Every interactive action MUST be reachable as a non-interactive subcommand of equivalent power.

**Cancellation and signal handling**

- **FR-053**: The interruption signal handling established in Phase 1 (exit with the dedicated interrupted status) MUST continue to apply to every Phase 2 operation. Cancellation MUST be honoured at every skill boundary in the embedding pipeline and at every file boundary in the model download; an in-flight inference for a single skill or a single network read MAY complete before the system shuts down, but the system MUST exit with the interrupted status within a bounded number of seconds of the signal.
- **FR-054**: An interrupted enable, reindex, or model download MUST leave the system in a state observationally equivalent to its pre-operation state. No half-indexed skills, no orphaned model files, and no stale registrations are permitted on the next invocation.

**Self-inspection**

- **FR-055**: The system's version output MUST identify, in addition to the Tome version itself, the configured embedder name and version and the configured reranker name and version, so that bug reports and reproducibility checks have unambiguous model provenance.
- **FR-056**: The system MUST provide a status command (separate from the Phase 1 catalog-listing surface) that reports the health of each Phase 2 subsystem independently: index database integrity, model file presence and checksum status, model-version drift relative to stored vectors, and schema-version status. Each subsystem MUST be reported as healthy or unhealthy with a specific diagnosis, and the command MUST exit non-zero if any subsystem is unhealthy. Output MUST be available in both human and structured forms.

### Non-Functional Requirements

- **NFR-001**: The stripped release binary, including the new database engine, the new vector-search capability, and the new inference runtime, MUST stay within the binary-size cap defined in the constitution. If the inference runtime, the embedded database, and the vector-search engine together threaten that cap, the plan MUST revise its component choices rather than waive the cap. The plan MUST document, per component: whether linkage is static or dynamic, which optional execution providers or features are enabled, and any link-time optimisations applied. Model files are downloaded at runtime and MUST NOT be counted against the binary cap.
- **NFR-002**: Each new third-party dependency added in Phase 2 MUST carry a written justification per the constitution. The plan MUST enumerate every new direct and significant transitive dependency together with its licence, and MUST confirm each licence is within the constitution's allowlist. Downloaded model files MUST also have their licences recorded and surfaced to the developer (in the model-list output and in documentation).
- **NFR-003**: Sensitive material that could appear in upstream service URLs MUST continue to be scrubbed before reaching logs or error output, exactly as in Phase 1. The same scrubbing boundary MUST apply to any new code paths that surface Git output, upstream model download URLs, or error chains derived from such operations.
- **NFR-004**: Writes to the index database and to the models directory MUST be atomic from the developer's perspective. For the index database, this means a write is observable either in full or not at all; a crash or interrupt MUST NOT leave the database file in a state the engine refuses to open. For the models directory, this means a partial download, a failed checksum verification, or an interrupted registration MUST leave no on-disk artefacts that future invocations interpret as "installed."
- **NFR-005**: All Phase 1 quality gates (formatting, lint, typo, security audit, dependency policy, conventional commits) MUST continue to apply to Phase 2 code.

### Key Entities

- **Plugin record**: an entry tracked by Tome representing a plugin known to be installed on disk (because its catalog is registered). Carries its catalog, plugin name, current enabled state, version, the on-disk path of its files, and the time it was last indexed.
- **Skill record**: an entry in the index database representing one indexed skill. Carries the catalog, plugin, skill name, the description used for embedding, the plugin version at index time, the on-disk path of the skill file, a content fingerprint of the source, an enabled flag, and the time it was indexed.
- **Skill embedding**: the vector representation of a skill record's embedding text, linked one-to-one with the skill record.
- **Model registration**: an installed embedding or reranker model, identified by name, version, source location it was downloaded from, expected cryptographic checksum, and on-disk size.
- **Index metadata**: per-installation state recording the index schema version, the embedding model name and version that produced the stored vectors, and the reranker model name and version.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer who has registered one catalog of around ten plugins and ten skills can, on a recent laptop, enable a plugin and see its skills available for query within ten seconds from confirming the enable.
- **SC-002**: After enabling a representative plugin, a free-form query of natural-language length on a representative topic returns the most relevant skill in the top three results.
- **SC-003**: From a clean install, the explicit model download command completes within the expected time for the size of the models and a typical home-internet connection, and reports a verified-good status for both models on completion.
- **SC-004**: Re-enabling a plugin whose source content has not changed completes in less than one second on a recent laptop.
- **SC-005**: A refresh of a catalog whose enabled plugin has had one of its skills modified upstream re-embeds only that single skill and reports the change in its summary.
- **SC-006**: Attempting to remove a catalog with at least one enabled plugin always either refuses (without force) or cleans up the index, the plugin records, and the catalog (with force) — never producing a half-removed state.
- **SC-007**: A query piped to a file or another process produces deterministic, terminal-independent output: no escape sequences, no progress, no spinner glyphs.
- **SC-008**: When the index has been built with one embedding model and the configured model changes, a query refuses to run and points the developer at the recovery command rather than returning mixed-model results.
- **SC-009**: When a developer enables a plugin without a connected terminal and the embedding models are missing, the command exits with a dedicated non-zero status and a message pointing at the model download command, never blocking on a prompt.
- **SC-010**: All Phase 1 success criteria continue to hold; the Phase 2 surface does not regress catalog management.
- **SC-011**: Every Phase 2 command honours the global structured-output flag and produces parseable, byte-stable structured output regardless of terminal context.
- **SC-012**: An interrupted embedding pipeline or model download leaves the system in a clean state on the next invocation: no half-indexed skills appear in query results and no partial model files appear as installed.
- **SC-013**: The version output and the status command together let a developer reading a bug report reconstruct, without running the failing operation, which Tome version, which embedder version, and which reranker version produced the reported behaviour.
- **SC-014**: Two Tome processes running at the same time never leave the index database in a state that future invocations refuse to open; the second writer fails fast with a recognisable status rather than waiting indefinitely.
- **SC-015**: A forward schema migration from an older Tome's on-disk state to a newer Tome completes inside a single atomic boundary: a crash during migration leaves either the old schema (intact) or the new schema (intact), never an intermediate state.

## Assumptions

- Phase 1 catalog management and the Phase 1 plugin manifest format are in place and unchanged by Phase 2. Phase 2 reads them; it does not modify them.
- The plugin manifest is the existing Claude Code plugin manifest format unchanged; Tome does not introduce a Tome-specific manifest in Phase 2.
- Only skill declarations are indexed in Phase 2. Other plugin components (agents, commands, hooks, embedded service declarations) are counted in the plugin view but never embedded.
- A single global index database is sufficient. Project-scoped indexes (workspaces) are explicitly deferred to a later phase.
- Cross-harness installation (writing into other agentic coding assistants' install directories) is explicitly deferred to a later phase. The Phase 2 index is internal to Tome.
- Plugin authoring tools (scaffolding, lint, new) are explicitly deferred to a later phase.
- The reranker is on by default. The off-by-default flag exists only for debugging and benchmarking.
- The embedding model is fixed for the duration of an installation. A future change of model is allowed but requires the developer to run an explicit force reindex; the tool refuses to query until they do.
- Models are not bundled in the binary; they are downloaded on first need. The binary-size cap from the constitution remains in force on the binary itself.
- Phase 2 adds no async runtime; the synchronous architecture from Phase 1 continues to apply.

## Dependencies on Phase 1

- The catalog registry, the catalog cache layout, and the per-catalog Git operations are reused unchanged.
- The global structured-output flag and Phase 1's TTY-detection behaviour are reused unchanged.
- The closed `TomeError` enum is extended with the Phase 2 cases; Phase 1 cases are not modified.
- The Phase 1 atomic-write discipline (registry mutations, per-catalog cache mutations) is extended to cover the new mutations introduced in Phase 2 (index database, models directory).
- The Phase 1 credential-scrubbing boundary continues to apply to any new code paths that may surface Git output.

## Out of Scope (Phase 2)

- MCP server exposing search / get-skill tools.
- Workspace awareness (project-scoped catalogs or indexes).
- Cross-harness installation — writing or symlinking enabled plugin content into the install directories of other agentic coding assistants.
- Translation of commands, agents, or hooks across harnesses.
- Indexing non-skill plugin components.
- Hybrid lexical-plus-semantic search; Phase 2 is pure semantic with reranking only.
- Multiple embedding backends or pluggable model selection.
- Plugin authoring tools (scaffolding, lint, new).
- Tome-initiated edits to plugin source files; Tome only reads.
