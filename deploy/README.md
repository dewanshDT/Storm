# Deploying Storm

> **The user-facing guide is the docs** — [`docs/site/`](../docs/site/), served at
> `/docs` on the website: installing, pairing, firewalls, Runtime Hosts,
> MCP clients, backups and security. This file keeps packaging, upgrades of
> older installs, the manual setup and the relay.

A single static binary, a systemd unit, and a nightly backup. No container, no
runtime dependencies — the server is a ~5 MB musl binary that runs on a bare
Ubuntu or Debian box with nothing installed.

See decision 8 for why this isn't a Docker image. Prefer the
packaged install (`apt install storm-server` then `sudo storm-server up`) once
a release exists; the steps below are the manual equivalent. Secrets and the
**clean-install** runbook are in [release-secrets.md](release-secrets.md).
After apt works, install under `/srv/storm` and remove the hand-rolled tree /
`~/storm-m15-cutover` — do not treat the NFS cutover kit as permanent.

## First-time setup (packaged)

**The interactive installer** does all of this — server, Runtime Host, app,
pairing, enrollment — on Linux or macOS (decision 84):

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh
```

The manual steps below stay for operators who want them. Run as root with no
terminal and no arguments (`curl … | sudo sh`), the same script still does
the old apt bootstrap, so existing automation keeps working.

```sh
# Bootstrap apt (key + source + install) — same idea as Tailscale:
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
sudo storm-server up                    # data root = /srv/storm
sudo storm-server status
```

`install.sh` lives in `deploy/install.sh` and is published at the apt Pages
root by `apt-repo.yml`. Manual key + `sources.list` steps are in
[release-secrets.md](release-secrets.md) if you prefer not to pipe to shell.

`up` creates the `storm` user, writes `/etc/storm/storm.env` (mode 600; it
holds paths and the port, never a credential), and `systemctl enable --now
storm-server`. The web client
lives at `/usr/share/storm/web` and upgrades with the package.

## Updating (packaged)

There is no `storm-server upgrade`. Once the apt source is registered, refresh
through apt. The package replaces the binary and `/usr/share/storm/web`; it
does not rewrite `/etc/storm/storm.env`. Restart so the new binary is what is
listening:

```sh
sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status
```

**A vault root on NFS (or any network mount) must be mounted before Storm
starts** (decision 73). The unit orders itself after `remote-fs.target`, and
`storm-server up` adds `RequiresMountsFor=` for its paths. An install set up by
an older `up` gets the second by re-running `up` with the same flags, or with:

```sh
sudo systemctl edit storm-server     # add these two lines, then save:
# [Unit]
# RequiresMountsFor=/mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
```

Name an NFS server by IP or a name that resolves before DNS is fully up: a
`nas.lan` that fails to resolve at boot fails the mount outright.

**Nightly backups never ran from the package before this fix** (decision 75):
the unit named `/usr/local/bin/storm-backup.sh`, which the `.deb` does not
install, and ran as `storm` without being able to read `/etc/storm/storm.env`.
On an install set up by an older `up`, until the fixed package is installed,
give it the right command, environment and user (use the paths and user from
`/etc/systemd/system/storm-server.service.d/data-root.conf`):

```sh
sudo systemctl edit storm-backup     # add, then save:
# [Unit]
# RequiresMountsFor=/mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
# [Service]
# ExecStart=
# ExecStart=/usr/bin/storm-backup.sh
# EnvironmentFile=/etc/storm/storm.env
# User=dewansh
# Group=dewansh
# ReadWritePaths=/srv/storm /mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
sudo systemctl start storm-backup.service && journalctl -u storm-backup -n 12
```

**Up to and including 0.2.9, every local account could read Storm's data**
(decision 76): every note, every index, `vaults.json`, and `state/auth.db`,
which holds the password hashes. `postinst` created `/srv/storm` 0755 and the
unit set no `UMask`, so everything the server wrote came out world-readable.
The fixed package sets `UMask=0027` in both units and removes other-access
from `/srv/storm`, but only from the root directory: it cannot know what an
operator's tree holds below it. An existing install needs this once; until the
fixed package is installed, the drop-ins are what set the `UMask`.

**Run it as one `sudo sh -c`, not as separate `sudo` lines.** On prod, a pasted
block of separate lines stopped the server at the first password prompt, and
the prompt swallowed the remaining lines, including the `start`. This form asks
for the password once, and it ends by starting the server whatever happened
before:

```sh
sudo sh -c '
systemctl stop storm-server
chmod 0750 /srv/storm /srv/storm/state
# The local tree only. -xdev keeps find off any other filesystem, so a vault
# root mounted under /srv/storm is never touched (the NAS sets its modes).
# Symlinks are skipped, never followed.
find /srv/storm -xdev ! -type l -exec chmod o-rwx {} +
mkdir -p /etc/systemd/system/storm-server.service.d /etc/systemd/system/storm-backup.service.d
printf "[Service]\nUMask=0027\n" > /etc/systemd/system/storm-server.service.d/umask.conf
cp /etc/systemd/system/storm-server.service.d/umask.conf /etc/systemd/system/storm-backup.service.d/umask.conf
systemctl daemon-reload
systemctl start storm-server
'
systemctl show -p UMask storm-server               # UMask=0027
sudo -u nobody cat /srv/storm/state/auth.db        # Permission denied
```

Only other-access is removed, so the service user (`User=` in `data-root.conf`)
keeps every bit it had. **A vault root outside `/srv/storm` is deliberately
left alone:** on a NAS share other machines write it, and its modes are the
NAS's to set. An NFS server that assigns its own modes ignores the client's
umask (prod's assigns 664), so keeping the share from other local accounts is
a NAS-side change. Once the fixed package is installed the `umask.conf`
drop-ins are redundant and harmless. If another local account ever needs to
read the data, add it to the service user's group; never restore other-access.

**Upgrades up to and including 0.2.8 disabled the service.** The package's
`prerm` ignored whether it was being removed or upgraded, so each `apt upgrade`
left `storm-server` and `storm-backup.timer` disabled. Both kept running until
the next reboot, which the server did not survive, and the backups stopped.
Check with `systemctl is-enabled storm-server storm-backup.timer`. The fix
arrives with the first release after 0.2.8, and that upgrade re-enables both
once, on any box `storm-server up` configured. Until then, after every upgrade:

```sh
sudo systemctl enable --now storm-server storm-backup.timer
```

Native clients (macOS zip, Android APK) are separate downloads from
[GitHub Releases](https://github.com/dewanshDT/Storm/releases) — upgrade those
on each device when a new release lands.

**NFS / non-`storm` User:** the package `postinst` used to
`chown -R storm:storm /srv/storm` on every upgrade, which breaks a unit that
`up` configured as the state-dir owner (Permission denied on
`vaults.json`). Fixed in postinst (skip chown when `vaults.json` already
exists). If an older upgrade already flipped ownership, restore it to the
`User=` in `/etc/systemd/system/storm-server.service.d/data-root.conf`:

```sh
sudo chown -R dewansh:dewansh /srv/storm   # match your drop-in’s User=
sudo systemctl restart storm-server
sudo storm-server status
```

## First-time setup (manual)

On the server, once:

```sh
sudo useradd --system --home /srv/storm --shell /usr/sbin/nologin storm
sudo mkdir -p /srv/storm/{vaults,state,backups} /etc/storm
sudo chown -R storm:storm /srv/storm

