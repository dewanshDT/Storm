---
title: Quickstart
description: Install the Storm server, set up your first device, then add your phone, an agent host and an AI app.
---

## 1. Install the server

On **Linux** (Debian or Ubuntu) or **macOS**, run the interactive installer:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh
```

It asks what to set up on this machine — the server and app, a Runtime Host
for agents, or everything — and shows its plan, every `sudo` step included,
before it changes anything. Run it again any time to upgrade, check status,
pair a device or uninstall; your notes are always kept.

On **Windows**, in PowerShell (as administrator if you want the Runtime Host):

```powershell
irm https://dewanshdt.github.io/Storm/install.ps1 | iex
```

It installs the server, the Runtime Host, or both. The server runs as you at
logon, with your notes in `%USERPROFILE%\Storm`.

Either installer asks whether devices on your local network may reach Storm
directly. Saying yes allows Storm's port from your local network only, never
from the internet.

## 2. Set up your first device

Storm has one account, protected by a password you choose on the first device.

- **In a browser:** open `http://<server>:8484` from the same network. The page
  pairs itself with the server and asks you to set your Storm's password
  (12 characters or more).
- **In the macOS or Android app:** the first device needs a setup code. The
  installer can show one for your phone; on a Linux server you can also print
  it yourself:

  ```sh
  sudo -u storm storm-server pair --state /srv/storm/state --qr
  ```

  The code works once, and only while the server has no account.

## 3. Add your other devices

Download the apps from the [Clients page](/clients) — macOS (Apple Silicon)
and Android. The web app is served by your server.

- **On your home network**, the app lists the Storms it finds under
  **On this network**. Pick yours and enter your Storm's password.
- **Otherwise**, open **Settings › Devices & access › Add a device** on a
  device that is already signed in, and scan the QR code it shows.

Every device gets its own credential, and you can sign any of them out from
**Settings › Devices & access**.

## 4. Run an agent (optional)

Agents run on a **Runtime Host**: the server's own machine or another one. If
you chose a Runtime Host in the installer, it is enrolled already. Otherwise
install `storm-runtime` on that machine, open **Settings › Hosts & default
agent › Enroll a host**, and give the host the one-time string it shows.

Then press **Start session** on a note, or **New session** in Agents. A
session runs Claude Code, OpenCode or a shell in a workspace on the host, and
you can drive its terminal from any of your devices. Log the agent CLIs in
once on the host, as the Runtime Host's own account.

## 5. Connect an AI app (optional)

Turn on **Settings › AI access › Let AI apps read your notes**; writing needs
**Let them create, edit and delete** as well. Then make a key under
**Settings › Devices & access › New key**. The key is shown once, together
with the configuration for your AI app, filled in for your server.

## Next

- [Concepts](/docs/start/concepts/) explains what each of these pieces is.
- The [install page](/install) has the by-hand steps, updates and firewalls.
