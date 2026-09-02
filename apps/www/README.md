# storm (www)

Marketing / home site for Storm. Astro, static. Milestone **M16** in
`PLAN.md`.

## Routes

| Path | Purpose | Copy |
|---|---|---|
| `/` | Product story — knowledge, MCP, architecture, install CTA | `docs/www/home.md` |
| `/clients` | Get Storm — macOS / Android / Web downloads | `docs/www/install.md` (clients) |
| `/install` | Apt source, `storm-server up`, **first login**, link to clients | `docs/www/install.md` |
| `/404` | Not-found page — links back to the four real routes | — |
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
make www-check # release tag is current + no stale credential copy
```

`www-check` (`scripts/check-claims.sh`) is the one that matters, because the
other two cannot fail on a wrong sentence. It checks two things a build never
will:

- **`src/data/release.ts`'s `tag` matches the newest `v*` tag.** Nothing bumps
  it automatically — `release.yml` does not touch this file — so cutting a
  release without bumping leaves every download link on `/clients` pointing at
  the previous build. **Bump `tag` as part of cutting a release.**
- **No page describes the shared token.** It stopped existing at the A10
  cutover (M19); the pages said otherwise for thirteen days while CI stayed green.
  The check bans the phrasings, not the word — saying "there is no token" is
  the point.

It runs in the `www` CI job, which is why that job checks out with
`fetch-depth: 0`.

## The lockfile cannot be regenerated on a Mac

`npm install` on macOS **prunes two hoisted entries a Linux tree needs** —
`@emnapi/core` and `@emnapi/runtime`, reached through `@img/sharp-wasm32` and
`@napi-rs/wasm-runtime`. Nothing on darwin requires them at the top level, so
npm drops them; CI then fails at `npm ci` with *"can only install packages when
your package.json and package-lock.json are in sync — Missing:
@emnapi/runtime"*, which names the symptom and not the cause.

**`--os=linux`, `--cpu=wasm32` and `--package-lock-only` do not help** — all
three still prune. The options are: regenerate on Linux (a container is
enough), or restore the two entries by hand from the last lockfile CI accepted.

So: **keep dependency changes here small and check the diff against the last
green lockfile**, package by package, not by line count. A change that should
add one subtree and instead rewrites the tree is the warning sign.

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
- Describe a credential the server no longer has. Authentication is per-device
  pairing plus sessions, and MCP clients use keys — `deploy/storm.env.example`
  and `deploy/README.md` are the source of truth, not memory
- Market anything that is not released. The relay works on `staging` and is not
  deployed; it does not belong on this site until it is
