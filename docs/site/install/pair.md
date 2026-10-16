---
title: Pair a device
description: Set up your first device, add the rest, and recover a forgotten password.
---

Storm has **one account**, protected by a password. Every app you sign in is a
**device** with its own credential, which you can sign out on its own. When a
device connects, it first checks that the server proves the identity it
pinned at pairing, on your network and through a relay alike, before it sends
any credential.

## The first device

The password is chosen on the first device, so it needs no password to enter.

- **A browser** on the same network: open `http://<server>:8484`. The page
  pairs itself with the server that served it and asks you to choose the
  password (12 characters or more).
- **The macOS or Android app**: scan a single-use setup code. The installer
  can show one at the end; or print it on the server:

  ```sh
  sudo -u storm storm-server pair --state /srv/storm/state --qr
  ```

  Without `--qr` it prints the code as a `storm://pair?…` line you can paste
  into the app's **Enter address manually**. `pair` works only while the
  server has no account.

## Every device after that

- **On your home network**, the app lists the Storms it can see under
  **On this network**. Pick yours and enter the password.
- **Otherwise**, on a device that is already signed in, open
  **Settings › Devices & access › Add a device** and scan the QR code. Each
  code works once.

On a network the computer treats as **public** (a café, a hotel), the server
does not announce itself and refuses password pairing; scan a QR code instead.
To turn discovery and password pairing off, use **Settings › Connection ›
Discoverable on this network**, or set `STORM_DISCOVERY=0`
([Configuration](/docs/reference/config/)), which wins over the app's
setting. QR pairing still works.

## Signing devices out

**Settings › Devices & access** lists every signed-in device. Removing one
signs it out at once; it cannot connect again until it pairs again.

## A forgotten password

Reset it on the server. On Linux, run it as the service user, never as root —
a database created by root is one the service cannot write:

```sh
sudo -u storm storm-server passwd --state /srv/storm/state
```

It prompts without echo, or reads `--password-stdin`; there is never a
password argument, because arguments are visible to every user on the machine.
On a server with no account yet, the same command creates it.
