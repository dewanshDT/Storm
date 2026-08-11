---
tags: [storm, www, docs, deploy]
route: /install
---

# Storm Website Install

Content for route **`/install`**. Parent: [[Storm Website]].

Operator truth lives in the repo — keep this note aligned with
`deploy/release-secrets.md`, `deploy/install.sh`, and `deploy/README.md`
when apt lines change.

## Meta

- **Title:** Install · Storm
- **Description:** Install storm-server from the apt repository, then run storm-server up.

## Page hero

**Eyebrow:** Install

**Heading:** Get a server running

**Lede**

Preferred path: one bootstrap on Debian/Ubuntu, then `storm-server up`. The
package ships the binary, systemd unit, and web client.

## 1. Install

Same pattern as Tailscale: the script adds Storm’s apt key and source, then
runs `apt install storm-server`. Served from the apt repo root at
[dewanshdt.github.io/Storm](https://dewanshdt.github.io/Storm/) — that URL is
the apt root, not the marketing site.

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
```

Source of truth: `deploy/install.sh` (copied onto Pages by `apt-repo.yml`).

## 2. Start Storm

`up` creates the `storm` user when it can, writes `/etc/storm/storm.env`
(mode 600, with a generated token), and enables the systemd unit. Default data
root is `/srv/storm`.

```sh
sudo storm-server up
sudo storm-server status
```

## Custom paths

Point `--vault-root` at a directory that *contains* vaults (one folder per
vault), not at a single vault. Vaults on NFS often need the process to run as a
user that can write the mount — `up` follows the state-dir owner in that case.

```sh
sudo storm-server up \
  --data-root /srv/storm \
  --vault-root /path/to/vaults \
  --state /srv/storm/state
```

## Manual apt (optional)

Prefer reading the script over piping to shell? The bootstrap does exactly this:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/storm-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/storm.gpg >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main' \
  | sudo tee /etc/apt/sources.list.d/storm.list
sudo apt update
sudo apt install storm-server
```

## Clients

Point the Flutter app (macOS, Android, or the packaged web UI) at your server
URL and the token from `/etc/storm/storm.env`. Releases and checksums live on
[GitHub Releases](https://github.com/dewanshDT/Storm/releases). Operator detail
stays in [`deploy/README.md`](https://github.com/dewanshDT/Storm/blob/main/deploy/README.md).

## Related

[[Storm Releases]] · [[Storm Website]] · [[Storm Website How it works]]