sudo cp deploy/storm-server.service /lib/systemd/system/
sudo cp deploy/storm-backup.service deploy/storm-backup.timer /lib/systemd/system/
sudo cp deploy/storm.env.example /etc/storm/storm.env
sudo chmod 600 /etc/storm/storm.env
sudoedit /etc/storm/storm.env            # set the paths and the backup path
sudo systemctl enable --now storm-server storm-backup.timer
```

The env file holds paths and the port, never a credential: the first device
pairs as in [Pair a device](../docs/site/install/pair.md).

Then from your machine:

```sh
make deploy HOST=you@your-server
```

## Vaults and the storage root

Moved to the docs: [Vaults and folders](../docs/site/notes/vaults.md).

## Migrating a single-vault install

Existing deployments have `/srv/storm/vault` and `/srv/storm/state/index.db`.
The move is manual, and worth a backup first — `note_versions` is the merge
base and cannot be rebuilt from the markdown.

```sh
sudo /usr/bin/storm-backup.sh          # and check it wrote something
sudo systemctl stop storm-server
sudo -u storm mkdir -p /srv/storm/vaults
sudo -u storm mv /srv/storm/vault /srv/storm/vaults/personal
sudoedit /etc/storm/storm.env                # STORM_VAULT -> STORM_VAULT_ROOT
sudo systemctl start storm-server
```

On that first boot the server registers `personal` and moves
`state/index.db` into `state/<vault-id>/index.db`, keeping version history.
Confirm before pointing any client at it:

```sh
curl -s -H "Authorization: Bearer $TOKEN" localhost:8484/v1/vaults
```

`--vault` still works for one release: the storage root becomes that
directory's *parent* and the one directory is registered. It logs a deprecation
warning, and refuses to start if the path is missing rather than coming up with
zero vaults.

## Importing a real vault

Moved to the docs: [Vaults and folders › Bringing in an existing vault](../docs/site/notes/vaults.md#bringing-in-an-existing-vault).

## Everyday use

```sh
make deploy HOST=...     # cross-compile, push binary + web client, restart
make deploy-check HOST=... # health, service state, next backup
journalctl -u storm-server -f
```

`make deploy` restarts the service and fails loudly if it doesn't come back.

## The account

Moved to the docs: [Pair a device › A forgotten password](../docs/site/install/pair.md#a-forgotten-password).

Upgrading a 0.3.x server keeps its oldest active owner and removes every
other account, after writing `state/auth.db.pre-v6`. If it has no active owner
it refuses to start; name the account to keep with
`sudo -u storm storm-server single-user --keep <username> --state /srv/storm/state`.

## Firewalls

Moved to the docs: [Firewalls](../docs/site/install/firewalls.md). The normative spec is
the vault note *Reachability on Laptops and Desktops* (decision 119).

## Backups

Moved to the docs: [Backups and restore](../docs/site/operate/backups.md).

### Restoring

Moved to the docs: [Backups and restore › Restoring](../docs/site/operate/backups.md#restoring).

## Relay (optional)

`storm-relay` lets clients reach a server from outside its network
(`docs/srp-v1.md`). It is a **separate package** in the same apt repository,
and nothing needs it: `storm-server` does not depend on it and works the same
without one. It holds no vault data and authenticates no clients (R5, R12);
a client's credential rides inside the tunnel to the origin.

```sh
sudo apt install storm-relay
sudoedit /etc/storm-relay/storm-relay.env   # set STORM_RELAY_PUBLIC_BASE
sudo systemctl enable --now storm-relay
journalctl --namespace=storm-relay -u storm-relay -f   # its own journal, kept 7 days
```

The package does not start the relay, because a relay with the wrong public
address hands every server a URL that goes nowhere. It runs as its own
`storm-relay` user, never `storm`, so it cannot read a vault on a shared box,
and it listens on `127.0.0.1:8486` (storm-server owns 8484).

**Binding server keys.** Set exactly one of these in the env file; setting
both stops the relay.

- `STORM_RELAY_BINDINGS` (the default): trust-on-first-use, persisted to
  `/var/lib/storm-relay/bindings`. The first key to register a `server_id` owns
  it. Safe only where you control who can reach the relay, so the relay
  refuses to start in this mode on a public or wildcard bind unless you pass
  `--allow-public-tofu` (decision 116j).
- `STORM_RELAY_ALLOWLIST`: only listed keys register. **Use this on a public
  address.** Same format as the bindings file, so a bindings file you trust can
  be copied over as the allowlist.

**Revoking a server** on an allowlist relay: remove its line and run `sudo
systemctl reload storm-relay`. The relay re-reads the file and ends that
server's live trunk at once, not only its next registration (decision 116k).

**Limits** (all on by default; `storm-relay --help` names the flags): `/register`
attempts and `HELLO`s per source address, an IPv6 /64 counting as one;
streams per server and opens in flight per client; 32 MiB unread per stream.
A client past one gets `rate_limited` or `client_backlogged`; the server's
trunk and every other client carry on.

**Logs** go to the relay's own journal namespace, kept at most 7 days
(`/etc/systemd/journald@storm-relay.conf`, decision 114b):
`journalctl --namespace=storm-relay -u storm-relay`. They hold lifecycle
events and error codes, server ids and trunk ids; never a client's address, a
payload or a credential.

**If a server's identity changes** (a restore without `state/identity/`, see
*Backups*), its key changes with it. Every paired device must pair again, and
every allowlist line naming the old key must be replaced; until then the relay
refuses the server with `auth_failed`.

`/var/lib/storm-relay` is the relay's only state, and it **cannot be rebuilt**.
Lose the bindings file and every `server_id` is open to whoever registers
next. Back it up. Purging the package deliberately leaves it in place.

**On a public address, the relay terminates TLS itself** (decision 71). Do not
put Caddy or nginx in front of it. The relay takes each client's address from
the socket, never from a header, because a header is something the client
wrote. Behind a proxy every client would be the proxy: one `HELLO` rate limit
shared by everybody, and one `relay_peer_ip` for the origin's login limiter,
so one noisy client locks everyone out.

With certbot, standalone for the first certificate and a deploy hook for every
renewal:

```sh
sudo certbot certonly --standalone -d relay.example.com
sudo tee /etc/letsencrypt/renewal-hooks/deploy/storm-relay >/dev/null <<'HOOK'
#!/bin/sh
# certbot's own files are root-only. Copy them where the relay can read them.
install -d -m 0750 -g storm-relay /etc/storm-relay/tls
install -m 0640 -g storm-relay "$RENEWED_LINEAGE/fullchain.pem" /etc/storm-relay/tls/
install -m 0640 -g storm-relay "$RENEWED_LINEAGE/privkey.pem" /etc/storm-relay/tls/
systemctl reload storm-relay 2>/dev/null || true
HOOK
sudo chmod 755 /etc/letsencrypt/renewal-hooks/deploy/storm-relay
sudo RENEWED_LINEAGE=/etc/letsencrypt/live/relay.example.com \
    /etc/letsencrypt/renewal-hooks/deploy/storm-relay
