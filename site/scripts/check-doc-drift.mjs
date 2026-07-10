#!/usr/bin/env node
// Fails CI when the docs and specs/reference/cli-surface.json disagree, in
// either direction:
//   forward: everything in the contract must appear in the docs
//   reverse: every exit code documented / `tome X` heading must be in the contract
// Also checks the harness matrix in docs/using-tome/harnesses.md against the
// registries in ../src/harness/mod.rs (the site lives inside the tome repo, so
// the Rust source is readable at check time — no binary build needed).
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

const errors = [];

/** Read a file and return its text, or push a clean error entry and return null. */
function readFileSafe(filePath) {
  try {
    return fs.readFileSync(filePath, 'utf8');
  } catch (e) {
    errors.push(`Cannot read ${filePath}: ${e.message}`);
    return null;
  }
}

const contractRaw = readFileSafe(path.join(root, 'specs/reference/cli-surface.json'));
let contract;
try {
  contract = contractRaw != null ? JSON.parse(contractRaw) : null;
} catch (e) {
  errors.push(`Cannot parse specs/reference/cli-surface.json: ${e.message}`);
  contract = null;
}
const commandsDoc = readFileSafe(path.join(root, 'docs/reference/commands.md'));
const exitCodesDoc = readFileSafe(path.join(root, 'docs/reference/exit-codes.md'));

// -- forward: contract ⊆ docs ------------------------------------------------
if (contract != null && commandsDoc != null && exitCodesDoc != null) {
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
}

// -- reverse: docs ⊆ contract --------------------------------------------------
if (contract != null && exitCodesDoc != null) {
  const knownCodes = new Set(contract.exitCodes.map((e) => e.code));
  for (const m of exitCodesDoc.matchAll(/^\| `(\d+)` \|/gm)) {
    const code = Number(m[1]);
    if (!knownCodes.has(code)) errors.push(`exit-codes.md: documents code ${code}, not in contract`);
  }
}
if (contract != null && commandsDoc != null) {
  for (const m of commandsDoc.matchAll(/^## `tome ([a-z-]+)`/gm)) {
    if (!(m[1] in contract.commands)) errors.push(`commands.md: documents \`tome ${m[1]}\`, not in contract`);
  }
}

// -- harness matrix: src/harness/mod.rs is the registry SSOT -------------------
// Machine names come from each module's `name()` literal, reached through the
// `use <module>::<CONST>;` imports in mod.rs — never hardcoded here, so a new
// harness (or a rename) fails this check until the doc table follows.
const harnessesDoc = readFileSafe(path.join(root, 'docs/using-tome/harnesses.md'));
const modRs = readFileSafe(path.join(root, '../src/harness/mod.rs'));

if (modRs != null && harnessesDoc != null) {
  const constToModule = new Map();
  for (const m of modRs.matchAll(/^use ([a-z_]+)::([A-Z_]+);$/gm)) {
    constToModule.set(m[2], m[1]);
  }

  function registryNames(registry) {
    const slice = modRs.match(new RegExp(`pub static ${registry}[^=]*=\\s*&\\[([\\s\\S]*?)\\];`));
    if (!slice) {
      errors.push(`mod.rs: could not locate the ${registry} slice`);
      return [];
    }
    const names = [];
    for (const m of slice[1].matchAll(/&([A-Z_]+)/g)) {
      const module = constToModule.get(m[1]);
      if (!module) {
        errors.push(`mod.rs: no \`use\` import found for registry entry ${m[1]}`);
        continue;
      }
      const src = readFileSafe(path.join(root, `../src/harness/${module}.rs`));
      if (src == null) continue;
      const name = src.match(/fn name\(&self\) -> &'static str \{\s*"([^"]+)"/);
      if (!name) {
        errors.push(`src/harness/${module}.rs: could not extract the name() literal`);
        continue;
      }
      names.push(name[1]);
    }
    return names;
  }

  const supported = registryNames('SUPPORTED_HARNESSES');
  const optIn = registryNames('OPT_IN_TARGETS');
  const aliases = [...modRs.matchAll(/HarnessAlias\s*\{\s*name:\s*"([^"]+)",\s*target:\s*"([^"]+)"/g)]
    .map((m) => ({name: m[1], target: m[2]}));

  // forward: every registered machine name (and alias) appears in the doc.
  for (const name of [...supported, ...optIn, ...aliases.map((a) => a.name)]) {
    if (!harnessesDoc.includes(`\`${name}\``)) {
      errors.push(`harnesses.md: missing harness \`${name}\` (registered in src/harness/mod.rs)`);
    }
  }

  // reverse: every machine name in the summary tables is still registered, and
  // the row count matches the registries (a dropped row fails loudly).
  const knownHarnesses = new Set([...supported, ...optIn]);
  const summarySection = harnessesDoc.split('## Per-harness summary')[1]?.split(/\n## /)[0] ?? '';
  const documented = new Set();
  for (const m of summarySection.matchAll(/^\|[^|`]*`([a-z0-9-]+)`/gm)) {
    documented.add(m[1]);
    if (!knownHarnesses.has(m[1])) {
      errors.push(`harnesses.md: summary table documents \`${m[1]}\`, not in the registries`);
    }
  }
  if (documented.size !== knownHarnesses.size) {
    errors.push(
      `harnesses.md: summary tables document ${documented.size} harnesses; the registries define ${knownHarnesses.size}`,
    );
  }
}

if (errors.length) {
  console.error(`DOC DRIFT — ${errors.length} problem(s):\n`);
  for (const e of errors) console.error(`  ✗ ${e}`);
  console.error('\nFix the docs, or regenerate the contract: node scripts/generate-cli-surface.mjs');
  process.exit(1);
}
console.log('✓ docs match specs/reference/cli-surface.json');
