# Feature Specification: Phase 5 — Commands as Prompts, Unified Entries, and Variable Substitution

**Feature Branch**: `005-phase-5-commands-prompts`
**Created**: 2026-05-26
**Status**: Draft
**Input**: User description: "Phase 5 — treat plugin commands as first-class entries alongside skills with kind discriminator. Expose user-invocable entries as MCP prompts. Add a new get_skill_info middle-tier metadata tool. Implement portable variable substitution covering Tome-namespaced built-ins, environment variable passthrough with default values, and Claude Code-compatible argument substitution. Per PRDs/phase-5.md."

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details.

**Source PRD**: [PRDs/phase-5.md](../../PRDs/phase-5.md) — the implementation-level brief is authoritative for HOW; this spec is authoritative for WHAT.

## Overview

Phase 4 finished the central architecture refactor and put one command between a developer's project and a fully-configured cross-harness MCP integration. With that integration in place, every agent connected through Tome can already discover and read the skills enabled in a workspace. Phase 5 builds on that foundation in three ways.

First, it widens what counts as an "entry". Plugin commands — the markdown-with-frontmatter files Claude Code installs under a plugin's `commands/` directory — share their entire structural anatomy with skills (markdown body plus YAML frontmatter), differ only in their default invocation pattern, and are indistinguishable in every meaningful sense once you stop caring about which directory they came from. Phase 5 collapses that distinction: commands and skills become two kinds of one underlying concept, indexed together, searched together, governed by the same per-entry frontmatter flags. Skills retain their default behaviour (searchable by the agent, not slash-invocable by the user); commands gain agent-searchability on top of their default slash-invokability. Plugin authors can override either default per entry.

Second, it adds a slash-invocation surface. The MCP protocol's `prompts` capability is the right place for user-triggered, parametrisable templates — exactly the role plugin commands serve in their native environment. Phase 5 exposes every entry marked user-invocable as an MCP prompt, with an argument schema derived from the entry's frontmatter and a body rendered through the new substitution engine on each call. A harness that supports MCP prompts (Claude Code and most others; Codex still pending upstream support) shows them in its slash-command menu. Commands are no longer second-class citizens accessible only through Tome's search tool — they are first-class user-invocable surfaces alongside the agent-callable search and read tools.

Third, it makes entry content portable. Skills authored against one project's filesystem layout, one user's environment variables, or one harness's quirks have historically been hard to share without retouching. Phase 5 ships three substitution layers that run on every entry body before it leaves Tome: a Tome-namespaced set of built-ins (paths, names, plugin and workspace data directories, date and timestamp) that resolve against the current install and active workspace; an explicit user-controlled environment-variable passthrough (`${TOME_ENV_*}` only — no accidental exfiltration of `${GITHUB_TOKEN}` or `${AWS_SECRET_ACCESS_KEY}`) with default-value syntax for missing variables; and Claude Code-compatible argument substitution covering positional, indexed, and named forms, with the "ARGUMENTS:" append fallback. Built-ins and env passthrough run on every retrieval; argument substitution runs whenever the caller passes args. The agent calling the read tool without args still receives a body whose path references are fully resolved.

A new middle-tier MCP discovery tool sits between the existing search and read tools. Search returns a small ranked list with truncated descriptions, cheap. Read returns the entire rendered body, expensive when the entry's directory is heavy with reference material. The new tool returns full metadata, the `when_to_use` hint, and a one-level enumeration of the entry's resource directory — enough for the agent to decide whether the body is worth fetching, without ever paying for the body itself. The `when_to_use` frontmatter field, previously read but never indexed, now contributes to the embedding text so that the disambiguation hint plugin authors already write also improves retrieval ranking.

What this phase explicitly does not do: translate Claude Code agents (subagents) to other harnesses, which has no clean cross-harness mapping and is likely a permanent non-goal; translate hooks, which is Phase 6's central problem; execute the server-side shell-exec syntax (``` !`cmd` ```) that Claude Code commands sometimes use to inject dynamic context, which is also Phase 6; add new harnesses beyond the five Phase 4 supports; ship plugin authoring tools; or introduce a full templating-engine dependency — Phase 5's hand-rolled substitution covers its own needs without the dependency weight and learning surface a full templating engine would bring.

"MCP" throughout this spec refers to the Model Context Protocol — the standard Tome's server speaks to agentic coding harnesses. Phase 3 introduced Tome's MCP server; Phase 5 extends what that server exposes.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Invoke a plugin command as a slash command in a harness (Priority: P1)

A developer has a workspace with one or more plugins enabled. At least one of those plugins ships a `commands/` directory — for example, a `fix-issue.md` that takes a GitHub issue number and tells the agent how to triage it. The developer wants to invoke that command from inside their harness exactly the way a user of the plugin's native Claude Code installation would: type a slash, pick the command from the menu, and have the rendered command body land in the conversation as their own next message.

**Why this priority**: This is the headline Phase 5 deliverable. Without it, plugin commands stay invisible to anyone using a non-Claude-Code harness, which defeats the cross-harness premise of Tome. Every other Phase 5 surface — the unified entry concept, the per-entry invocation flags, the variable substitution layer, the argument-schema derivation — exists to make this one user experience work end-to-end.

**Independent Test**: From a fresh install on a machine with a harness that supports MCP prompts, the developer creates a workspace, enrols a catalog containing a plugin with both a `skills/` directory and a `commands/` directory, binds a project to the workspace, and launches the harness in that project. The harness's slash-command menu shows entries for the plugin's commands (qualified by plugin to avoid collisions). Selecting one of those entries — with or without arguments — injects the entry's rendered body into the conversation. The plugin's skills, by contrast, do not appear in the slash menu by default; they remain accessible through the agent-callable search and read tools.

**Acceptance Scenarios**:

