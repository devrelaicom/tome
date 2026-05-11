# Feature Specification: Phase 1 — Project Foundations and Catalog Management

**Feature Branch**: `001-phase-1-foundations`
**Created**: 2026-05-11
**Status**: Draft
**Input**: User description: "Tome Phase 1: Rust CLI project foundations and catalog management as specified in PRDs/phase-1.md"

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details.

**Source PRD**: [PRDs/phase-1.md](../../PRDs/phase-1.md) — the implementation-level brief is authoritative for HOW; this spec is authoritative for WHAT.

## Overview

Tome is a command-line tool that lets developers manage **catalogs** of plugins for AI coding assistants. A catalog is a remote source (a Git repository) that lists one or more plugins. In Phase 1 the tool lets a developer register catalogs, refresh them, and inspect their contents. Installing plugins from catalogs into specific harnesses is deferred to a later phase.

This phase also delivers the project foundations needed for the work that follows: a contributor-friendly repository structure, automated quality gates, and a documented release/contribution workflow.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Register and inspect a remote catalog (Priority: P1)

A developer has heard about a plugin collection (a catalog) published on GitHub. They want to register it locally so they can see what plugins it offers, refresh it as the catalog evolves, and remove it if they lose interest.

**Why this priority**: Catalog management is the entire visible user surface of Phase 1. Without it, the tool delivers no user value. P1 is the MVP.

**Independent Test**: From a fresh install, a developer registers a public Git-hosted catalog using a short identifier (e.g. `owner/repo`), confirms it appears in their list of catalogs, inspects its declared plugin list, refreshes it, and removes it — each step producing intelligible human-readable output and a usable structured output mode for scripts.

**Acceptance Scenarios**:

1. **Given** the tool is freshly installed and no catalogs are registered, **When** the developer registers a public catalog by its short identifier, **Then** the catalog appears in the list of registered catalogs with its name, source location, tracked ref, plugin count, and the time it was last synchronised.
2. **Given** a registered catalog, **When** the developer asks to view its details, **Then** the manifest contents are displayed — name, description, owner contact, version, and the catalog's declared plugin list.
3. **Given** a registered catalog, **When** the developer requests a refresh, **Then** the catalog is updated against its upstream source, the local copy reflects any upstream changes, and the last-synchronised time is updated. If multiple catalogs are refreshed in one command, the first failure stops the run and the failing catalog is named in the error output.
4. **Given** a registered catalog, **When** the developer requests its removal, **Then** the developer is asked to confirm before anything is deleted, and only after explicit confirmation are the local copy and registration removed.
5. **Given** a developer running in a non-interactive context (CI, automation, scripting), **When** they invoke any command that would otherwise prompt for confirmation without supplying a confirmation flag, **Then** the command exits with a clear error rather than hanging or silently proceeding.
6. **Given** any catalog command, **When** the developer requests structured output, **Then** the command produces machine-readable structured output suitable for piping into scripting tools, with errors emitted in the same structured form on the error stream.

---

### User Story 2 — Author a catalog that the tool accepts (Priority: P2)

A plugin author maintains a Git repository that bundles one or more plugins. They write a small manifest file at the root of that repository so the tool can recognise it as a catalog and surface its plugins to developers.

**Why this priority**: Catalogs are only useful if authors can produce them. The manifest must be small enough to write by hand, strict enough that mistakes produce clear errors, and stable enough that early adopters' catalogs continue to work as Phase 2 lands.

**Independent Test**: An author writes a manifest declaring a catalog name, description, owner contact, version, and one or more plugins each with a name and a relative location inside the same repository. Registering the catalog succeeds. Introducing a typo, an unknown field, an absolute path, a parent-directory-traversal path, or a URL-shaped plugin location produces a clear error that names the offending field, value, and file.

**Acceptance Scenarios**:

1. **Given** a Git repository containing a well-formed catalog manifest, **When** the developer registers it, **Then** the registration succeeds and the catalog is listed.
2. **Given** a manifest with an unknown top-level field, **When** the developer attempts to register the catalog, **Then** the operation fails with an error identifying the unknown field by name and pointing the author at the expected schema.
3. **Given** a manifest where a plugin's declared location is an absolute path, a parent-directory-traversal path, a URL, or otherwise resolves outside the catalog repository, **When** the developer attempts to register the catalog, **Then** registration fails with an error naming the field, the offending value, and the manifest file path.
4. **Given** a manifest missing a required field, **When** the developer attempts to register the catalog, **Then** the operation fails with an error naming the missing field and the manifest file path.

---

### User Story 3 — Onboard as a contributor (Priority: P3)

A new contributor wants to fix a bug or propose a small feature. They clone the repository, run a single setup command, and submit a pull request that passes all automated checks on the first attempt.

**Why this priority**: Tome is an open-source project with an indefinite lifespan. Contributor onboarding friction directly affects the project's long-term health. It is third in priority only because catalog functionality must exist before there is anything meaningful to contribute to.

**Independent Test**: A developer who has never seen the repository before clones it, follows the contributor guide, runs the documented setup command, makes a trivial documentation change, opens a pull request, and observes that all automated quality gates pass without manual intervention — within ten minutes of `git clone`.

**Acceptance Scenarios**:

1. **Given** a fresh clone of the repository, **When** the contributor follows the documented setup steps, **Then** linting, formatting, typo detection, commit-message validation, and tests can be run locally with a single documented command each.
2. **Given** a contribution that violates a quality gate (formatting, linting, typo, non-conventional commit message), **When** the contributor attempts to commit or push, **Then** the local automation rejects the change before it leaves the developer's machine and explains what needs to be fixed.
3. **Given** a contribution that passes local checks, **When** the contributor opens a pull request, **Then** the project's automated checks run on supported operating systems and on the project's pinned minimum supported language version, and the result is reported on the pull request.
4. **Given** the project depends on third-party components, **When** the project's scheduled security checks run, **Then** vulnerable or disallowed-license dependencies are reported through a clearly visible automated alert before they reach a release.

---

### Edge Cases

- **Catalog already registered.** Re-registering an already-known catalog produces a distinguishable error code; the existing registration is unchanged.
- **Catalog not found at refresh time.** If the upstream catalog is unreachable, the failure is reported with the underlying cause surfaced (without leaking credential material) and the local copy remains untouched.
- **Catalog cache missing or corrupted.** If the local copy is missing or unreadable when a refresh is requested, the tool re-fetches from upstream rather than failing outright.
- **Catalog pinned to a specific commit identifier.** Refreshing a catalog that is pinned to a specific commit produces an informational message rather than attempting an update that cannot succeed.
- **Refresh-all with a partial failure.** When refreshing every registered catalog in one command, the first failure stops the run; previously refreshed catalogs are not rolled back, and the failure names the catalog and reason.
- **Non-interactive environment with a destructive operation.** Commands that would normally prompt for confirmation must not hang on a non-TTY input stream; they exit with a clear error directing the caller to supply the appropriate confirmation flag.
- **Credential-bearing source location.** If an upstream tool emits credential-bearing material in its error output (for example, a URL containing an embedded token), the tool must scrub that material before surfacing the error to the user, the log, or any structured output.
- **Author supplies a destination name that conflicts with an existing one.** Registration fails with a distinguishable error code; the existing catalog is unchanged.
- **Local filesystem error during cache operations.** Permission-denied, disk-full, and missing-parent-directory conditions produce a dedicated, distinguishable error code rather than being collapsed into a generic failure.

## Requirements *(mandatory)*

### Functional Requirements

**Catalog management**

