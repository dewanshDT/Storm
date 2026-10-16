---
title: Install the server
description: Run storm-server on Linux, macOS or Windows — with the interactive installer, or by hand.
---

`storm-server` is one program. It serves the apps, keeps your notes, and runs
as a background service that starts with the machine (Linux) or when you log
in (macOS, Windows). Its default port is **8484**.

## The installer (recommended)

On **Linux** (Debian or Ubuntu) or **macOS**:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh
```

On **Windows**, in PowerShell:

```powershell
irm https://dewanshdt.github.io/Storm/install.ps1 | iex
```

It asks what to set up — the server and app, a [Runtime Host](/docs/agents/hosts/),
or everything — shows its whole plan before changing anything, and can pair
your phone over the local network at the end. Re-running it is how you
upgrade, check status, pair a device, enroll a host or uninstall; your notes
are always kept.

For scripts: `sh install.sh --yes --server --app`, `--runtime`,
`--everything`, `--dry-run`. `sh install.sh --help` lists the rest. On
Windows the switches are `-Server`, `-Runtime`, `-Yes`, `-DryRun` and
`-NoFirewall`.

`dewanshdt.github.io/Storm` is Storm's **apt repository**, which also serves
the installers. It is not this website.

## Where things live

| | Linux | macOS | Windows |
|---|---|---|---|
| Runs as | the `storm` user, systemd `storm-server` | you, a LaunchAgent | you, a logon task (no console window) |
| Notes and state | `/srv/storm` (`vaults/`, `state/`, `backups/`) | `~/Storm` | `%USERPROFILE%\Storm` |
| Settings | `/etc/storm/storm.env` | written by `up` | written by `up` |
| Web app | `/usr/share/storm/web` (from the package) | `web/` beside the binary | `web\` beside the binary |

`/etc/storm/storm.env` holds paths and the port, never a credential. See
[Configuration](/docs/reference/config/).

## By hand

### Linux

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh   # apt repository + storm-server only
sudo storm-server up
sudo storm-server status
```

Run as root with no terminal, the installer only registers the apt repository
and installs the package. `up` creates the `storm` user, writes
`/etc/storm/storm.env` (mode 600) and enables the service. To keep notes
somewhere else:

```sh
sudo storm-server up --data-root /srv/storm \
  --vault-root /path/to/vaults --state /srv/storm/state
```

`--vault-root` is a folder that *contains* vaults, one folder per vault —
never a single vault. Vaults on a NAS often need the service to run as a user
that can write the share; `up` follows the owner of the state folder.

**Notes on a network share must be mounted before Storm starts.** `up` makes
the service wait for its paths. Name the NAS by IP address or by a name that
resolves early in boot: a name that fails to resolve fails the mount.

### macOS

Download `storm-server-<version>-macos-universal.tar.gz` from
[the releases](https://github.com/dewanshDT/Storm/releases/latest), unpack it
and, as yourself (not root):

```sh
./storm-server up
```

It installs a LaunchAgent that runs as you, with your notes in `~/Storm`.
`storm-server uninstall` removes what `up` installed and keeps the notes.

### Windows

The release's `storm-server-windows-x86_64.zip` holds `storm-server.exe`, its
windowless twin `storm-serverw.exe`, and the web app. From the unpacked
folder:

```powershell
.\storm-server.exe up
```

The server runs as you at logon, with your notes in `%USERPROFILE%\Storm`.
It is not code-signed, so SmartScreen may warn.

## Check it

```sh
sudo storm-server status      # Linux; plain `storm-server status` elsewhere
```

It reports the service, a health probe, whether the firewall lets Storm in,
and the server's name, id and key fingerprint. Then
[set up your first device](/docs/install/pair/).
