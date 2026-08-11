---
tags: [storm, www, docs]
color: sage
---

# Storm Website

Canonical **copy** for the public marketing site (`apps/www`, milestone M16).
Edit these notes when the words should change; the Astro pages in the repo
should follow — not the other way around for prose.

Layout / tokens / BrandMark stay in code. Visual identity for www is
**SlowFlow earth** (`apps/www/src/styles/tokens.css`). Install command truth
still comes from `deploy/release-secrets.md` — keep [[Storm Website Install]]
aligned with that file.

**Positioning:** Storm as its own product. No Obsidian, Syncthing, or
competitor comparisons in marketing copy.

Companion: [[Storm Codebase Map]] · [[Storm Active Work]] · [[Storm Releases]]

## Pages (v1)

| Route | Content note | Role |
|---|---|---|
| `/` | [[Storm Website Home]] | Product story + MCP + install CTA |
| `/install` | [[Storm Website Install]] | Apt + `storm-server up` + clients |
| `/how-it-works` | [[Storm Website How it works]] | Architecture sketch |

Non-goal: a docs portal. Depth stays on GitHub / `PLAN.md` / `deploy/`.

## Hosting rules

- **Cloudflare** static deploy from this GitHub repo (root `apps/www`, build
  `npm ci && npm run build`, output `dist`). Live preview has used
  `storm.dewansh-dt.workers.dev`.
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
