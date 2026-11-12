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
vault chosen when it was started. That is a rule on Storm's tools; on disk,
an agent can do whatever its host's account can
([below](#agents-run-as-the-hosts-account)).

`storm-server status` prints the server's name, id, key fingerprint and the
relays it is registered with: what a device pairing from the LAN list shows,
and what a relay allowlist line must contain.

## Agents run as the host's account

**Storm coordinates agents that run as you.** It starts them, shows their
terminals, and decides what they may do through Storm: which vault a
session may write, which integrations it gets, which deletes you approved.
Everything else an agent does, it does as an ordinary process of the
[Runtime Host's](/docs/agents/hosts/) account. On a host you run as
yourself, that account is yours. Storm does not isolate those sessions from
you or from each other. What an agent asks before acting is its own
harness's setting (its permission mode, its rules and its own sandbox).
Storm passes on the mode you pick at launch and adds no rule of its own. **Settings › Agents & hosts** says which
account each host runs as.

So, on a host you run as yourself:

- **An agent can read and change what you can**: your home, your SSH keys,
  your cloud and git tokens, your other projects.
- **Your groups are its groups.** In `docker`, it can start a container
  with root's power over the machine. In `wheel`, `sudo` or `admin`, it can
  run `sudo` wherever your sudo rules let you in without a password. If you
  want agents without that, run the host as the service, or as a separate
  account that is in neither group.
- **The host key is readable by its agents.** The host's key is in
  `identity/` in its state directory, readable only by the host's account,
  and so by every agent that account runs. That is true of the service too,
  unless it isolates sessions. Whoever holds the key can connect to your
  server as that host. If you think it got out, revoke the host in
  **Settings › Agents & hosts** and enroll it again.
- **One machine, server and host together.** On Linux the server's data
  belongs to `storm` and its group, so a host you run as yourself cannot
  read the vaults unless you join that group; never do (`install --user`
  warns if you have). On a Mac or a Windows PC where the server
  runs as you, a host as you could read the vaults on disk, so run the host
  as the service there; the installers do, given the server and the host
  together ([Runtime Hosts](/docs/agents/hosts/#never-share-an-account-with-the-vaults)).