- **FR-001**: The tool MUST allow a developer to register a catalog identified by a Git source location, a short identifier expanded to a default hosting provider, or a local filesystem path.
- **FR-002**: The tool MUST allow a developer to optionally override the registered catalog's display name and pin the catalog to a specific branch, tag, or commit at registration time.
- **FR-003**: The tool MUST allow a developer to list every registered catalog, showing for each its display name, source location, tracked reference, plugin count, and last-synchronised timestamp.
- **FR-004**: The tool MUST allow a developer to view the contents of a registered catalog's manifest, including name, description, owner contact, version, and declared plugin list.
- **FR-005**: The tool MUST allow a developer to refresh a single named catalog or all registered catalogs in one operation, surfacing changes from upstream and updating the recorded synchronisation timestamp. When refreshing all catalogs, the operation MUST process catalogs sequentially in Phase 1; concurrent or parallel refresh is explicitly deferred.
- **FR-006**: The tool MUST allow a developer to remove a registered catalog, prompting for confirmation before removing local data and registration; the prompt MUST be bypassable with an explicit confirmation flag.
- **FR-007**: When refreshing multiple catalogs in one command, the tool MUST stop on the first failure and exit with the failing catalog's error category; partial-failure modes are not supported in Phase 1.
- **FR-008**: When a catalog is pinned to a specific commit, refresh operations MUST produce an informational message and exit successfully rather than attempting to track upstream movement.

**Catalog manifest format**

- **FR-009**: The tool MUST recognise a catalog by the presence of a single, documented manifest file at the root of the catalog's Git repository.
- **FR-010**: Every declarative document the tool parses (catalog manifest, persisted configuration, and any future declarative input) MUST be parsed strictly: every table at every nesting level MUST reject unknown keys with an error naming the offending key and the source file. No table in any declarative document is exempt from this rule, present or future.
- **FR-011**: The manifest MUST require, at minimum, the catalog's name, description, version, owner contact (name and email), and a list of plugins each with a name and a location within the catalog repository.
- **FR-012**: A plugin's declared location MUST be a normalised relative path within the catalog repository; absolute paths, paths containing parent-directory references, URL-shaped locations, and platform-specific absolute prefixes MUST be rejected at registration time.
- **FR-013**: A plugin's location MUST, after canonicalisation including symlink resolution, resolve to a path inside the catalog's repository root; locations that escape the repository root — whether through path syntax (`..`, absolute paths) or through symlink targets — MUST be rejected with the same error category. Resolution behaviour MUST be identical across supported operating systems.

**Local configuration and storage**

- **FR-014**: The tool MUST persist its catalog registry in a platform-appropriate user configuration directory; users MUST NOT need to specify a path manually.
- **FR-015**: The tool MUST store its local catalog cache in a platform-appropriate user data directory, with each catalog isolated into its own subdirectory keyed by a hash of its source location to prevent collisions.
- **FR-016**: The persisted configuration file MUST be parsed under the same strictness rule as the catalog manifest (FR-010): every table at every nesting level rejects unknown keys.
- **FR-017**: The catalog cache MUST be treated as tool-owned storage; refresh operations MUST be permitted to overwrite local modifications inside the cache without prompting.
- **FR-017a**: Every mutation of the catalog cache MUST be atomic per catalog: a failed refresh MUST leave the affected catalog's cache directory either in its pre-operation state or in its fully-updated state, never in a partially populated intermediate state. Temporary working directories used during refresh MUST be cleaned up on both success and failure paths.
- **FR-017b**: Every mutation of the persisted catalog registry MUST be atomic with respect to concurrent invocations and to mid-write interruption (process kill, power loss). A partially written registry file MUST never be observable to a subsequent invocation.

**Output and scripting**

