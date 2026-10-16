---
title: Upgrade and uninstall
description: Keep Storm current, and remove it without losing notes.
---

## Upgrade

**Re-run the installer.** It shows what is installed, offers to upgrade it,
and never deletes notes or workspaces:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh -s -- upgrade
```

On Windows, run `install.ps1` again.

**A server installed by hand on Linux** upgrades through apt — there is no
`storm-server upgrade`. The package replaces the binary and the web app and
leaves `/etc/storm/storm.env` alone; restart so the new binary is listening:

```sh
sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status
```

**Upgrade the server and its Runtime Hosts together.** New features often need
both sides; with an older host the app hides what the host cannot do.

**The native apps** are separate downloads: install the new build on each
device. The web app comes with the server.

## Uninstall

Re-run the installer and choose uninstall; your notes are always kept. By
hand: `sudo apt remove storm-server` on Linux, `storm-server uninstall` on
macOS. A Runtime Host is removed with `sudo apt remove storm-runtime` on
Linux, or `storm-runtime uninstall` on macOS and Windows (add `--purge` to
also remove its state); [workspaces are always kept](/docs/agents/hosts/).
