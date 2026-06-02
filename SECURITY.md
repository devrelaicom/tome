# Security Policy

## Reporting a vulnerability

Please report security issues — credential leaks, path traversal, sandbox escapes, or anything else with a security impact — **privately**, not as a public issue.

Use **GitHub private vulnerability reporting**: open the repository's **Security** tab and choose **Report a vulnerability** (this opens a private advisory visible only to you and the maintainers). Maintainers triage reports there.

Please include enough detail to reproduce: the Tome version (`tome --version`), your OS and architecture, the commands or inputs involved, and what you observed versus expected. We will acknowledge your report and keep you updated as we investigate.

## Trust model: mechanical guarantees vs. semantic content

Tome's security posture rests on one distinction. Tome makes **mechanical** guarantees about how it reads and writes files. It makes **no semantic** guarantees about the *content* of the catalogs you register. Understanding where that line sits is the most important thing to know before you use Tome.

### What Tome does defend against (mechanical guarantees)

These protections apply at the boundary, on every relevant path, regardless of how hostile the input is:

- **No memory exhaustion from oversized inputs.** Every read of a third-party file is bounded by that file class's existing size cap — 256 KiB for plugin manifests and skill/command/agent frontmatter, 1 MiB for harness settings and hook specs — and fails with a named, bounded error rather than reading an unbounded file into memory.
- **No path traversal via plugin-supplied names.** Any plugin- or frontmatter-supplied string that becomes a filename (for example an agent's `name`) is validated as a single safe path segment: empty, NUL, `/`, `\`, `.`, `..`, and leading-dot values are rejected, and the write site additionally checks that the resolved parent is the intended directory.
- **No symlink escape on writes.** Before any Tome-managed write (harness rules files, MCP config, hooks, guardrails regions, native agent files, the catalog registry, atomic directory landings), Tome refuses to traverse a symlinked path component — not just a symlinked final node, but a symlinked **intermediate directory** as well. On Linux this uses an in-kernel no-symlink resolution; on macOS and other Unix it is a per-component `O_NOFOLLOW` walk. This is defence-in-depth against a directory being swapped for a symlink mid-operation (a TOCTOU window) and redirecting a write outside the intended tree.
- **No partial or corrupted state on failure.** Tome's registry, index, cache, model directory, and per-plugin installs are written atomically — built in a staging location and renamed into place — so an interrupted operation (a crash or `Ctrl-C`) never leaves a half-written file or directory. Merges into existing harness settings are structural-match-only: Tome edits only its own marker-delimited regions and leaves operator-owned content untouched.
- **No credential leakage.** URLs that may carry embedded credentials are scrubbed before they reach logs, error messages, or any display path, including in error chains from Git and the model/summariser downloads.
- **No telemetry.** Tome does not phone home. Its only network access is Git operations against catalogs you explicitly register and one-time downloads of pinned, SHA-256-verified inference models.

### What Tome cannot defend against (semantic content)

**Tome cannot vet the *content* of a catalog.** A catalog's skills, commands, and agents are instructions — prose, prompts, and templates that **your own agent will read and may act on**. Tome indexes them, makes them searchable, and surfaces them to your coding harness; it does not, and cannot, judge whether those instructions are safe, honest, or in your interest.

Concretely, Tome cannot tell whether a skill quietly instructs your agent to exfiltrate a secret, run a destructive command, or subvert your intent. The mechanical guarantees above stop a malicious catalog from corrupting Tome's own files or escaping the filesystem boundaries it controls — they do **not** stop a malicious *instruction* from doing harm once your agent follows it.

**Adding a catalog is trusting it.** Treat `tome catalog add` the way you would treat installing a dependency or running a script from that source: **only add catalogs you trust**, and review a catalog's contents the same way you would review any code or instructions you are about to give an autonomous agent.

## Supported versions

Tome is in beta. Security fixes are applied to the latest released version. Until a 1.0 release, older versions are not separately maintained — please upgrade to the latest release.
