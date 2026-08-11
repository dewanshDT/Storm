# storm (www)

Marketing / home site for Storm. Astro, static. Milestone **M16** in
`PLAN.md`.

## Routes

| Path | Purpose | Copy |
|---|---|---|
| `/` | Product story — knowledge, MCP, architecture, install CTA | `docs/www/home.md` |
| `/clients` | Get Storm — macOS / Android / Web downloads | `docs/www/install.md` (clients) |
| `/install` | Apt source + `storm-server up` + link to clients | `docs/www/install.md` |
| `/how-it-works` | Architecture sketch; deep links to the repo | `docs/www/how-it-works.md` |

Canonical prose also lives in the personal vault as **Storm Website** notes
(Home / Install / How it works). Edit those or `docs/www/`, then mirror into
`src/pages/`.

## Stack

- Astro (static), TypeScript, npm + `package-lock.json`
- Design tokens: **SlowFlow earth** from `docs/design_handoff_storm_design_system/`
- Brand mark from `apps/client/assets/icon/storm_icon_full.png`
- Host: Cloudflare static deploy connected to this repo — **not** the apt
  GitHub Pages root at `https://dewanshdt.github.io/Storm/`

## Positioning

Storm as its own product: self-hosted, Markdown-native knowledge with MCP for
AI agents. Do not compare to other note apps in marketing copy.

## Commands

From repo root:

```sh
make www-dev   # astro dev
make www       # npm ci + production build → apps/www/dist
```

Or:

```sh
cd apps/www && npm install && npm run dev
```

## Deploy

Connect the GitHub repo in the Cloudflare dashboard:

| Setting | Value |
|---|---|
| Root directory | `apps/www` |
| Build command | `npm ci && npm run build` |
| Output directory | `dist` |

Cloudflare builds and deploys on push to `main`. GitHub CI only *checks* the
build (`www` job in `ci.yml`); there is no deploy workflow.

## Do not

- Deploy into the apt Pages site root
- Duplicate operator runbooks — link or quote `deploy/`
- Add a full documentation corpus
- Invent install commands, platforms, or MCP tools
