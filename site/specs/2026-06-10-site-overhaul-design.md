# Tome site overhaul — design

- **Date:** 2026-06-10
- **Status:** approved (brainstormed interactively; all sections validated)
- **Approach:** big bang — one branch carrying accuracy + IA + content rewrite + theme + landing rebuild + blog removal + drift tooling, landed as a single PR.

## 1. Why

The site (launched 2026-06-01) has three problems, confirmed by a codebase audit:

1. **Accuracy.** The docs predate Tome Phases 8 and 9. The entire authoring/conversion surface (`tome {catalog,plugin,skill} {create,convert,lint}`), the `tome meta` command group, the MCP `meta` tool, and exit codes 80–89 are undocumented. The authoring guide and concepts page describe `plugin.json` as the plugin manifest — wrong since the Phase 8 cutover to `tome-plugin.toml` (legacy-only plugins now exit 80).
2. **Quality & IA.** 14 thin pages (~50–120 lines), no worked examples, no narrative or motivation, flat prose. Authoring is buried as one page under "Catalogs".
3. **Landing page.** Static, feature-listy, doesn't show the product working, no proof, dull aesthetic.

## 2. Decisions

| Question | Decision |
| --- | --- |
| Primary audience | **Consumers first** (devs using Claude Code/Cursor/etc.); authoring is a strong secondary track |
| Docs IA | **Journey-based tracks** (Getting started → Using Tome → Authoring → Reference), rendered as "chapters" |
| Docs failures to fix | Too thin / no narrative / flat prose (context-grounding was OK) |
| Landing failures to fix | All four: no demo, weak copy, dull aesthetic, no proof |
| Visual direction | **C — the grimoire**: warm paper, serif display, chapters, marginalia, crisp monospace terminals |
| Docs theming | **Reading room**: docs inherit palette + personality (serif headings, red accent, dark code blocks) but keep clean sans body and conventional Docusaurus layout |
| Dark mode | **Midnight library** dark variant (deep ink, warm cream, candlelit); light/parchment is default |
| Display serif | **Newsreader** (self-hosted) |
| Metaphor | **Bookshelf/reading**, not lending: "add it to your bookshelf once, read it everywhere". The words "lend/lent/loan" do not appear on the site. |
| Blog | **Removed entirely** — no redirects (post was never promoted) |
| URL moves | **No redirects** anywhere; URLs move freely |
| Extras in scope | Catalog-page TODO counts, doc-drift prevention tooling, footer/navbar refresh |
| Out of scope | New blog post; tome CLI changes; logo redesign |

## 3. Accuracy fix-list (from the 2026-06-10 codebase audit)

Source of truth for the content work. Every claim in the rewrite must trace to the audited CLI surface; unverifiable claims get cut, not hedged.

**Missing surfaces (no coverage at all):**

- `tome catalog {create,convert,lint}`, `tome plugin {create,convert,lint}`, the whole `tome skill` group (`create`, `convert`, `lint`). Convert sources: claude-code, codex, cursor, opencode, cline, agent-skills; flags `--from`, `--output`/`--into` (mutually exclusive), `--force`, `--dry-run`, `--strict`, `--no-fetch` (catalog only), `--name`. Create flags: `--template`, `--bare`, `--plugin-name`, `--output`/`--into`, `--force`. Lint flags: `--autofix`, `--dry-run`, `--strict`.
- `tome meta {list,add,remove}` (`--harness` repeatable, `--global`, `--force`).
- MCP `meta` tool (install-only; fails closed with exit-code-89 semantics when no host harness is stamped); `tome mcp --harness` host stamping; the reserved `add-tome-conversion-skill` prompt.
- Exit codes 80–86 (Phase 8) and 87–89 (Phase 9): `PluginNotConverted`, `OutputExists`, `TemplateInvalid`, `SourceFormatUnrecognized`, `ConversionUnsupportedStrict`, `ValidationFoundErrors`, `ValidationStrictWarnings`, `MetaSkillNotFound`, `MetaInstallFailed`, `NoHarnessDetected`.

**Inaccuracies:**

- `docs/catalogs/authoring.md` and `docs/getting-started/concepts.md` present `plugin.json` as the plugin manifest. Reality: `tome-plugin.toml` is the manifest Tome reads; a legacy-only plugin exits 80 with a convert nudge.

**Incomplete:**

- `tome query` flags `--top-k`, `--no-rerank`, `--min-score` undocumented.
- `TODO(launch)` placeholder counts in `docs/catalogs/midnight-expert.md` — replace with real counts from the catalog repo.

## 4. Information architecture

Four chapters, 19 pages. Every existing page is rewritten; ★ = net-new page (6, one of which — plugins-and-catalogs — absorbs the deleted midnight-expert page).

