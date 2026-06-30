#!/usr/bin/env node
// Fails CI when the docs and specs/reference/cli-surface.json disagree, in
// either direction:
//   forward: everything in the contract must appear in the docs
//   reverse: every exit code documented / `tome X` heading must be in the contract
import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const contract = JSON.parse(readFileSync(path.join(root, 'specs/reference/cli-surface.json'), 'utf8'));
const commandsDoc = readFileSync(path.join(root, 'docs/reference/commands.md'), 'utf8');
const exitCodesDoc = readFileSync(path.join(root, 'docs/reference/exit-codes.md'), 'utf8');

const errors = [];

// -- forward: contract ⊆ docs ------------------------------------------------
for (const [group, def] of Object.entries(contract.commands)) {
  if (!commandsDoc.includes(`## \`tome ${group}\``)) {
    errors.push(`commands.md: missing section heading \`tome ${group}\``);
    continue;
  }
  for (const [sub, flags] of Object.entries(def.subcommands)) {
    if (!commandsDoc.includes(`\`${sub}`)) {
      errors.push(`commands.md: \`tome ${group}\` is missing subcommand \`${sub}\``);
    }
    for (const flag of flags) {
      if (!commandsDoc.includes(flag)) {
        errors.push(`commands.md: \`tome ${group} ${sub}\` is missing flag ${flag}`);
      }
    }
  }
  for (const flag of def.flags) {
    if (!commandsDoc.includes(flag)) {
      errors.push(`commands.md: \`tome ${group}\` is missing flag ${flag}`);
    }
  }
}
for (const flag of contract.globalFlags) {
  if (!commandsDoc.includes(flag)) errors.push(`commands.md: missing global flag ${flag}`);
}
for (const {code, category} of contract.exitCodes) {
  if (!exitCodesDoc.includes(`| \`${code}\` |`)) {
    errors.push(`exit-codes.md: missing row for code ${code}`);
  } else if (category && !exitCodesDoc.includes(`\`${category}\``)) {
    errors.push(`exit-codes.md: code ${code} present but category \`${category}\` missing`);
  }
}

// -- reverse: docs ⊆ contract --------------------------------------------------
const knownCodes = new Set(contract.exitCodes.map((e) => e.code));
for (const m of exitCodesDoc.matchAll(/^\| `(\d+)` \|/gm)) {
  const code = Number(m[1]);
  if (!knownCodes.has(code)) errors.push(`exit-codes.md: documents code ${code}, not in contract`);
}
for (const m of commandsDoc.matchAll(/^## `tome ([a-z-]+)`/gm)) {
  if (!(m[1] in contract.commands)) errors.push(`commands.md: documents \`tome ${m[1]}\`, not in contract`);
}

if (errors.length) {
  console.error(`DOC DRIFT — ${errors.length} problem(s):\n`);
  for (const e of errors) console.error(`  ✗ ${e}`);
  console.error('\nFix the docs, or regenerate the contract: node scripts/generate-cli-surface.mjs');
  process.exit(1);
}
console.log('✓ docs match specs/reference/cli-surface.json');
