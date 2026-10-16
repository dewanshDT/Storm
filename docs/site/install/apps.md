---
title: Get the apps
description: Storm for macOS, Windows, Linux, Android and the web.
---

All of them connect to your own server. Downloads are on the
[Clients page](/clients) and on [GitHub Releases](https://github.com/dewanshDT/Storm/releases/latest).

| | What | Notes |
|---|---|---|
| **macOS** | `Storm-<version>-macos-arm64.zip` (Apple Silicon) | Ad-hoc signed, not notarized: macOS may ask you to allow it on first launch. |
| **Windows** | `storm-app-windows-x86_64.zip` (Windows 10/11, x64) | Unzip anywhere and run `storm.exe`; nothing else to install. Not code-signed: SmartScreen may warn ("More info", then "Run anyway"). |
| **Linux** | `storm-app_<version>-1_amd64.deb` (Debian, Ubuntu) or `storm-app-<version>-x86_64-linux.tar.gz` (any distribution, x86_64) | Needs GTK 3 and libsecret; LAN discovery needs Avahi. |
| **Android** | `storm-<version>.apk` | A direct APK: Android may ask you to allow installing from this source. |
| **Web** | Served by your server | Open `http://<server>:8484`. Nothing to install. |

**The installers put the app in for you.** On Windows, in PowerShell (no
administrator needed for the app alone):

```powershell
& ([scriptblock]::Create((irm https://dewanshdt.github.io/Storm/install.ps1))) -App
```

On Linux, `install.sh --app` installs the `storm-app` package on Debian and
Ubuntu (`sudo apt install storm-app` once the Storm apt repository is set up),
and anywhere else unpacks the tarball into `~/.local/share/storm-app` with a
launcher entry, without root:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh -s -- --app
```

On Linux without a running Secret Service (gnome-keyring or KWallet), Storm
keeps its sign-in in its own settings instead, and Settings › Advanced says so.

Only one Storm runs at a time on a desktop: opening it again brings the
running window forward.

**Update the native apps yourself** when a release lands: download the new
build and replace the old one, or run the installer again (on Linux with apt,
`sudo apt upgrade` does it). The web app updates with the server.

Next: [pair the device](/docs/install/pair/).
