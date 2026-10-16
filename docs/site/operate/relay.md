---
title: Relays
description: Reach your server from outside your network without opening a port.
draft: true
---

A **relay** lets your devices reach the server from outside your network: the
server connects out to it, so nothing is port-forwarded, and the apps and
`mcp-connect` use whichever of the server's own address and its relays
answers first. A relay holds no notes and checks no credentials — a client's
credential travels inside the tunnel to your server, which checks it exactly
as on your network. **Until end-to-end encryption, a relay can read what it
carries, notes and sign-ins included: run your own, or use one you trust.**

## Running your own

`storm-relay` lets clients reach a server from outside its network
. It is a **separate package** in the same apt repository,
and nothing needs it: `storm-server` does not depend on it and works the same
without one. It holds no vault data and authenticates no clients;
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
  `--allow-public-tofu`.
- `STORM_RELAY_ALLOWLIST`: only listed keys register. **Use this on a public
  address.** Same format as the bindings file, so a bindings file you trust can
  be copied over as the allowlist.

**Revoking a server** on an allowlist relay: remove its line and run `sudo
systemctl reload storm-relay`. The relay re-reads the file and ends that
server's live trunk at once, not only its next registration.

**Limits** (all on by default; `storm-relay --help` names the flags): `/register`
attempts and `HELLO`s per source address, an IPv6 /64 counting as one;
streams per server and opens in flight per client; 32 MiB unread per stream.
A client past one gets `rate_limited` or `client_backlogged`; the server's
trunk and every other client carry on.

**Logs** go to the relay's own journal namespace, kept at most 7 days
(`/etc/systemd/journald@storm-relay.conf`):
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

**On a public address, the relay terminates TLS itself**. Do not
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
alone. `GET /v1/server` lists the relays it has actually
registered with, which is the check that it worked; the `PUT` itself returns
before any connection is attempted. Clients learn the relay from the server when they pair.

## On Cloudflare instead

`apps/relay-cf` is the same relay as a Cloudflare Worker with Durable
Objects, for an operator who would rather not run a public box. It deploys to **your own** Cloudflare account, on the Free plan, with
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
