# Deploying the Tome docs site

The site lives at `site/` in the `devrelaicom/tome` monorepo and deploys via the
existing **`tome-mcp`** Netlify project to `tome-mcp.com`. Build config is in
`site/netlify.toml` (`base = "site"`, `command = "pnpm build"`, `publish = build`).

## One-time reconnect (operator only)

When moving the site from the old `devrelaicom/tome-site` repo into this monorepo:

1. In the Netlify UI, open the **`tome-mcp`** project → **Site configuration →
   Build & deploy → Continuous deployment**.
2. **Link to a different repository** → select `devrelaicom/tome`, branch `main`.
3. Netlify reads `site/netlify.toml`, so the base directory (`site`), build
   command (`pnpm build`), and publish directory (`build`) are picked up
   automatically — leave the UI fields blank or matching.
4. Confirm the **custom domain** `tome-mcp.com` is still attached to this project
   (Domain management). No DNS change is needed — the project is unchanged, only
   its connected repo.
5. Trigger a deploy and verify `https://tome-mcp.com` serves the new build.

## Local development

```bash
sfw pnpm install
pnpm start      # dev server
pnpm build      # production build → build/
```