```
Ch. I   Getting started      install · quickstart · concepts
Ch. II  Using Tome           plugins-and-catalogs ★ · search · harnesses ·
                             workspaces · meta-skills ★ · mcp-server · troubleshooting
Ch. III Authoring            overview ★ · create ★ · convert ★ · lint ★ ·
                             distributing  (evolved from catalogs/authoring.md)
Ch. IV  Reference            commands · exit-codes · config · security-model
```

Page notes:

- **plugins-and-catalogs ★** — day-to-day consumption lifecycle (catalog add/update/remove, plugin enable/disable/list/show, interactive picker); absorbs the midnight-expert showcase with real counts. `catalogs/midnight-expert.md` and the "Catalogs" sidebar category are deleted.
- **meta-skills ★** — what meta skills are, `tome meta list/add/remove`, scopes (project default, `--global`), the `convert-marketplace` guided skill, doctor drift report + `--fix`.
- **mcp-server** — adds the `meta` tool, host-harness stamping (`tome mcp --workspace <ws> --harness <name>` written by `harness sync`), MCP prompts including the always-on reserved `add-tome-conversion-skill`.
- **authoring/overview ★** — plugin anatomy from an author's view: `tome-plugin.toml` (strict), entry kinds, directory layout, the manifest cutover story.
- **authoring/convert ★** — the headline authoring page: migrating Claude Code marketplaces/plugins/skills, Codex projects, native SKILL.md from Cursor/OpenCode/Cline/agent-skills; detection + `--from` override; `--dry-run`; honest warn-on-unsupported and `--strict`; exit 80/83/84 stories.
- **authoring/lint ★** — rule registry behavior (never-halt, findings not crashes), `--autofix` fixpoint, `--strict`, CI usage with exit codes 85/86.
- **reference/commands** — complete surface including global flags (`--json`, `-v/-vv`, `--workspace`).
- **reference/exit-codes** — full table 0–89.

No redirects for moved/deleted URLs (decision above).

## 5. Content & voice

**Page-type skeletons:**

- **Task pages** (quickstart, search, harnesses, workspaces, meta-skills, create, convert, lint, distributing, troubleshooting): motivation lede (2–3 sentences of situation/pain) → worked example with **real captured terminal output** → variations and flags that matter → pitfalls with their exit codes → where to go next.
- **Concept pages** (concepts, authoring/overview, security-model): the problem first, then the model that solves it; one diagram each.
- **Reference pages** (commands, exit-codes, config): terse, tabular, complete. Voice lives in ledes and asides, never inside tables.

**Voice (revised 2026-06-10, supersedes the bookish docs voice):** docs read like Stripe's documentation — professional but not stiff. Plain, direct, second person, active voice, short sentences, common words. **No similes and no metaphor vocabulary in docs prose** (bookshelf/shelf/volume/chapter-as-content/library/spellbook and similar are banned; "standard library" the technical term is fine) — figurative language is hard for ESL readers, and clarity comes first. No idioms ("by default", not "out of the box"). Wry warmth only as plain statements of pain or benefit. **The grimoire personality lives in the visual design, navigation chapter labels, and the landing page (where a limited number of similes is acceptable) — not in docs prose.** Calibration sample (revised):

> You probably don't use one coding agent. You use Claude Code at work, Cursor for a side project, and whatever shipped this week. Each one stores rules, skills, and MCP config in its own format, in its own location — and none of them read each other's. Tome maintains one copy of everything: run `tome harness use <name>` and your enabled plugins are written to that agent's native configuration.

**Accuracy rules:**

- All terminal output shown in docs is captured from a locally built `tome` binary against real catalogs (midnight-expert), never invented.
- Every command/flag/exit-code claim traces to the §3 audit or the codebase.
- Honest numbers only — no invented benchmarks. The token-savings claim is expressed as the structural argument (load one ~4k-token skill instead of holding the whole library in context) with real counts from the demo catalog.

## 6. Landing page

Full rebuild of `src/pages/index.tsx` + `src/components/landing/`. Six sections:

1. **Hero** — "CHAPTER I" kicker; serif *Tome.*; one-liner: *one bookshelf for everything your coding agents know — read everywhere, loaded one skill at a time*; CTAs (Install / Read the docs); centerpiece **auto-typing terminal demo**: `catalog add` → `query` → "loaded 1 skill · 4.2k tokens — the other N stayed on the shelf" (real numbers); harness strip (Claude Code / Cursor / Codex / Gemini CLI / OpenCode).
2. **The Sprawl** (problem) — the same skill pasted into `.claude/`, `.cursor/rules`, `AGENTS.md`, `GEMINI.md`, copies drifting; always-loaded knowledge billing you all day. Visual: one skill scattered across five config trees.
3. **The Method** (how it works) — "Shelve it once. Read it everywhere." Three plates: ① Acquire (`catalog add`) ② Bind (`harness use`) ③ Consult (`query`), plus the MCP kicker: inside a harness the agent pulls skills itself, mid-task.
4. **Colophon** (proof) — book-colophon-styled spec plate: entirely local (index/embeddings/reranker on disk, no telemetry) · one Rust binary · honest exit codes · 5 harnesses with native config.
5. **Authoring onramp** — "Already have plugins? Convert them." Real `convert` → `lint` session.
6. **Final CTA** — dark ink panel: install commands + "Quickstart → 4 commands".

