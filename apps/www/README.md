# storm (www)

Marketing / home site for Storm. Astro, static. Milestone **M16** in
`PLAN.md`.

## Routes

| Path | Purpose | Copy |
|---|---|---|
| `/` | Home — brand-first hero, CTA to Install + GitHub | `docs/www/home.md` |
| `/install` | Apt source + `storm-server up` | `docs/www/install.md` |
| `/how-it-works` | Short self-host sketch; deep links to the repo | `docs/www/how-it-works.md` |

Canonical prose also lives in the personal vault as **Storm Website** notes
(Home / Install / How it works). Edit those or `docs/www/`, then mirror into
`src/pages/`.

## Stack

- Astro (static), TypeScript, npm + `package-lock.json`
- Design tokens from `docs/design_handoff_storm_design_system/` (Storm dark)
- Brand mark from `apps/client/assets/icon/storm_icon_full.png`
- Host: Cloudflare Pages connected to this repo (build + deploy on push) —
  **not** the apt GitHub Pages root at `https://dewanshdt.github.io/Storm/`

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

Connect the GitHub repo in the Cloudflare Pages dashboard:

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
