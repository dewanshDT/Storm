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
- **Description:** Install Storm on Linux, macOS or Windows with the interactive installer, then pair your devices and download the apps.

## Page hero

**Eyebrow:** 08 · Installation

**Heading:** Install Storm

## Linux and macOS · Windows · Update · Start by hand · Custom paths

- **Linux and macOS:** the interactive installer, `curl -fsSL
  https://dewanshdt.github.io/Storm/install.sh | sh` (`release.installCommand`).
- **Windows** (`/install#windows`, decision 99): `irm
  https://dewanshdt.github.io/Storm/install.ps1 | iex`
  (`release.windowsInstallCommand`) — server, Runtime Host, or both; server
  at logon with notes in `%USERPROFILE%\Storm`, the host as the
  `StormRuntime` service. Not code-signed: SmartScreen may warn.
- **Update:** re-run the installer, `… | sh -s -- upgrade`
  (`release.installerUpgradeCommand`); the apt commands below stay for a
  server installed by hand.
- **Start the server by hand:** `curl … | sudo sh` (apt repo + storm-server
  only) → `storm-server up` → custom roots.
Web client default: `http://<host>:8484` (from `deploy/storm.env.example`).

**There is no token.** `storm.env` stopped carrying one at the A10 cutover
(M19) — credentials are per device and live in `state/auth.db`. **Start the
server** says so explicitly; the connect detail is now **First login**, not the
download rows.

## First login

Site section: `/install#first-login`. Source of truth: `deploy/README.md`
(The account) and `apps/server/README.md`.

Storm is single-user (decision 82): one account, with a password set when the
first device pairs. More devices, MCP keys and Runtime Hosts are added from a
signed-in client, under Settings. Three ways in, in the order the page presents
them:

1. **Browser** — open `http://<host>:8484` on the same network. The served page
   bootstraps its own device against the server that served it, then asks you
   to set your Storm's password.
2. **macOS / Android app** — `sudo -u storm storm-server pair --state
   /srv/storm/state --qr` on the host prints a single-use pairing URI and QR.
   `pair` refuses once the account exists; after that a new device is added
   with Settings › Devices & access › Add a device on a signed-in client.
3. **On the host** — `storm-server passwd --state /srv/storm/state`, the
   recovery path for a forgotten password (it creates the account on a server
   that has none).
   **Run as `storm`, not root**: `auth.db` is created on first use, and a
   root-owned database is one the service cannot write.

### Update by hand (apt already registered)

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

Off by default; enable from Settings › AI access, or `PUT /v1/config/mcp`.
Write tools need *Let them create, edit and delete*. An MCP client
authenticates with an **MCP key** (`stk_…`) minted under Settings › Devices &
access — shown once, revocable, accepted on `/mcp` and nowhere else (A14). The
tool list is on `/#mcp`, checked against `apps/server/src/mcp.rs` by `make
www-check`.

## Related

[[Storm Releases]] · [[Storm Website]] · [[Storm Website How it works]]