1. **Given** a workspace has an enabled plugin whose source tree contains both a `skills/` directory and a `commands/` directory, **When** Tome indexes the plugin, **Then** entries from both directories are present in the index with a kind marker distinguishing them; both kinds appear in the agent's search results by default; only command-kind entries appear in the harness's prompt list by default.
2. **Given** a command entry is exposed as an MCP prompt, **When** the harness's user invokes it from the slash menu without arguments, **Then** the harness receives the entry's rendered body as a single user message; the body has had built-in and environment-variable substitution applied; the body has not had any agent-style argument substitution applied; and no plugin-author-declared arguments are required for the invocation to succeed.
3. **Given** a command's frontmatter declares the entry not user-invocable, **When** Tome indexes the plugin, **Then** the entry is present in the agent's search results (assuming it is also not declared model-invisible) but does not appear in any harness's prompt list.
4. **Given** a skill's frontmatter declares the entry user-invocable, **When** Tome indexes the plugin, **Then** the entry appears in the harness's prompt list alongside the plugin's commands and remains discoverable through the agent's search tool.
5. **Given** a plugin ships two entries (of any kind) whose generated prompt names collide after sanitisation, **When** Tome lists the prompt surface, **Then** the entry indexed first receives the unsuffixed name and subsequent entries receive numeric counter suffixes; each collision is recorded so the plugin author can resolve the ambiguity by adding the explicit name-override frontmatter field.
6. **Given** an entry declares an explicit prompt-name override in its frontmatter, **When** Tome generates that entry's prompt identity, **Then** the override replaces both the plugin-prefix portion and the entry-name portion of the generated prompt name, still subject to sanitisation and length limits.
7. **Given** a plugin's command file is renamed or removed upstream, **When** Tome reindexes the plugin, **Then** the prompt list reflects the new contents of the plugin tree on the next harness session.

---

### User Story 2 — Reference paths, environment variables, and workspace state from inside an entry body (Priority: P2)

A skill author maintains a plugin that ships supporting scripts under the skill's `scripts/` subdirectory and accepts caller-configured values (a deployment target, a harness identifier, a project root override) through environment variables wired in the user's MCP config. They want to write the skill body once, deploy it across machines and harnesses with different layouts and conventions, and have it just work without per-deployment forks. They also want a place to write persistent files — per-plugin or per-workspace — that survives plugin reinstalls and is discoverable from the entry body without hardcoding paths.

**Why this priority**: Without portable substitution, skill authors either hardcode paths (breaks on the next machine), force users to maintain forks (defeats the catalog model), or restrict themselves to entirely self-contained markdown (rules out a large class of high-value skills that lean on shipped tooling). This story makes the difference between a one-developer convenience and a shareable plugin ecosystem.

**Independent Test**: From an install with at least one plugin enabled, the developer authors a skill body whose markdown contains references to the entry's own directory and a plugin-data directory, a passthrough environment variable with a default, and a plain Markdown reference to a non-Tome variable. They retrieve the skill through both the agent-callable read tool and through any harness surface that renders the entry as a user-invocable prompt. The retrieved body has all Tome-namespaced references substituted to absolute paths; the environment variable resolves to its caller-set value or its default if unset; the non-Tome variable passes through unchanged; the persistent plugin-data and workspace-data directories exist on disk after the first retrieval.

**Acceptance Scenarios**:

1. **Given** a skill body containing built-in references to the entry's own directory, the plugin's root directory, the plugin's persistent data directory, and the active workspace's data directory, **When** the body is retrieved through either the read tool or the prompts surface, **Then** every Tome-namespaced built-in resolves to an absolute path appropriate for the current installation; the data directories exist on disk after the substitution; the data directory paths are stable across reinstalls of the same plugin version.
2. **Given** a skill body containing an environment-variable reference in the user-controlled namespace, **When** the harness's MCP launcher has set that environment variable, **Then** the variable resolves to the caller-supplied value; **When** the harness has not set it but the reference includes a default, **Then** the default is substituted; **When** the harness has not set it and no default is supplied, **Then** an empty string is substituted and the omission is recorded for diagnostics.
3. **Given** a skill body contains a reference to a variable outside the Tome-controlled namespace (for example, a token-shaped environment variable the user has set for unrelated reasons), **When** the body is retrieved, **Then** the reference passes through unchanged and is never substituted with the host environment value.
4. **Given** a skill body contains harness-native shell-execution syntax (the backtick-prefixed-with-bang form Claude Code uses), **When** the body is retrieved through Tome, **Then** Tome leaves the syntax verbatim in the rendered output for downstream processing by the harness; Tome itself does not execute any shell command.
5. **Given** an entry body has no argument-style references and a caller has supplied arguments, **When** the body is rendered, **Then** the arguments are appended in a documented fallback footer so the agent always sees what the user actually passed.
6. **Given** two retrievals of the same entry within the same session, **When** the per-plugin data directory has been written to between the two retrievals, **Then** the second retrieval still resolves the same path and the directory contents are preserved.

---

### User Story 3 — Invoke a command with structured arguments (Priority: P3)

A plugin author ships a command whose body parametrises on a small set of values — a component name, a source framework, a target framework — and uses positional, indexed, and named substitution references in the body. They declare those argument names in the command's frontmatter. A user of any harness that supports MCP prompt arguments sees structured fields in their invocation UI, fills them in, and the rendered body lands in the conversation with each named placeholder replaced.

**Why this priority**: Argument-bearing commands are the dominant pattern in Claude Code's existing plugin ecosystem. A Phase 5 that exposed commands as prompts but treated arguments as a stretch goal would still leave the bulk of plugin content second-class. This story closes that gap and makes Phase 5's prompt surface compatible with existing plugin libraries without per-plugin rewrites.