```

Then in `/etc/storm-relay/storm-relay.env`:

```sh
STORM_RELAY_BIND=0.0.0.0:443
STORM_RELAY_PUBLIC_BASE=wss://relay.example.com
STORM_RELAY_TLS_CERT=/etc/storm-relay/tls/fullchain.pem
STORM_RELAY_TLS_KEY=/etc/storm-relay/tls/privkey.pem
# and an allowlist rather than TOFU, as above
```

`systemctl reload storm-relay` swaps the certificate without dropping a trunk;
a reload that fails (a half-written file, a key that does not match) keeps the
old one and says so in the journal. The relay refuses to start with `--tls-cert`
and a `ws://` public base, and warns about a `wss://` base without
`--tls-cert`. **storm-server trusts the public web PKI only**, so the relay
needs a real certificate; a self-signed one will not register.

**Pointing a server at it.** In the app, Settings › Connection › Relays ›
Add relay. Or, as the owner, `PUT /v1/config/relays` with
`{"relays": ["wss://relay.example.com"]}`.
The server connects to an added relay and disconnects from a removed one (with
a `DEREGISTER`) right away, without a restart; relays in both lists are left
alone (decision 74). `GET /v1/server` lists the relays it has actually
registered with, which is the check that it worked; the `PUT` itself returns
before any connection is attempted. Clients learn the relay from the server when they pair.

