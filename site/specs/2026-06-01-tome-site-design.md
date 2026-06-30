# tome-site — Design Spec

- **Date:** 2026-06-01
- **Status:** Draft for review (brainstorm complete; awaiting sign-off → implementation plan)
- **Repo:** `devrel-ai/tome-site` (separate from the `tome` CLI repo)
- **Stack (already scaffolded):** Docusaurus 3.10.1 · React 19 · TypeScript · pnpm
- **Relationship:** Track 2 of the Tome public beta. Track 1 (the `tome` CLI/MCP fixes + release wrapper) is decided separately and captured in its own decisions doc → plan.

---

## 1. Overview & goals

`tome-site` is the public promo + documentation site for **Tome**, a Midnight Foundation tool: a cross-harness plugin manager for AI coding agents. The site launches with the public beta.

It has two co-equal jobs (decided: "balanced", landing weighted for the launch moment):

1. **Promo** — a landing page that earns the click: explain what Tome is and why it matters, harness-agnostic and feature-led.
2. **Docs** — documentation good enough that install → first run → useful work is flawless.

**Primary success criterion:** a developer who has never heard of Tome can land on the page, understand the value in ~10 seconds, and reach a working install + first useful action without friction.

## 2. Positioning & messaging

- **Harness-agnostic, feature-led.** Do **not** frame Tome as "migrate your Claude Code plugins" — that is the Foundation's origin story, not the user's value. Lead with capabilities.
- **Headline value props (in priority order):**
  1. **Cross-harness plugin management** — enable a plugin once; Tome serves it natively to every harness (hooks, agents, rules and all).
  2. **Search & load on demand** — semantic search finds the right skill; the MCP server loads *only* what's needed instead of dumping everything into context. **Protects the context window, cuts tokens, saves money.** (A primary differentiator — foreground it.)
  3. **Workspaces** — per-project composition; different catalogs/plugins per workspace, bound to projects.
  4. **Local & private** — runs on your machine, no telemetry; index and models stay local.
- **Supported harnesses (list as equal peers):** Claude Code, Cursor, Codex, Gemini CLI, OpenCode.
- **Maker:** "A Midnight Foundation tool." Attribution present but the product is presented as general-purpose.
- **Hero copy (locked):**
  - Wordmark: `Tome`
  - Kicker: `// cross-harness plugin manager for AI coding agents`
  - Tagline: **"One catalog, every coding agent."** (green highlighter on "every coding agent")
  - Lede: "Manage your skills, commands, agents & hooks in one place — then search and load them on demand, protecting the context window and cutting token spend, across Claude Code, Cursor, Codex, Gemini & OpenCode."

## 3. Audience & catalog-discovery strategy