**Independent Test**: From a plugin containing a command with two or more named arguments declared in frontmatter and references to those names in the body, the developer invokes the command from a harness with structured argument inputs. The harness UI offers a field per declared argument. With all fields filled in, the rendered body has every reference substituted; the all-positional reference is also populated with the concatenated values; an indexed reference (zero-based) returns the corresponding value. A second invocation with one field left blank still completes, with the missing reference rendered as an empty string. A command declaring no arguments still accepts a freeform input field whose label comes from the command's argument-hint frontmatter.

**Acceptance Scenarios**:

1. **Given** a command's frontmatter declares a list of named arguments, **When** the command is enumerated in the prompts list, **Then** the prompt's argument schema contains one required string entry per declared name in declaration order.
2. **Given** a command's frontmatter declares no named arguments, **When** the command is enumerated in the prompts list, **Then** the prompt's argument schema contains exactly one optional argument carrying the entry's argument-hint as its description (or a generic description when no hint is present).
3. **Given** a caller invokes a prompt with all declared arguments supplied as named values, **When** the body is rendered, **Then** named references in the body resolve to the supplied values; positional references resolve to the same values in the declared name order; the catch-all positional reference resolves to the values joined with single spaces.
4. **Given** a caller supplies a single string in place of an argument object, **When** the body is rendered for an entry that declares no named arguments, **Then** the string is preserved verbatim as a single positional value; **When** the entry does declare named arguments, **Then** the string is split on whitespace and the resulting tokens are zipped with the declared names.
5. **Given** an entry body contains no argument-style references and the caller has supplied any arguments, **When** the body is rendered, **Then** the documented append-fallback footer ensures the supplied arguments are surfaced to the agent.

---

### User Story 4 — Discover entries with a cheap mid-tier metadata fetch (Priority: P4)

An agent has just run a semantic search and received back a small ranked list of candidates with truncated descriptions. The candidates look plausible but the descriptions alone are not enough to decide which one is worth fetching the full body of — especially for entries with large supporting directories where the full read is expensive in tokens. The agent wants one more cheap call: full description, the `when_to_use` hint, the plugin version, the user-invocability flag, and a one-level enumeration of the entry's resource directory so it can see what kinds of supporting material the entry ships.

**Why this priority**: Without a middle tier, the agent's only options are to take a chance and fetch the body, or fall back to its own training knowledge. Both are wasteful: the first burns tokens on entries that turn out to be wrong; the second misses entries that would have been right. This story gives the agent a cheap disambiguation step and improves the overall quality of which entries get fetched.

**Independent Test**: From an install with at least one plugin enabled whose skill ships a heavy supporting directory (scripts, references, examples), the agent calls the new middle-tier tool naming the skill. The response contains the entry's untruncated description, its `when_to_use` text (when set), its plugin version, whether it is user-invocable, an absolute path to the entry's source file, and an enumeration of the entry's directory listing top-level files separately from directories of one-level-deep contents. Directories with many children are listed with a per-directory cap and a sentinel indicating how many more entries exist. Calling the middle-tier tool for a command (which has no associated directory) succeeds and omits the resource enumeration. Calling the middle-tier tool for the same entry name in two different kinds disambiguates by an optional kind parameter.

**Acceptance Scenarios**:

1. **Given** the agent calls the new middle-tier tool naming a skill, **When** the response is returned, **Then** the response contains the entry's full untruncated description, its `when_to_use` hint, its plugin version, its user-invocability flag, the absolute path to its source file, and an enumeration of the entry's directory contents listed under top-level files and per-subdirectory child paths.
2. **Given** any single subdirectory of the entry's resource tree contains more than a documented number of children, **When** the enumeration is built, **Then** the listing includes the first children alphabetically followed by a sentinel naming the count of omitted children.
3. **Given** the entry being looked up is a command rather than a skill, **When** the response is returned, **Then** the resource enumeration is omitted entirely and the rest of the response is populated.
4. **Given** the same entry name exists in both kinds for a single plugin, **When** the agent calls the middle-tier tool without a kind parameter, **Then** the response describes the skill-kind entry; **When** the agent supplies a kind parameter explicitly, **Then** the response describes the entry of that kind.
5. **Given** the agent calls the existing search tool, **When** results are returned, **Then** each result's description is truncated to a documented default character count, and the agent may override the truncation limit by a documented parameter.
6. **Given** an entry author has populated the `when_to_use` frontmatter field, **When** Tome indexes the entry, **Then** the `when_to_use` text contributes to the entry's embedded representation alongside name and description; queries semantically related to the `when_to_use` text retrieve the entry with higher rank than they would have absent the indexing.

---

### User Story 5 — Govern per-entry invocability through frontmatter (Priority: P5)

A plugin author has an opinion about how each of their entries should be surfaced. One entry is internal scaffolding that other entries reference; it should be indexed for completeness but never surfaced to the agent or the user. Another is a power-user shortcut they want exposed as a slash command but kept out of the agent's search results so it doesn't get auto-invoked. A third is a fully discoverable skill that they also want available as a slash command for direct invocation. They want to express all three intents through two frontmatter flags whose defaults match the entry's kind.

**Why this priority**: Without per-entry control, every command would always be both searchable and slash-invocable, and every skill would always be searchable but never slash-invocable. Plugin authors lose the ability to ship dormant entries (scaffolding referenced from other entries) or to hide power-user shortcuts from the agent's auto-discovery surface. This story closes that gap with a minimal-surface frontmatter knob set.

**Independent Test**: A plugin ships four entries that exhaustively cover the matrix of two boolean flags: (a) a command with default behaviour, (b) a command with model invocation disabled, (c) a skill with default behaviour, (d) a skill with user invocation enabled. After indexing, the agent's search results contain (a), (c), and (d); the harness's prompt list contains (a) and (d). A fifth entry that toggles both flags to make it neither searchable nor user-invocable is present in the index but absent from both surfaces.

**Acceptance Scenarios**:

1. **Given** an entry's frontmatter declares model invocation disabled, **When** the index is queried by the agent, **Then** the entry is excluded from the search results regardless of relevance to the query.
2. **Given** an entry's frontmatter declares user invocation enabled, **When** the prompts surface is enumerated, **Then** the entry appears in the prompts list with the appropriate argument schema derived from frontmatter.
3. **Given** an entry's frontmatter omits both flags, **When** the entry is indexed, **Then** the entry's behaviour matches the documented default for the entry's kind: skills are searchable and not user-invocable; commands are searchable and user-invocable.
4. **Given** an entry's frontmatter declares both flags so as to disable both surfaces, **When** the entry is indexed, **Then** the entry is present in the index but appears in neither the agent's search results nor the harness's prompt list.
5. **Given** a plugin's tree is enabled or disabled in a workspace, **When** the prompts surface is rebuilt for that workspace, **Then** the surface reflects the union of currently enabled plugins' user-invocable entries.

---

### Edge Cases

- A plugin ships two entries with the same name across kinds (one in `skills/`, one in `commands/`). They are stored as distinct rows and may each be surfaced through whichever surface their flags allow; the middle-tier tool disambiguates by an explicit kind parameter.
- An entry's frontmatter is malformed YAML or contains a forbidden value in a recognised field. The malformed entry is rejected loudly with a parse error naming the file and field; sibling entries in the same plugin continue to be indexed.
- A retrieval of an entry whose body references an environment variable in the Tome-controlled namespace happens before that environment variable has been wired in the harness's MCP launcher config. The substitution resolves to the documented default (when present) or to an empty string; the absence is logged for diagnostics but does not fail the retrieval.
- A plugin ships an entry whose generated prompt name exceeds the documented total-length budget. The plugin-prefix and entry-name portions are truncated independently to documented per-portion budgets; the truncation is logged but does not block exposure.
- A workspace's persistent plugin-data directory or workspace-data directory cannot be created on first reference (permission failure on the central state tree). The retrieval fails with a dedicated error code; the entry body is not partially substituted. The same error code MAY cover both directory classes, since the failure mode (write to the central state tree) is symmetric across them; the eventual contract documents the exact code.
- A harness that does not support MCP prompts (Codex, until upstream support lands) is configured against Tome. The agent surface continues to work; commands are still accessible to the agent through the search and read tools using the underlying kind discriminator; users do not get a slash-menu entry for those commands.
- An entry has substitution references that look syntactically Tome-namespaced but do not match any built-in. The references are left verbatim in the rendered output and the unrecognised reference is logged for diagnostics.
- An entry body has substitution references inside fenced code blocks. The substitution layer does not parse Markdown structure and substitutes references uniformly across the body; entry authors who want literal references in code samples must avoid the Tome namespace prefix or escape the references in their source content.
- The agent calls the read tool for an entry that exists in the index but whose source file has been removed from disk (a stale row remains because reindex has not yet run). The read returns the entry-not-found error code (the eventual contract pins the numeric value); the agent's next sensible recovery is to retry after a reindex or to call the search tool for fresh candidates.
- An agent passes arguments to the read tool for a command that declares no named arguments. The argument substitution layer applies the single-string positional convention; the documented append-fallback footer is added if no references in the body consumed the arguments.

## Requirements *(mandatory)*

### Functional Requirements

#### Unified entry concept and indexing

- **FR-001**: Tome MUST treat plugin commands as first-class indexable entries alongside skills, walking both the plugin's skills directory and its commands directory at index time and producing rows in a single underlying entry table.
- **FR-002**: Each indexed entry MUST carry a kind marker distinguishing skill-kind from command-kind entries; the combination of catalog, plugin, kind, and name MUST uniquely identify an entry.
- **FR-003**: A plugin shipping the same entry name across kinds (one skill and one command sharing a name) MUST produce two distinct rows; neither shadows the other.
- **FR-004**: The reindex pipeline MUST handle commands and skills identically — content-hash diffing, embedding refresh on hash change, removal of deleted entries, addition of new entries — without per-kind branching beyond initial classification.
- **FR-005**: When `when_to_use` is present in an entry's frontmatter, Tome MUST include it in the text used to compute the entry's embedded representation; entries without `when_to_use` MUST embed using the existing name-plus-description composition.
- **FR-006**: When `description` is absent from an entry's frontmatter, Tome MUST fall back to a documented prefix of the entry's body and record the fallback for diagnostics; when `name` is absent, Tome MUST fall back to the sanitised filename stem.
- **FR-007**: Tome MUST parse the following entry frontmatter fields: `name`, `description`, `when_to_use`, `arguments`, `argument-hint`, `disable-model-invocation`, `user-invocable`, and `prompt_name`. Each field is optional. Recognised fields with malformed values MUST be rejected loudly with a parse error naming the file and field; unrecognised frontmatter fields MUST be tolerated for forward-compatibility with upstream harnesses' additions (consistent with the existing third-party strictness boundary for plugin frontmatter). Boolean fields MUST accept the standard YAML truthy/falsy values; absence MUST resolve to the documented default.

#### Per-entry invocability

- **FR-010**: Tome MUST honour the frontmatter flag that disables model invocation by excluding the entry from the agent's search results regardless of query relevance.
- **FR-011**: Tome MUST honour the frontmatter flag that enables or disables user invocation by including or excluding the entry from the prompts surface.
- **FR-012**: The default value of the user-invocation flag MUST depend on entry kind: skills default to not user-invocable; commands default to user-invocable. Both defaults MUST be overridable per-entry through frontmatter.
- **FR-013**: An entry whose frontmatter disables both surfaces MUST be present in the underlying index but absent from both the agent's search results and the harness's prompts surface.

#### Variable substitution: built-ins

