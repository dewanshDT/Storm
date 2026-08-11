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
Token + URL connect detail lives in **Start the server**, not the download rows.

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
| Web | Served by Storm Server · `:8484` | Open Web Client → (`#start`) | Download web bundle |

Footer: Checksums · Release notes · All releases · Installation instructions

Optional: UA-based **Recommended** badge (mac / android / web) — does not hide
other platforms.

## MCP

Enable from Server → AI access. Tools in `apps/server/README.md`.

## Related

[[Storm Releases]] · [[Storm Website]] · [[Storm Website How it works]]
