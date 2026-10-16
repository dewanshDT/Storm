---
title: Security model
description: Who can get in, how a device knows it reached your server, and what is protected on disk.
---

**Who can get in.** Storm has one account, protected by a
password hashed with Argon2id. Each device pairs — by QR, or on the LAN with
the password — and gets its own device credential and session
tokens, revocable in Settings › Devices & access. MCP clients use separate keys. Logins
are rate-limited per address.

**How a client knows it reached your server.** Pairing pins the server's
public key, and every connection proves it with a signed challenge before a
credential is sent, on the LAN and through a relay alike.

**On the network.** On the LAN the app talks to the server over plain HTTP.
Away from home it goes through a relay over `wss://`, and the relay can read
what it carries, notes and sign-ins included, until end-to-end encryption: run your own relay or use one you trust. Do not port-forward
storm-server itself; reach it through a relay instead. The installer adds a
LAN-only firewall rule only after asking ([Firewalls](/docs/install/firewalls/)).

**Credentials for integrations** are encrypted at rest on the server, so a
leaked copy of the database alone reveals none of them; that does not protect
against someone with root or a full backup.

**On disk (Linux).** The unit runs as a dedicated `storm` user with
`ProtectSystem=strict` and write access to `/srv/storm` only. That bounds where
the storage root can go: a root outside `/srv/storm` fails validation as
unwritable, so moving it elsewhere means widening `ReadWritePaths` in the unit
first.

The data tree is the service user's and its group's alone:
`/srv/storm` is 0750 and both units set `UMask=0027`, so no other local account
can read a note, an index or `state/auth.db`. Check it with
`sudo -u nobody ls /srv/storm`, which must be refused. A vault root on a NAS
keeps the modes the NAS assigns.

**One account sees every vault.** A second vault is organisation, not
isolation. Agents are the exception: an agent session writes only to the
vault chosen when it was started.

`storm-server status` prints the server's name, id, key fingerprint and the
relays it is registered with: what a device pairing from the LAN list shows,
and what a relay allowlist line must contain.