- **FR-020**: Tome MUST resolve a documented set of Tome-namespaced built-in variables on every entry retrieval regardless of whether arguments were supplied. The Tome namespace MUST be defined as any reference of the form `${TOME_<NAME>}` where `<NAME>` is a sequence of uppercase ASCII letters, digits, and underscores. The Phase 5 set MUST cover absolute paths to the entry file, the entry's source directory, the plugin's root directory, the plugin's persistent data directory, the active workspace's persistent data directory, the plugin name, the plugin version, the entry name, the catalog name, the workspace name, the current date, and the current ISO 8601 timestamp.
- **FR-021**: The two persistent data directories (per-plugin and per-workspace) MUST be created lazily on first substitution under Tome's central state tree (NOT under any bound project's marker directory). The per-plugin directory MUST be shared across workspaces, scoped by catalog and plugin identity. The per-workspace directory MUST be scoped by workspace name, catalog, and plugin identity. Subsequent substitutions MUST be idempotent and safe under concurrent retrieval. Both directories MUST persist across plugin version upgrades.
- **FR-022**: Built-in variables MUST support a default-value syntax for uniformity with environment-variable references; the unset-with-default case MUST never trigger for built-in variables under normal operation (they are always set by Tome itself).
- **FR-023**: Unrecognised references in the Tome namespace MUST pass through unchanged and the unrecognised reference MUST be recorded for diagnostics.
- **FR-024**: Catalog name and plugin name MUST be sanitised when used as path components in persistent data directories (replacing characters illegal under any Phase 4-supported OS with safe placeholders, preserving readability). The unsanitised values MUST be returned by the `${TOME_CATALOG_NAME}` and `${TOME_PLUGIN_NAME}` built-ins; sanitisation applies only to path-construction contexts. Sanitised path components MUST be stable across plugin version upgrades.
- **FR-025**: When a Phase 4 `tome workspace rename` runs against a workspace that has previously caused a `${TOME_WORKSPACE_DATA}` directory to be created, the central per-workspace data directory MUST be relocated atomically to match the new workspace name so that subsequent substitutions resolve to the same on-disk contents. Failure to relocate the directory MUST surface a dedicated error code rather than silently leaving a stranded directory.

#### Variable substitution: environment passthrough

- **FR-030**: Tome MUST resolve environment-variable references in a user-controlled namespace by reading the corresponding host environment variable using the literal prefixed name. The user-controlled namespace MUST be defined as any reference of the form `${TOME_ENV_<NAME>}` where `<NAME>` is a sequence of uppercase ASCII letters, digits, and underscores. Tome reads the host environment variable named literally `TOME_ENV_<NAME>` (the prefix is preserved when looking up the host environment).
- **FR-031**: Tome MUST support a default-value syntax of the documented form `${TOME_ENV_<NAME>:-default}` for environment-variable references; the default MUST be substituted when the host variable is unset.
- **FR-032**: A reference in the user-controlled environment namespace whose host variable is unset and whose reference includes no default MUST resolve to an empty string; the omission MUST be recorded for diagnostics.
- **FR-033**: Tome MUST NOT substitute environment-variable references outside the documented user-controlled namespace; in particular, references that look syntactically like secrets or like other tools' environment-variable conventions MUST pass through unchanged regardless of whether the host environment has them set. References whose name lies in the Tome namespace but outside the user-controlled subnamespace are governed by FR-020 and FR-023; references in neither namespace are governed by FR-052.

#### Variable substitution: arguments

- **FR-040**: When a caller supplies arguments to a retrieval, Tome MUST resolve the documented set of argument-substitution patterns covering all-positional joined, indexed positional, plain numeric positional, and named forms.
- **FR-041**: When the caller's supplied argument value is a single string and the entry declares named arguments in its frontmatter, Tome MUST split the string on whitespace and zip the tokens with the declared names in declaration order.
- **FR-042**: When the caller's supplied argument value is a single string and the entry declares no named arguments, Tome MUST treat the entire string as a single positional value (the whole-string convention for the all-positional reference).
- **FR-043**: When the caller's supplied argument value is a structured object with named keys, Tome MUST source named-substitution values from those keys and MUST build positional values from those named values in the declared name order.
- **FR-044**: When arguments are supplied and the entry body contains no substitution references in any documented argument pattern, Tome MUST append a documented append-fallback footer to the rendered body containing the supplied arguments.
- **FR-045**: Argument values MUST be treated as opaque strings during substitution; substituted values MUST NOT be re-scanned for further substitution.

#### Substitution call sites

- **FR-046**: The substitution layer MUST be invoked from exactly these surfaces:
  - The agent-callable read tool: built-in and environment-passthrough stages MUST always run; the argument-substitution stage and the append-fallback footer MUST run when the caller supplied arguments.
  - The prompts get response: built-in, environment-passthrough, argument-substitution, and append-fallback stages MUST all be available; built-in and environment-passthrough MUST always run; the argument stages MUST run when the caller supplied arguments.
  - The middle-tier metadata tool MUST NOT invoke the substitution layer (it returns metadata about the entry, not the entry body).
  - The search tool MUST NOT invoke the substitution layer (it returns scored result metadata, not bodies).

#### Substitution pipeline ordering

