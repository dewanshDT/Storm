---
tags: [storm, www, docs, deploy]
route: /install
---

# Storm Website Install

Content for route **`/install`**. Parent: [[Storm Website]].

Operator truth lives in the repo — keep this note aligned with
`deploy/release-secrets.md`, `deploy/install.sh`, and `deploy/README.md`
when apt lines change.

**Client downloads** are driven by a single config:
`apps/www/src/data/release.ts` (`tag` + asset names from `release.yml`).
Bump `tag` when cutting a release. UI: `GetStorm.astro`.

## Meta

- **Title:** Install · Storm
- **Description:** Install storm-server from the apt repository on Debian or Ubuntu, then download clients from GitHub Releases.

## Page hero

**Eyebrow:** 07 · Installation

**Heading:** Install Storm

## Linux · apt · Update · Start · Custom paths

Unchanged operator steps (bootstrap → `storm-server up` → custom roots).
Web client default: `http://<host>:8484` (from `deploy/storm.env.example`).

**There is no token.** `storm.env` stopped carrying one at the A10 cutover
(M19) — credentials are per device and live in `state/auth.db`. **Start the
server** says so explicitly; the connect detail is now **First login**, not the
download rows.

## First login

Site section: `/install#first-login`. Source of truth: `deploy/README.md`
(User accounts) and `apps/server/README.md`.

The first account on a server is always an **owner**. Three ways in, in the
order the page presents them:

1. **Browser** — open `http://<host>:8484` on the same network. The served page
   bootstraps its own device against the server that served it, then asks for
   the first account.
2. **macOS / Android app** — `sudo -u storm storm-server pair --state
   /srv/storm/state --qr` on the host prints a single-use pairing URI and QR.
   `pair` refuses once any account exists; after that a new device is added from
   a client that is already signed in.
3. **On the host** — `user add` / `user list` / `passwd`, the recovery path.
   **Run as `storm`, not root**: `auth.db` is created on first use, and a
   root-owned database is one the service cannot write.

### Update (apt already registered)

No `storm-server upgrade` — refresh through apt, then restart:

```sh
sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status
```

Refreshes the binary and `/usr/share/storm/web`. Leaves `/etc/storm/storm.env`
alone. Site section: `/install#update` (`release.upgradeCommand`).

## Clients — Get Storm

Section label: `07 · Clients`

**Heading:** Get Storm

**Support:** Choose a client for your device. All clients connect to your Storm
server.

Compact release manifest (rows, not cards):

| Platform | Meta | Primary action | Secondary |
|---|---|---|---|
| macOS | Apple Silicon · arm64 · tag | Download for macOS | filename |
| Android | APK · tag | Download APK | filename |
| Web | Served by Storm Server · `:8484` | Open Web Client → (`#first-login`) | Download web bundle |

Footer: Checksums · Release notes · All releases · Installation instructions

Optional: UA-based **Recommended** badge (mac / android / web) — does not hide
other platforms.

## MCP

Off by default; enable from Server → AI access, or `PUT /v1/config/mcp`. Write
tools need writable mode. An MCP client authenticates with an **MCP key**
(`stk_…`) minted in the app — shown once, revocable, accepted on `/mcp` and
nowhere else (A14). Twelve tools; list in `apps/server/README.md`.

## Related

[[Storm Releases]] · [[Storm Website]] · [[Storm Website How it works]]