- **FR-018**: Every command that produces output MUST default to human-readable output on the standard output stream, and MUST support a structured output mode that emits machine-readable output suitable for downstream tooling.
- **FR-019**: All errors MUST be emitted on the standard error stream; in structured-output mode, errors MUST be emitted as structured records on the error stream.
- **FR-019a**: The tool's own output MUST be UTF-8 regardless of the user's locale. Output captured from external processes (e.g. the system Git client) MUST be handled as opaque bytes and lossily decoded only for display purposes; the tool MUST NOT assume any particular locale encoding for external output.
- **FR-019b**: Diagnostic log records MUST always be emitted on the standard error stream, unstructured, regardless of whether the user has selected structured-output mode for the command's primary output. The verbosity of diagnostic logging MUST be controllable independently of the structured-output flag.
- **FR-020**: The tool MUST honour an environment-variable mechanism for disabling colour output, and MUST auto-disable colour when its output is not connected to a terminal.
- **FR-021**: Every interactive prompt MUST have a non-interactive flag equivalent; when standard input is not connected to a terminal, the tool MUST exit with a clear error rather than hang or silently proceed. The same flag name MUST bypass every interactive prompt across every command — there MUST NOT be per-command variations of the confirmation-bypass flag.
- **FR-021a**: Every command and subcommand MUST support a help flag that prints usage and a short description to the standard output stream and exits successfully. The top-level binary MUST support a version flag that prints the build's semantic version to the standard output stream and exits successfully.

**Error reporting and exit codes**

- **FR-022**: The tool MUST distinguish error categories through documented, stable exit codes. The set of error categories is **closed and exhaustive**: usage error, catalog-not-found, catalog-already-registered, manifest-invalid, Git-operation-failed, filesystem-or-IO error, and user-initiated interruption (see FR-026a). Adding a new error category requires a documented amendment to this specification. A generic exit code MUST be reserved for genuinely unexpected internal conditions only, and no failure mode named above may collapse into the generic code.
- **FR-023**: Every user-facing error MUST name what failed, where it failed, and — where possible — what the user can do next.
- **FR-024**: When the tool surfaces an upstream error (for example, output from a system Git invocation), it MUST scrub credential material from that output **at the boundary where the material enters the tool** — at the point of capturing the external process's output — and not at the eventual display site. As a consequence, no downstream surface (diagnostic logs, structured records, error chains, traces, future telemetry) can carry an un-scrubbed value.
- **FR-025**: The tool MUST NOT log, emit, or otherwise persist credential material (tokens, embedded URL credentials, credential-helper names, raw invocation arguments that may carry credentials).

**Source-control integration**

- **FR-026**: The tool MUST inherit the user's existing source-control authentication configuration; it MUST NOT prompt for, store, or otherwise manage credentials of its own.
- **FR-026a**: A long-running external operation (a clone or fetch) MUST be cancellable by the operating system's standard interrupt signal. On cancellation the tool MUST exit with the documented interrupted exit code, MUST leave no orphaned child processes, and MUST honour the per-catalog cache atomicity rule (FR-017a) — the cache is left either in its pre-operation state or in its fully-updated state, never partially populated.

**Contributor experience and quality gates**

- **FR-027**: The repository MUST document a single setup command that prepares a new contributor's local environment, and the documented setup-to-first-PR path MUST be completable within ten minutes from a fresh clone.
- **FR-028**: Formatting, linting, typo detection, and commit-message validation MUST be enforced locally before a contributor pushes a branch.
- **FR-029**: Automated checks MUST run on every pull request and every push to the default branch, across supported operating systems and at minimum against the pinned minimum supported language version of the project.
- **FR-030**: Security and licence-compliance checks MUST run on a recurring schedule and on every pull request, reporting vulnerable dependencies and disallowed licences before a release.
- **FR-031**: Every commit landing on the default branch MUST follow the project's chosen commit-message convention, enforced locally before push.
- **FR-032**: The project MUST be published under a dual permissive licence acknowledged in every source distribution.

### Key Entities

