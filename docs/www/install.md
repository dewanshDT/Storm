---
tags: [storm, www, docs, deploy]
route: /install
---

# Storm Website Install

Content for route **`/install`**. Parent: [[Storm Website]].

Operator truth lives in the repo — keep this note aligned with
`deploy/release-secrets.md` and `deploy/README.md` when apt lines change.

## Meta

- **Title:** Install · Storm
- **Description:** Install storm-server from the apt repository, then run storm-server up.

## Page hero

**Eyebrow:** Install

**Heading:** Get a server running

**Lede**

Preferred path: apt on Debian/Ubuntu, then one `up` command. The package ships
the binary, systemd unit, and web client.

## 1. Add the apt source

The GitHub Pages site at [dewanshdt.github.io/Storm](https://dewanshdt.github.io/Storm/)
*is* the apt repository root — do not put a marketing site there.

```sh
curl -fsSL https://dewanshdt.github.io/Storm/storm-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/storm.gpg >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main' \
  | sudo tee /etc/apt/sources.list.d/storm.list
sudo apt update
sudo apt install storm-server
```

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

## Clients

Point the Flutter app (macOS, Android, or the packaged web UI) at your server
URL and the token from `/etc/storm/storm.env`. Releases and checksums live on
[GitHub Releases](https://github.com/dewanshDT/Storm/releases). Operator detail
stays in [`deploy/README.md`](https://github.com/dewanshDT/Storm/blob/main/deploy/README.md).

## Related

[[Storm Releases]] · [[Storm Website]] · [[Storm Website How it works]]
