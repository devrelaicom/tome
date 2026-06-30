// Supply-chain guard: this project is pnpm-only.
//
// Runs as the root `preinstall` script. npm, pnpm, yarn and bun all execute
// the root project's `preinstall` BEFORE resolving, downloading, or building
// any dependency — so a non-zero exit here aborts the install before any
// un-vetted lifecycle script or freshly-resolved version can touch the machine.
//
// Why this matters: `npm install` ignores pnpm-lock.yaml entirely. It would
// re-resolve every dependency to whatever is newest on the registry (bypassing
// our pinned, age-gated lockfile) and run dependency install scripts. This
// guard turns that accidental command into a hard stop.
//
// Designed to NEVER false-positive on pnpm: it only blocks a *known* foreign
// package manager (npm/yarn/bun). An empty or unrecognised user-agent is
// allowed, so pnpm is never wrongly blocked in odd environments.

const ua = process.env.npm_config_user_agent || "";
const usedPnpm = ua.startsWith("pnpm/");
const foreignPM = /^(npm|yarn|bun)\//.exec(ua);

if (foreignPM && !usedPnpm) {
  const name = foreignPM[1];
  process.stderr.write(`
✘ This project is pnpm-only — refusing to install with ${name}.

  '${name} install' ignores pnpm-lock.yaml, re-resolves every dependency to
  fresh registry versions, and runs dependency lifecycle scripts. That bypasses
  this project's supply-chain controls (lockfile pinning + minimumReleaseAge).

  Use pnpm instead:

      pnpm install

  Pinned version: see "packageManager" in package.json.
`);
  process.exit(1);
}