- **Catalog**: A registered, named pointer to a remote (or local) Git source containing a manifest at its root. Has a display name, source location, optional pinned reference, last-synchronised timestamp, and a cached local copy.
- **Catalog manifest**: A small declarative document at the root of a catalog's repository declaring the catalog's identity (name, description, version), its owner contact, and the list of plugins it ships.
- **Plugin (declaration)**: A named entry within a catalog manifest describing where the plugin lives inside the catalog repository. In Phase 1, plugins are described but not installed.
- **Catalog registry**: The persisted, on-disk list of catalogs known to the developer's local environment.
- **Catalog cache**: The on-disk working copy of each registered catalog, owned exclusively by the tool.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer who has just installed the tool can register a public Git-hosted catalog and view its plugin list in fewer than three commands and under two minutes, end-to-end.
- **SC-002**: A new contributor can clone the repository, run the documented setup, and submit a pull request whose automated checks pass on the first attempt within ten minutes of `git clone`.
- **SC-003**: Every commands' error output names what failed, where it failed, and the next action, as verified by an inventory of all error paths exercised by the test suite.
- **SC-004**: Every documented error category is represented by a distinct exit code; no two distinct failure modes share an exit code, as verified by automated assertions in the test suite.
- **SC-005**: Manifest parsing rejects 100% of malformed inputs in the test corpus (unknown fields, missing fields, path traversals, URL-shaped plugin locations, platform-specific absolute paths) with an error that names the offending field and the manifest file.
- **SC-006**: No credential material is observable in any user-facing output, log line, or structured-output record across the full integration test suite, including tests that deliberately exercise authentication failure paths.
- **SC-007**: Every documented command behaves identically in human-readable mode and structured-output mode with respect to exit codes, error categorisation, and the operations it performs.
- **SC-008**: Every interactive prompt has a flag-based equivalent that allows the command to be driven without a connected terminal; this is verified by an end-to-end suite that runs every prompting command under a non-TTY harness.
- **SC-009**: Automated quality gates (formatting, linting, tests, security scan, licence scan) run green on every pull request and on a recurring schedule, on every supported operating system and on the pinned minimum supported language version.
- **SC-010**: Distributable artefacts (the installed binary) are under ten megabytes after stripping on release builds, verified on every release.
- **SC-011**: Interrupting any long-running operation with the standard interrupt signal causes the tool to exit with the documented interrupted exit code and leaves no orphaned child processes, verified by an interruption-injecting test that runs against every command that invokes an external Git operation.
- **SC-012**: A simulated mid-write interruption of registry or cache writes leaves the on-disk state recoverable on the next invocation — either the pre-operation state or the fully-updated state — verified by an interruption-injecting test for every code path that mutates persisted state.

## Assumptions

- The developer running the tool has the system Git client installed and configured with whatever authentication their target Git host requires. The tool does not provision or manage credentials.
- The developer running the tool has read/write access to the platform-appropriate user configuration and user data directories.
- Catalog authors are willing to write a small declarative manifest by hand; bespoke tooling for manifest generation is not part of Phase 1.
- The Git hosting provider used by short-identifier expansion is a sensible default for the developer's community; alternate providers can still be addressed by supplying a full Git URL.
- Catalogs in Phase 1 declare their plugins as locations within the catalog repository only. Other ways to declare a plugin source (external Git URL, registry pointer, submodule) are explicitly deferred.

## Out of Scope (Phase 1)

The following are explicitly deferred — call-outs here exist so that the spec is not silently understood to include them:

- Installing plugins from a catalog into any specific AI-coding-assistant harness (the visible purpose of later phases).
- Detecting which AI-coding-assistant harness is in use locally.
- Translating skills, commands, agents, or hook definitions between harnesses.
- A built-in semantic search index over plugin descriptions.
- A long-running server process exposing tool operations to other agents.
- Authentication beyond what the user's existing Git client provides.
- Compatibility with any third-party plugin marketplace or manifest format.
- Cross-platform release tooling and distribution through package managers.

## Dependencies and Constraints

- The tool depends on the user's system Git client being on the executable path.
- The tool runs on at least the two primary developer operating systems and on the project's pinned minimum supported language version.
- The tool's persistent state (registry and cache) is platform-appropriate by default; users must not need to specify paths manually.

## Codebase Documentation

This is a new, greenfield project. See [`.sdd/codebase/STACK.md`](../../.sdd/codebase/STACK.md) for the intended technology stack derived from the PRD and constitution. Additional codebase documents will be populated after initial development.