- **Audience:** consumers-first messaging, with a **prominent author on-ramp** (you can't have consumers without catalogs to consume).
- **Catalog discovery for beta:** a focused **"Install the Midnight Expert catalog"** section/page — *not* a generic gallery. Midnight Expert is the flagship/seed catalog (the `devrelaicom/midnight-expert-tome` fork).
- **Deferred:** a curated multi-catalog **gallery** (data-file-driven cards + "submit yours" PR flow) graduates in once more ecosystem teams publish catalogs. A searchable registry is explicitly out of scope.

## 4. Visual identity (locked)

**Concept:** the **Owners Workshop Manual** — a light, engineered, technical-manual aesthetic (Haynes-manual *sensibility*, not literal kitsch). Tome is deliberately the **light "printed manual" sibling** to Midnight Expert's **dark "terminal."** Same family DNA (the Midnight green, mono chrome, `FIG.` markers, schematic instinct), flipped onto manual paper.

**References that informed it:** Haynes Owners Workshop Manual covers (sensibility only), the Midnight Expert site (family palette + mono technical density), pencil.dev (clean marketing structure, compatibility strip), teamprojects.uk / Inside South East (mono detailing, crop marks, blueprint metaphor).

**Palette (CSS tokens):**
```
--paper:    #f4f1e8   /* warm manual paper, faint dot grid */
--ink:      #16201b   /* near-black ink (text) */
--ink-soft: #5a655c   /* muted */
--green:    #0f9d63   /* Midnight-family accent (links, CTAs) */
--green-hi: #6ef0ad   /* highlighter swipe */
--blue:     #2b4a8a   /* blueprint accents, crop marks */
--dark:     #0c130f   /* "back cover" footer / terminal panels */
--rule:     rgba(22,32,27,.16)
```

**Type:**
- Display & body: **Space Grotesk** (deliberately less shouty than Expert's heavy grotesque).
- Technical chrome, labels, code, captions: **IBM Plex Mono**.

**Recurring devices:**
- Mono status bar, `§` section markers, `FIG.`/`DWG`/`REV` chrome, crop-mark corners (`+`) on illustration plates, the green highlighter swipe used sparingly, one green "pop" per view (primary CTA).
- **Back-cover footer:** the page flips to the dark Midnight palette at the footer (book front cover = light manual; back cover = dark), repeating the `TOME` wordmark — ties the family together.

**Illustration system:**
- Hero = a **Haynes-style exploded "tome"** (hardcover exploding into boards, fanned pages, ribbon bookmark, and gears + circuit traces inside — "book as precision machine"), warm paper, ink linework, single green accent.
- The example was generated with nanobanana (Gemini 3 Pro Image). The build uses the **same hand** for **spot illustrations** per major section (how-it-works, featured catalog, etc.), generated or commissioned to a consistent style.

## 5. Information architecture

### 5.1 Landing page `/` (custom React page, `src/pages/index.tsx`)
1. **Hero** — wordmark, tagline, lede, Install / GitHub CTAs, exploded illustration plate, "Works with" strip.
2. **§02 Highlights** — 4 benefit cards: Cross-harness management · Search & load on demand · Workspaces · Local & private.
3. **§03 How it works** — 3 steps with green-outline numerals: `Add a catalog → Enable plugins → Use any harness`, each with the representative command.
4. **§04 What you get** — the Haynes **spec sheet**: a command table (catalog, plugin, query, mcp, harness, workspace, models, doctor) with one-line functions + a "class" tag.
5. **§05 Featured catalog: Midnight Expert** — two-column: description (Compact smart-contract expertise) + a terminal panel with the real `tome catalog add devrelaicom/midnight-expert-tome` flow. Counts to be confirmed at build.
6. **§06 Get started** — terminal panel (`brew install aaronbassett/homebrew-tap/tome`, `cargo install tome-mcp`, first-run), plus a self-contained-binary / no-telemetry / platform note.
7. **§07 Publish your own** — author on-ramp band → authoring guide.
8. **Footer** — dark back-cover: repeated `TOME` wordmark, "A Midnight Foundation tool", `tome-mcp.com`, license, link columns (Product / Project / Midnight).

### 5.2 Docs `/docs` (YAGNI-trimmed for beta)
- **Getting started** — Install · Quickstart · Concepts (catalogs, plugins, the 4 entry kinds, workspaces, harnesses).
- **Using Tome** — Harnesses (one page, all five) · Semantic search · MCP server · Workspaces · Troubleshooting (`doctor`).
- **Commands** — reference for the 10 `tome` commands.
- **Catalogs** — Install Midnight Expert · Author a catalog (the author on-ramp).
- **Reference** — Security model (mechanical vs. semantic trust; "only add catalogs you trust") · Exit codes · Config.

### 5.3 Blog `/blog`
- One launch post ("Introducing Tome"). Keep the scaffold for future posts. (lean)

### 5.4 Navigation & footer
- **Navbar:** Tome wordmark; links Docs · Catalogs · Blog · GitHub; green **Install** CTA. Sticky, light, mono links.
- **Footer:** as 5.1 §8.

## 6. Content / copy principles
- Feature-led, harness-agnostic; no "Claude Code origin" framing.
- Surface the **token/context/cost** benefit of search-and-load explicitly.
- Technical, confident, manual-like voice; mono for anything spec/command-shaped.
- Every install/usage command shown must actually resolve at launch (see §11 sequencing).

## 7. Tech stack & conventions
- Docusaurus 3.10.1 classic preset (docs + blog + pages), React 19, TypeScript, **pnpm** (enforced via `scripts/enforce-pnpm.mjs`).
- **Preserve the existing supply-chain hardening:** `@lavamoat/allow-scripts`, `enforce-pnpm`, `.npmrc`, and the `supply-chain-check.yml` workflow.
- Replace all scaffold placeholders (`docusaurus.config.ts` title/tagline/url/org/links, default blog/docs/tutorial content, default favicon/social card/logo).

## 8. Theming approach
- Global theme via `src/css/custom.css` (Infima variable overrides → the palette + type tokens above), light theme primary. Decide dark-mode handling: either a tuned dark theme or pin light for v1 (lean: pin light for launch, revisit).
- Custom landing in `src/pages/index.tsx` with section components under `src/components/`.
- **Swizzle** only what's needed (navbar/footer) to achieve the manual chrome; avoid over-swizzling.
- Self-host the two web fonts (Space Grotesk, IBM Plex Mono) rather than rely on a CDN (perf + privacy/no-telemetry posture).

## 9. Hosting & deployment
- **Host:** Netlify (per-PR deploy previews; integration available).
- **Domain:** `tome-mcp.com` (DNS CNAME → Netlify; configure `url`/`baseUrl` in `docusaurus.config.ts`).
- Build: `pnpm build` → static output to `build/`. Netlify build settings + (optionally) `netlify.toml`.
- Keep the `supply-chain-check` GitHub workflow; Netlify handles deploy.

## 10. README ↔ docs (single source of truth)
- The **site docs** are canonical for install/usage.
- The `tome` repo **README** stays a strong standalone (crates.io + GitHub need it) but is **quickstart-only + links to the site** — minimal, intentional overlap, no drifting duplicate of the full docs.

## 11. Relationship to the Tome-code beta & launch sequencing
- The site's install commands (`cargo install tome-mcp`, `brew install …/tome`, `tome catalog add devrelaicom/midnight-expert-tome`) only resolve once Track 1 ships (crates.io publish, Homebrew tap, the catalog fork made public). **The site goes live with — not before — the release.**
- Shared decisions inherited from Track 1: crate name `tome-mcp` (binary `tome`), the five supported harnesses, the security/trust messaging, platform support (Linux + macOS only — state it).

## 12. Out of scope (beta)
- Catalog **gallery** / searchable registry (featured single catalog only).
- Blog beyond the launch post.
- Windows (state Linux + macOS only).
- i18n / localization.
- Any dynamic backend (the site is fully static).

## 13. Open items to resolve during implementation
- Real **Midnight Expert counts** (plugins / skills / agents) for §05.
- Final **spacing, polish, responsive** behavior (the mockup is low-fi; refine in build).
- **Spot illustrations** for non-hero sections (consistent hand).
- **Dark-mode** decision (pin light vs. tuned dark).
- Exact **docs content** authoring (this spec defines structure, not page bodies).
- Favicon, social/OG card, logo lockup in the manual style.

## 14. Success criteria
- Value legible in ~10s; balanced promo + docs.
- Visual identity reads as a Midnight sibling, distinct from Midnight Expert.
- Every shown command resolves at launch.
- Lighthouse-clean static build on Netlify at `tome-mcp.com`; no telemetry/third-party calls.
- Supply-chain hardening intact.

## 15. Decision log (from the brainstorm)
- Scope decomposed: Tome-code beta (plan) vs tome-site (this brainstorm); thin seam = shared install instructions + launch sequencing + catalog discovery.
- Primary job: **balanced** (C), landing weighted for launch.
- Audience: **consumers-first + prominent author on-ramp** (C).
- Discovery: **featured Midnight Expert catalog** now; gallery later (A → B).
- Identity: **sub-brand / harmonized** (C); manual-paper "workshop manual" sibling to Midnight Expert's terminal; v1 too-kitsch and v2 too-clone rejected; v3–v5 locked the direction.
- Hero illustration: exploded "tome" cutaway (nanobanana-generated example; consistent spot-illustration system in build).
- Hosting **Netlify**; domain **tome-mcp.com**.
- Stack stays Docusaurus (scaffold already in place); preserve supply-chain hardening.
