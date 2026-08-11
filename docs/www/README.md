---
tags: [storm, www, docs]
color: sage
---

# Storm Website

Canonical **copy** for the public marketing site (`apps/www`, milestone M16).
Edit these notes when the words should change; the Astro pages in the repo
should follow — not the other way around for prose.

Layout / tokens / BrandMark stay in code
(`docs/design_handoff_storm_design_system/`, `apps/www/src/`). Install command
truth still comes from `deploy/release-secrets.md` — keep [[Storm Website Install]]
aligned with that file.

Companion: [[Storm Codebase Map]] · [[Storm Active Work]] · [[Storm Releases]]

## Pages (v1)

| Route | Content note | Role |
|---|---|---|
| `/` | [[Storm Website Home]] | Brand-first hero + why it exists |
| `/install` | [[Storm Website Install]] | Apt source + `storm-server up` |
| `/how-it-works` | [[Storm Website How it works]] | Short self-host sketch |

Non-goal: a docs portal. Depth stays on GitHub / `PLAN.md` / `deploy/`.

## Hosting rules

- **Cloudflare Pages** connected to this GitHub repo (dashboard: root
  `apps/www`, build `npm ci && npm run build`, output `dist`). Cloudflare
  owns build + deploy on push.
- **Never** publish into `https://dewanshdt.github.io/Storm/` — that URL is the
  **apt repository root**. Overwriting it breaks every `sources.list` line
  (decision 49 in `PLAN.md`).

## Repo pointers

```
apps/www/                 Astro app
docs/www/                 Same page copy, checked into the monorepo
make www / make www-dev   build / local preview
.github/workflows/ci.yml  www build check (not deploy)
```

## How to change copy

1. Edit the page note (or the matching file under `docs/www/`).
2. Mirror the prose into the matching `apps/www/src/pages/*.astro`.
3. Tick [[Storm Active Work]] / `PLAN.md` if the milestone status moved.
