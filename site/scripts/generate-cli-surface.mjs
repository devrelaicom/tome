#!/usr/bin/env node
// Regenerates the `commands` half of specs/reference/cli-surface.json by
// recursively parsing `--help` from a locally built tome binary. The
// `exitCodes` half is hand-curated (it lives in tome's src/error.rs) and is
// preserved verbatim.
//
// Usage: node scripts/generate-cli-surface.mjs [path-to-tome-binary]
import {execFileSync} from 'node:child_process';
import {readFileSync, writeFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const contractPath = path.join(root, 'specs/reference/cli-surface.json');
const bin = process.argv[2] ?? path.join(root, '../target/release/tome');

// clap `global = true` flags propagate onto every subcommand's `--help`, so
// they must NOT be listed per-command — they belong in `globalFlags`. (The one
// deliberate exception is `--verbose`/`-v`, which is also `global = true` in
// cli.rs but IS kept per-command, because it affects each command's output.)
const IGNORED_FLAGS = new Set([
  '--help',
  '--version',
  '--json',
  '--no-color',
  '--non-interactive',
  '--workspace',
]);
const IGNORED_SUBCOMMANDS = new Set(['help']);

function help(args) {
  return execFileSync(bin, [...args, '--help'], {encoding: 'utf8'});
}

// clap v4 help: sections are "Usage:", "Commands:", "Options:"; entries are
// indented and the first token is the name.
function parseSection(text, section) {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => l.trim() === `${section}:`);
  if (start === -1) return [];
  const out = [];
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i];
    if (/^\S/.test(line)) break; // next section
    const m = line.match(/^\s{2,}(\S+)/);
    if (m) out.push({name: m[1], line});
  }
  return out;
}

function flagsOf(text) {
  const found = new Set();
  for (const {line} of parseSection(text, 'Options')) {
    // Scan ONLY the option-name head — the part before the 2+-space gap clap
    // uses to separate a flag from its description. A real `--flag` always sits
    // in the head; scanning the whole line would harvest `--xxx` tokens out of
    // description prose (e.g. the global `--non-interactive` help text names
    // `--force`/`--yes`, which would then be wrongly attributed to every
    // subcommand).
    const head = line.replace(/^\s+/, '').split(/\s{2,}/)[0];
    for (const m of head.matchAll(/--[a-z][a-z0-9-]*/g)) {
      if (!IGNORED_FLAGS.has(m[0])) found.add(m[0]);
    }
  }
  return [...found].sort();
}

const top = help([]);
const commands = {};
for (const {name: group} of parseSection(top, 'Commands')) {
  if (IGNORED_SUBCOMMANDS.has(group)) continue;
  const groupHelp = help([group]);
  const subcommands = {};
  for (const {name: sub} of parseSection(groupHelp, 'Commands')) {
    if (IGNORED_SUBCOMMANDS.has(sub)) continue;
    subcommands[sub] = flagsOf(help([group, sub]));
  }
  commands[group] = {subcommands, flags: flagsOf(groupHelp)};
}

const existing = JSON.parse(readFileSync(contractPath, 'utf8'));
const next = {...existing, commands};
writeFileSync(contractPath, `${JSON.stringify(next, null, 2)}\n`);
console.log(`Regenerated ${Object.keys(commands).length} command groups into ${path.relative(root, contractPath)}`);
console.log('Exit codes preserved (hand-curated — source: tome/src/error.rs).');