### On Cloudflare instead (optional)

`apps/relay-cf` is the same relay as a Cloudflare Worker with Durable
Objects, for an operator who would rather not run a public box (decision
108). It deploys to **your own** Cloudflare account, on the Free plan, with
`wrangler`; nothing in Storm's release or apt pipeline touches it. The runbook
is `apps/relay-cf/DEPLOY.md`. In short:

1. `cd apps/relay-cf && npm ci && npx wrangler login && npx wrangler deploy --env=""`.
2. Add a custom domain to the Worker (`relay.example.com`).
3. `npx wrangler secret put STORM_RELAY_ALLOWLIST --env=""` with one
   `<server_id> <public_key>` line per server, both from `GET /v1/server`.
   Allowlist only, never TOFU, on a public address.
4. Set `STORM_RELAY_PUBLIC_BASE=wss://relay.example.com`.
5. Settings › Connection › Relays › Add relay, then check `GET /v1/server`
   lists it.
6. Add the host to `PUBLIC_RELAY_HOSTS` in both the app and mcp-connect, so
   clients race it after direct and self-hosted relays.

Here Cloudflare's edge is the relay and attests each client's address
(`CF-Connecting-IP`), so the "no proxy in front" rule above is met as long as
you put nothing else in front of the Worker. Past the Free plan's daily limits
the relay errors until the day resets; the server and the LAN are unaffected.

## MCP clients: `storm-runtime mcp-connect`

Moved to the docs: [Connect an AI app](../docs/site/mcp/connect.md).

## Runtime Hosts (Agent Runtime)

Moved to the docs: [Runtime Hosts](../docs/site/agents/hosts.md) and
[Agent sessions](../docs/site/agents/sessions.md).

### Linux

[Runtime Hosts › Linux](../docs/site/agents/hosts.md#linux).

### macOS

[Runtime Hosts › macOS](../docs/site/agents/hosts.md#macos).

### Windows

[Runtime Hosts › Windows](../docs/site/agents/hosts.md#windows). The server on Windows:
[Install the server › Windows](../docs/site/install/server.md#windows).

### Claude Code: Claude login sync (optional)

Instead of logging Claude Code in on each host, you can keep **your own**
Claude subscription login in Storm and give it to the hosts you tick. It is
for one person, on their own machines, and the Anthropic terms risk is
yours: read [the README section](../README.md#claude-login-sync-optional)
and [Claude login sync](../docs/site/agents/claude-login.md) before turning
it on. To revoke the token, remove it in your Claude account settings on
claude.ai. A host's own login (its `claude.env` or `/login`) is untouched and
comes back when you untick it.

### Never share an account with the vaults

[Runtime Hosts › Never share an account with the vaults](../docs/site/agents/hosts.md#never-share-an-account-with-the-vaults).

## Security

Moved to the docs: [Security model](../docs/site/operate/security.md).