- **FR-050**: The substitution layer MUST process the four stages in this order: built-in references first, environment-variable references second, argument references third, and the append-fallback footer fourth where applicable.
- **FR-051**: Each substitution stage MUST scan the body at most once. Values substituted by an earlier stage MUST NOT be re-scanned by any later stage, and values substituted within a single stage MUST NOT be re-scanned within that stage. The composite effect MUST be equivalent to a single deterministic pass producing the rendered body.
- **FR-052**: References outside the Tome-controlled and user-controlled namespaces MUST pass through the substitution layer unchanged; in particular, harness-native variable conventions (e.g., other tools' `${...}` references) and harness-native shell-execution syntax MUST be preserved verbatim for downstream processing.

#### MCP prompts surface

- **FR-060**: Tome MUST advertise the MCP prompts capability during initialization when at least one user-invocable entry exists across the active workspace's enabled plugins; the documented capability flags MUST reflect the Phase 5 implementation (notably that the list does not change within a session without server restart).
- **FR-061**: Tome MUST respond to the prompts list request with one entry per user-invocable indexed entry, with a derived argument schema per entry and a prompt name composed from a documented per-portion sanitisation and truncation policy.
- **FR-062**: When two entries derive the same prompt name after sanitisation and truncation, Tome MUST resolve the collision by ordering on the entry's recorded index timestamp; ties on identical timestamps (which occur when entries are inserted in the same transaction) MUST be broken deterministically by lexicographic ordering on the tuple (catalog, plugin, kind, name). The earliest entry under this ordering MUST receive the unsuffixed name; subsequent entries MUST receive numeric counter suffixes starting at 2 (`foo`, `foo2`, `foo3`). Each collision MUST be recorded for diagnostics.
- **FR-063**: When an entry's frontmatter declares an explicit prompt-name override, Tome MUST use the override in place of the plugin-prefix and entry-name portions of the generated prompt name; the override MUST still be subject to the documented sanitisation and per-portion length limits. The harness-side prefix that wraps Tome's contributions (typically `mcp__<server>__`) MUST continue to be prepended by the harness; the override does NOT remove the harness-side prefix.
- **FR-064**: Tome MUST respond to the prompts get request for a named prompt by returning the entry's rendered body — after the substitution layer has run — as a single user-role text message.
- **FR-065**: The prompts get response MUST always include built-in and environment-variable substitution regardless of whether the caller supplied arguments; argument substitution MUST run when the caller supplies arguments.
- **FR-066**: Each prompt's `description` in the prompts list response MUST be truncated to a documented character count appropriate for harness slash-menu rendering. The cap MUST be larger than the search-tool result truncation cap (per FR-092) since the slash-menu surface benefits from richer description text than agent-consumed search results. The exact value is pinned in the eventual contract.

#### Argument schema derivation

- **FR-070**: When an entry's frontmatter declares one or more named arguments, the prompt's argument schema MUST contain one required string argument per declared name in declaration order.
- **FR-071**: When an entry's frontmatter declares no named arguments, the prompt's argument schema MUST contain exactly one optional argument named `args`; the argument's description MUST come from the entry's argument-hint frontmatter when present, or a documented generic description otherwise.
- **FR-072**: Even an entry that declares no named arguments and contains no argument-style references in its body MUST receive the optional `args` catch-all argument in its prompt schema; this is what enables the append-fallback footer to surface caller-supplied arguments uniformly.

#### Discovery: middle-tier tool

- **FR-080**: Tome MUST expose a new agent-callable middle-tier metadata tool that returns full description, `when_to_use` content, plugin version, user-invocability flag, the entry's absolute source path, and the kind discriminator for a named entry.
- **FR-081**: For skill-kind entries, the middle-tier tool's response MUST include a one-level enumeration of the entry's source directory: top-level files (other than the entry file itself) and per-subdirectory immediate-children listings.
- **FR-082**: Each subdirectory listing in the middle-tier tool's response MUST be capped at 5 children per subdirectory; over-cap directories MUST list the first 5 children alphabetically and append a sentinel string indicating the count of omitted children. Top-level file listing is also capped at 5 with the same sentinel convention.
- **FR-083**: For command-kind entries (which do not own a directory), the middle-tier tool's response MUST omit the resource enumeration entirely.
- **FR-084**: The middle-tier tool MUST accept an optional kind parameter to disambiguate when the same entry name exists in both kinds for a single plugin; the documented default for the kind parameter MUST select skill-kind.
- **FR-085**: The middle-tier tool's payload MUST be small in expected size relative to the existing read tool — appropriate for cheap mid-tier discovery — and MUST NOT include the entry's body content.

#### Discovery: search tool updates

- **FR-090**: The existing search tool MUST exclude entries whose model-invocation flag is disabled and MUST return both kinds in a single ranked result set.
- **FR-091**: Each search result MUST carry the entry's kind discriminator alongside its existing fields.
- **FR-092**: Each search result's description field MUST be truncated to a documented default character count; the caller MUST be able to override the truncation length via a documented parameter on the search request.
- **FR-093**: The search tool's behaviour MUST be unchanged for queries against entries authored before Phase 5 (skills without commands in the same plugin); the addition of commands MUST NOT degrade ranking for the skill-only case.

#### Discovery: read tool updates

- **FR-100**: The existing read tool MUST accept an optional kind parameter consistent with the middle-tier tool; the documented default MUST select skill-kind.
- **FR-101**: The read tool MUST apply the substitution layer to the returned body; built-in and environment-variable substitution MUST always run; argument substitution MUST run when the caller supplies arguments.

#### Schema, indexing, and lifecycle integration

- **FR-110**: The underlying entry schema MUST gain a kind column, a searchability flag, a user-invocability flag, and a `when_to_use` column; the unique constraint MUST widen to cover (catalog, plugin, kind, name).
- **FR-111**: The schema change MUST land through Phase 3's forward-only migration framework as a registered migration; the migration MUST run in-process on first open of an older database.
- **FR-111a**: The migration MUST backfill existing rows pre-migration as follows: `kind` column populated with the skill-kind value; searchability flag set true; user-invocability flag set false; `when_to_use` column populated as null. Rows so backfilled MUST keep their existing identity (catalog, plugin, name) under the widened unique constraint and MUST NOT be re-embedded merely by virtue of the migration; re-embedding MUST happen on the next reindex that observes a content-hash change or an embedding-text composition change.
- **FR-112**: The workspace enrolment table tracking which entries are active per workspace MUST widen its identification key to match the new entry identity; the plugin enable flow MUST scan both directory roots, create rows of both kinds, and synchronise both kinds into the active workspace's enrolment table.
- **FR-113**: The plugin disable flow MUST disenrol both kinds from the active workspace.
- **FR-114**: The workspace summary regeneration step's input MUST include command names and descriptions alongside skill names and descriptions when at least one command-bearing plugin is enabled in the workspace.

#### Diagnostics and observability

- **FR-120**: The doctor command MUST report the effective MCP surface for the resolved workspace: the existing tool list (now three tools — search, middle-tier metadata, read) and an enumeration of prompts grouped by plugin.
- **FR-121**: The doctor command MUST surface any prompt-name collisions encountered during enumeration, including the original entry names and the assigned counter-suffixed names.
- **FR-122**: The doctor command MUST surface any orphaned persistent data directories (per-plugin or per-workspace) whose referencing plugin is no longer enabled in any workspace; this is informational only in Phase 5 (cleanup is deferred to a later phase).
- **FR-123**: The doctor command MUST report per-kind counts of indexed entries and the number of entries pending re-embedding after a kind-affecting frontmatter change.
- **FR-124**: The Phase 5 additions to doctor's surface MUST be read-only by default: enumerating the prompts surface MUST NOT lazily create persistent data directories, MUST NOT trigger substitution against any entry body, and MUST NOT mutate any on-disk state under Tome's central tree.

#### Plugin show command updates

- **FR-130**: The plugin show command MUST list skills and commands separately, annotating each entry with its effective searchability flag and user-invocability flag, and the derived prompt name where applicable.

### Non-Functional Requirements

- **NFR-001**: The substitution layer MUST execute deterministically given the same entry body, the same substitution context (built-in values, environment passthrough values, caller-supplied arguments), and the same clock reading. The layer MUST NOT invoke any LLM, fetch any remote resource, or read any file other than the entry body itself during a single substitution pass. The clock-derived built-ins (`${TOME_DATE}`, `${TOME_TIMESTAMP}`) MUST be evaluated once per substitution pass against the substitution context's clock value so that golden-output tests can inject a fixed clock.
- **NFR-002**: The middle-tier tool's typical response MUST stay small relative to the read tool's typical response — appropriate for cheap mid-tier discovery; the cost of issuing the call against a heavy supporting directory MUST stay bounded by the per-directory enumeration cap rather than scaling with the directory's actual content.
- **NFR-003**: The total prompt name MUST stay within MCP's documented prompt-name length budget after sanitisation and truncation; the per-portion budgets MUST be chosen so that the combined name always fits.
- **NFR-004**: The substitution layer MUST never write to or read from the host filesystem outside the documented persistent data directories during a substitution pass; in particular, environment-variable substitution MUST be limited to reading from the host process's environment, not from any on-disk environment file.
- **NFR-005**: The substitution layer MUST never substitute an environment-variable reference whose name lies outside the documented user-controlled namespace, regardless of whether the host environment has the variable set.
- **NFR-006**: The default behaviour of the workspace MUST be safe: a workspace whose enabled plugins include no user-invocable entries MUST still function as it did in Phase 4; the prompts surface MUST simply be empty.
- **NFR-007**: The substitution layer MUST treat all caller-supplied argument values as opaque strings; in particular, an argument value that happens to contain Tome-namespaced or environment-namespaced references MUST NOT cause any further substitution.
- **NFR-008**: The MCP server MUST continue to advertise the prompts capability conservatively: list-changed notifications MUST be declared not supported in Phase 5; workspace switches that change the prompt set MUST be handled by server restart, which is the existing per-session model.
- **NFR-009**: The lookups underlying the prompts list, the search results, and the middle-tier tool MUST scale linearly in the size of the active workspace's enabled-entry set; no new asymptotic cost classes are introduced.
- **NFR-010**: The substitution layer MUST be invocable from blocking (non-async) contexts. The layer is reachable from both the agent-callable read tool (called from within the MCP server's blocking dispatch) and the prompts get response (called from the MCP server's request handler); both call sites MUST share the same substitution implementation without an async runtime requirement.
- **NFR-011**: The substitution layer's working memory MUST be bounded by a small constant multiple of the entry body size; in particular, no cost class that scales superlinearly in the body size due to repeated full-body scans MUST be introduced.
- **NFR-012**: The substitution layer's persistent data directory creation MUST be idempotent and safe under concurrent calls from multiple in-flight retrievals against the same plugin or workspace; the implementation MUST tolerate the case where two retrievals race on the same first-time directory creation.

### Key Entities *(include if feature involves data)*

- **Entry** (concept): The unified record for what was previously called a skill row. Carries catalog, plugin, kind, name, description, `when_to_use`, source path, content hash, searchability flag, user-invocability flag, and an embedded representation. Two kinds: skill and command. Identified by (catalog, plugin, kind, name).
- **Prompt** (concept): A view onto an entry exposed through the MCP prompts capability. Has a derived prompt name (with documented sanitisation, length, and override rules), a derived argument schema (from the entry's frontmatter and argument-hint), and a rendered body (the entry's body after the substitution layer has run).
- **Substitution context** (concept): The runtime values feeding the substitution layer for a single retrieval. Composed of: built-in values (paths and metadata derived from the entry, plugin, workspace, and clock); environment passthrough values (read from the host process's environment, restricted to a documented namespace); and optional caller-supplied argument values (a single string or a structured object).
- **Persistent data directory** (concept): Two writable directories created lazily by the substitution layer on first reference: one per plugin (shared across workspaces) and one per workspace per plugin. Survive plugin version upgrades; are managed by the plugin author at runtime; are not cleaned up by Tome in Phase 5.
- **Resource enumeration** (concept): The middle-tier tool's structured representation of an entry's directory listing. Top-level files (excluding the entry file itself) and per-subdirectory child paths, with a per-directory cap and a documented "and N more" sentinel for over-cap directories. Present only for skill-kind entries.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From a fresh install, a developer with a plugin containing both skills and commands sees both kinds appear in agent search results within a single workspace bind; commands appear in a harness's slash-menu without further per-plugin or per-project configuration.
- **SC-002**: A plugin author can write a skill body whose path references resolve correctly across at least two different deployment environments (different home directories, different harness directories, different bound projects) without per-environment edits to the skill source.
- **SC-003**: An argument-bearing command authored against the Phase 5 substitution rules executes correctly from at least one MCP-prompt-supporting harness with arguments delivered through the harness's structured prompt UI, with no degradation of behaviour compared to the same command's behaviour in its native Claude Code installation.
- **SC-004**: The middle-tier metadata tool's typical response size is at least an order of magnitude smaller than the read tool's typical response size for the same entry on a plugin whose skill ships a heavy supporting directory; the agent can use it to disambiguate between two competing skill candidates without paying for either body.
- **SC-005**: A plugin author who flips an entry's user-invocability flag and reindexes sees the change reflected in the harness's slash menu on the next harness session without any further commands; flipping the flag back and reindexing returns the surface to its previous state.
- **SC-006**: A harness that does not support MCP prompts (Codex as of Phase 5) connected to a Phase 5 Tome remains fully functional for agent search and read; commands stay accessible to the agent through the existing tools using the kind discriminator; no startup error or capability-mismatch surfaces.
- **SC-007**: Indexing the same plugin tree under Phase 5 produces a strict superset of what Phase 4 indexed: every Phase 4 skill row is preserved at the same identity; every Phase 5 command row is new; entries whose `when_to_use` frontmatter newly contributes to embedding text are re-embedded on the next reindex but remain identifiable at the same identity.
- **SC-008**: The doctor command's output for a workspace with at least one user-invocable entry includes the prompt enumeration grouped by plugin, including any counter-suffixed names from collision resolution; a workspace whose enabled plugins yield no user-invocable entries reports an empty prompts surface without failing.
- **SC-009**: The workspace summary regeneration input string includes the names and descriptions of command-kind entries when at least one command-bearing plugin is enabled in the workspace; the absence of command-bearing plugins reproduces Phase 4's skill-only summary input.
- **SC-010**: An entry body containing a token-shaped environment-variable reference outside the user-controlled namespace passes through Tome unmodified; the host environment's value for that variable does not appear in any Tome log, error message, or rendered output.
- **SC-011**: An entry body containing the harness-native shell-execution backtick syntax passes through Tome unmodified; no Tome process invokes any shell command during the retrieval.
- **SC-012**: Two enabled plugins whose entries resolve to the same prompt name after sanitisation produce counter-suffixed prompt names; both are independently invocable from the harness's slash menu; the doctor command surfaces the collision and points at the prompt-name-override frontmatter remediation.
- **SC-013**: Each new MCP response shape introduced in Phase 5 (the prompts list response, the prompts get response, the middle-tier metadata tool response) and each new diagnostic record surfaced through doctor (the prompts enumeration, the collision record, the orphan persistent-data report) is pinned by a byte-stable serialisation test covering both the human and JSON output paths where applicable.

## Assumptions

- Harnesses that have shipped MCP prompts support (Claude Code, and most Phase 4 harnesses besides Codex) render the prompts list in their slash-command UI without per-prompt configuration on the user's side. Where a harness's UI differs in detail (icons, ordering, grouping), Phase 5 does not constrain that surface beyond the prompt list and argument schemas Tome publishes.
- Codex's lack of MCP prompts support is upstream-tracked (issue #8342 as referenced in the source PRD) and not Phase 5's problem; Codex users access commands through the agent-callable search and read tools using the kind discriminator.
- The PRD's documented per-portion sanitisation budgets (16 characters for the plugin portion, ~32 characters for the entry portion) are sized against MCP's overall prompt-name length budget and current harness UI rendering; they are not subject to clarification in Phase 5.
- The PRD's documented set of 12 built-in variables is exhaustive for Phase 5; the deliberately-excluded variables (`${TOME_HARNESS}`, `${TOME_SESSION_ID}`, `${TOME_WORKSPACE_DIR}`) are out of scope and remain available via user-controlled environment passthrough or are not provided at all.
- Server-side execution of harness-native shell syntax remains the host harness's responsibility for Phase 5; plugin authors who need cross-harness dynamic context bundle scripts and reference them via the entry-directory built-in.
- Plugin authors of existing Claude Code plugins do not need to retouch their plugin trees to land them under Phase 5; the entries indexed and the prompts exposed reflect what their plugins already declare.
- The Phase 3 migration framework is the right place for the schema change; Phase 4's first registered migration validated the framework's mechanics, and Phase 5's migration follows the same pattern.
- Persistent plugin-data and workspace-data directories do not need active cleanup in Phase 5; orphans remain on disk indefinitely and are surfaced by doctor for informational purposes only. Cleanup tooling is Phase 6+ work.
- The substitution layer's per-body cost is dominated by simple pattern matching; the layer's runtime is negligible relative to the existing embedding and reranking costs that dominate the read and search paths.

## Development Standards

- All new declarative inputs (entry frontmatter beyond the lenient third-party boundary, new internal data types describing prompts and substitution contexts) follow the existing strictness boundary: Tome-owned inputs are strict, third-party inputs (plugin author frontmatter) are lenient — the latter being what Phase 5 reads from plugin trees.
- All new error classes use the existing closed-enum discipline; each documented Phase 5 failure mode either reuses an existing variant or claims a new dedicated exit code with a number documented in the eventual contracts.
- All new MCP tool schemas land through the existing schema-derivation pattern; argument-schema generation for prompts follows the same shape derivation discipline.
- The substitution layer ships as a hand-rolled module — no templating-engine dependency — consistent with the source PRD's deliberate non-goal.
- All new code surface that touches the on-disk persistent data directories follows the existing atomic-write discipline; the directories themselves are created via the same idempotent `create_dir_all` pattern Tome already uses for workspace and plugin state.
- All new tests cover both human and JSON output modes for any new CLI surface; existing JSON wire-shape pinning discipline carries forward to the new prompt-name, prompts list, and middle-tier tool response shapes.