**Animation:** restrained. Terminal types itself (small dependency-free component, loops, respects `prefers-reduced-motion`); section annotations fade in via IntersectionObserver. No animation libraries, no parallax.

## 7. Theme & visual system

**Palettes:**

- *Parchment* (light, default): warm paper `#f4ecdc`-family background, ink `#1d1a14` text, oxblood `#8a2f1f` accent, aged-gold `#6b5d3f` details.
- *Midnight library* (dark): deep ink-brown background (not gray-blue), warm cream `#f0e9d8` text, the oxblood warmed/brightened for contrast, gold accents. Exact token values fixed during implementation, gated by the WCAG AA check (§10).

**Typography:** Newsreader (self-hosted, display + italic) for headings/ledes; the system sans stack for docs body; the system mono stack for terminals. Newsreader is the only downloaded font; no runtime Google Fonts (consistent with the no-telemetry brand).

**Docs (reading room):** Infima variable overrides in `src/css/custom.css`; serif headings; oxblood active/hover states; sidebar categories labeled "Ch. I — Getting started" etc.; code blocks dark slate in both modes.

**Chrome:** minimal navbar (*Tome.* wordmark, Docs, GitHub, version badge); swizzled footer restyled, link columns matching the four tracks. Existing logo/favicons stay; `tome-exploded.png` hero image retires.

**Implementation:** Docusaurus-native (CSS variables + the existing swizzles); no new heavy dependencies. The typed-terminal is a hand-rolled React component.

## 8. Blog removal

Delete `blog/`; disable the blog preset in `docusaurus.config.ts`; remove navbar/footer blog links; delete blog e2e coverage if any. No redirects.

## 9. Drift prevention

All inside tome-site:

- **`specs/reference/cli-surface.json`** — machine-readable contract: commands → subcommands → flags, plus the exit-code table (0–89).
- **`scripts/check-doc-drift.mjs`** — CI check asserting (a) everything in the contract appears in `reference/commands` and `reference/exit-codes`, and (b) nothing documented there is absent from the contract. Both drift directions fail the build.
- **`scripts/generate-cli-surface.mjs`** — regenerates the command half of the contract by recursively parsing `--help` from a locally built `tome` binary (sibling repo at `../tome`). Exit codes stay hand-curated (they live in `error.rs`). Regeneration is a manual local step; CI only enforces docs ≡ contract.

## 10. Verification

- `pnpm build` with `onBrokenLinks: 'throw'` remains the link gate (no internal link may 404 after the IA moves).
- Playwright: updated landing smoke (new sections render; terminal demo present; `prefers-reduced-motion` honored; dark-mode toggle works) + a docs sidebar navigation smoke.
- Drift check joins CI.
- WCAG AA contrast verified for parchment/oxblood/ink and midnight/cream combos before landing.
- Netlify deploy preview on the PR (project `tome-mcp`).

## 11. Launch dependencies & risks

- **Install instructions** (`brew install aaronbassett/homebrew-tap/tome`, `cargo install tome-mcp`) depend on the user-reserved publish hard-stops in the tome repo (crates.io publish, v0.6.0 tag, Homebrew tap PR). The site keeps documenting them, but the site should not be promoted before those run.
- **The featured catalog repo does not exist yet** (discovered 2026-06-10 during implementation: `devrelaicom/midnight-expert-tome` 404s). The site documents the *intended* public slug, exactly like the install instructions; publishing that repo (a Tome conversion of `devrelaicom/midnight-expert`) is a launch hard-stop. All counts/scores shown on the site are captured from a *local* `tome catalog convert` of the real `midnight-expert` marketplace clone — real content, real numbers; only the repo slug is forward-looking.
- **Demo/doc capture** requires a locally built `tome` binary and the midnight-expert catalog; real-model search output additionally requires downloaded models (`tome models download`). Captured during implementation.
- **Parchment legibility** — mitigated by the reading-room decision (sans body, conventional layout) and the AA gate.
- **Newsreader licensing** — SIL OFL; self-hosting is permitted.

## 12. Out of scope

- A new blog post or any blog replacement.
- Changes to the tome CLI itself (including any `--help` text fixes discovered during capture — file issues in the tome repo instead).
- Logo redesign.
- Windows support claims beyond the existing "untested".
