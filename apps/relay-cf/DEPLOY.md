# Deploying the Cloudflare relay

The operator's runbook for `apps/relay-cf` (decision 108c). It puts an
SRP v1 relay on your own Cloudflare account, in **allowlist mode**, for your
own servers. Nothing here is automated: deployment is not part of
`release.yml` or `apt-repo.yml`, and no Storm build deploys anything to
Cloudflare.

What it is and is not:

- It is the same relay as `storm-relay` (`docs/srp-v1.md`, held to it by
  `tools/relay-conformance`). It holds no notes and authenticates no clients;
  a client's credential rides inside the tunnel to your server.
- It is a **public** relay: a client races your server's direct address and
  any self-hosted relay first, and this one last (`PUBLIC_RELAY_HOSTS`, step 7).
- It is invite-only by allowlist. There are no relay accounts yet (that is
  step 2 of the hosted relay, with abuse limits and log retention, Q14/Q15).
  **Never deploy it in `tofu` mode**: a `server_id` is public, and the first
  key to register it would own it for good (§4.1).

## 1. The account and the plan

- A Cloudflare account of your own. The **Workers Free** plan is enough for
  one household's servers.
- The relay's Durable Objects are **SQLite-backed** (`new_sqlite_classes` in
  `wrangler.jsonc`), which is the only kind the Free plan offers. Nothing to
  enable.
- Node 22 and this repository. From `apps/relay-cf`: `npm ci`.

## 2. Log in and deploy

```sh
cd apps/relay-cf
npx wrangler login            # opens a browser; authorises wrangler for your account
npx wrangler whoami           # check it is the account you meant
npx wrangler deploy --env=""  # the production Worker, "storm-relay"
```

`--env=""` picks the top-level (production) configuration; `wrangler.jsonc`
also defines the `conformance` Workers of step 8, and wrangler warns when a
deploy names none. The first deploy creates the Worker, both Durable Object
classes and their migration. It prints a `*.workers.dev` URL; you will replace
it with your own hostname next.

Until the allowlist is set (step 4) the relay refuses every registration,
which is the safe state to be in.

## 3. A custom domain

The domain must be a zone on the same Cloudflare account.

Dashboard: Workers & Pages › `storm-relay` › Settings › Domains & Routes ›
Add › Custom domain › `relay.example.com`. Cloudflare creates the DNS record
and the certificate. Or in `wrangler.jsonc`, then deploy again:

```jsonc
"routes": [{ "pattern": "relay.example.com", "custom_domain": true }]
```

Optionally turn off the `workers.dev` route (Settings › Domains & Routes), so
the relay has one address.

Decision 71 (no proxy in front of a relay) does not apply as written here:
Cloudflare's edge *is* the relay, and it sets `CF-Connecting-IP` on every
request, overwriting anything a client sent. The relay takes each client's
address from it (`docs/srp-v1.md` §5.2 *(108c)*). Do not put another proxy or
a Cloudflare Tunnel in front of the Worker.

## 4. The allowlist (a secret)

One line per server: `<server_id> <pubkey>`, storm-relay's allowlist format.
Both come from the server itself, unauthenticated:

```sh
curl -s https://storm.home.example/v1/server
# {"server_id":"srv_…","public_key":"…", …}
```

(`storm-server status` shows the service and its health, not the identity;
`GET /v1/server` is the place.) `public_key` is base64url without padding;
copy it exactly. Then:

```sh
printf '%s %s\n' srv_… <public_key> > allowlist      # one line per server
npx wrangler secret put STORM_RELAY_ALLOWLIST --env="" < allowlist
rm allowlist
```

A secret is applied at once, without a deploy. To add a server later, put the
whole list again: the secret is replaced, not appended to. A line that does
not parse refuses **every** connection with `protocol_error` and logs
`config_invalid`, so check the result (step 7) after each change.

The binding is the allowlist line and nothing else. A server whose key
changes (a restored backup without `state/identity/`) needs its line changed
too.

## 5. `STORM_RELAY_PUBLIC_BASE`

What `REGISTERED`'s `public_address` starts with, which is what your server
advertises to clients. Set it to the custom domain:

```sh
npx wrangler secret put STORM_RELAY_PUBLIC_BASE --env=""   # wss://relay.example.com
```

(or as a `vars` entry in `wrangler.jsonc`, then deploy). Without it the relay
derives it from each request's host, which is right for one hostname and
wrong as soon as there are two (the `workers.dev` route and the domain).

The other `STORM_RELAY_*` knobs (`README.md`) keep their production defaults;
leave them unset.

## 6. Point your server at it

In the app: **Settings › Connection › Relays › ＋ Add relay**, and enter
`wss://relay.example.com`. The server registers within a second or two, with
no restart. Equivalently, as the owner, `PUT /v1/config/relays`.

`GET /v1/server`'s `relays` lists only relays the server is actually
registered with, so that is the check: the new URL appears there. If it does
not, `npx wrangler tail --env=""` shows `registration_refused` (with a
private reason: `binding`, `nonce` or `signature`) and the server's log says
`auth_failed`. `binding` means the allowlist line does not match.

Paired clients learn the relay from `GET /v1/server` the next time they reach
the server by any path; new pairings carry it in the QR.

## 7. Make clients treat it as public

A hosted relay must lose the race to direct and to self-hosted relays (SRP
§2.2 in the vault's *Storm Relay Protocol*). Clients rank a relay as *public* only if its host is on
their list, so add the host, without scheme or port, to **both**:

- the app: `kPublicRelayHosts` in `apps/client/lib/api/storm_connection.dart`;
- mcp-connect: `PUBLIC_RELAY_HOSTS` in `apps/runtime/src/race.rs`.

Both lists ship in a release; a relay not on them is raced as self-hosted,
which is the safe direction to be wrong in but not the intended order. Storm's
own hosted relay, `storm-relay.dewansh.space`, is on both (decision 116g).
A public relay is raced only once every direct and self-hosted candidate has
failed or hit its 15 s deadline; there is no round timer.

## 8. Verify

**Your own server, end to end:** with the server registered (step 6),
`scripts/smoke.mjs` makes real requests through the relay — the identity
challenge, a 1 MiB note, a 20 MiB attachment both ways, the change feed:

```sh
RELAY=wss://relay.example.com ORIGIN=https://storm.home.example \
STORM_SESSION="Bearer sta_…" node scripts/smoke.mjs
```

**The conformance suite** needs the fixture contract (short timers, 48 test
keys whose seeds are public), so it **never runs against the production
Worker**. Deploy the separate `conformance` Worker, load the fixture
allowlist, run, and delete it:

```sh
npx wrangler deploy --env conformance
npx wrangler secret put STORM_RELAY_ALLOWLIST --env conformance \
  < ../../tools/relay-conformance/fixtures/allowlist
cd ../.. && make relay-conformance \
  RELAY_URL=wss://storm-relay-conformance.<subdomain>.workers.dev
```

That runs every check except the TOFU ones, which are skipped without
`RELAY_TOFU_URL`. For those too, deploy `--env conformance-tofu` (a TOFU
Worker, acceptable only because it is a throwaway with test knobs) and add
`RELAY_TOFU_URL=wss://storm-relay-conformance-tofu.<subdomain>.workers.dev`.
Leave `RELAY_MULTI_IP` and `RELAY_LATENCY` off: the first needs loopback, the
second a localhost median. Set `RELAY_PEER_IP` to your public address to check
that `relay_peer_ip` is the edge's attested address. Then:

```sh
cd apps/relay-cf
npx wrangler delete --env conformance
npx wrangler delete --env conformance-tofu
```

Two things only a real deployment shows, and `wrangler dev` cannot: that
Cloudflare overwrites a client-sent `CF-Connecting-IP` (under `wrangler dev` a
client can set it), and that the edge carries a 16 MiB WebSocket message (the
suite's §5.2 frame checks). A full suite run against the `conformance` Worker
covers the second; for the first, send a `CF-Connecting-IP` header on the
upgrade from a client and check the origin's `relay_peer_ip` is still your
real address.

## What it logs, and for how long

`src/log.ts` is the whole surface: one JSON line per lifecycle event and
error code (`server_registered`, `client_opened`, `registration_refused` with
its private reason, `hello_rate_limited`, …). **Never** a payload, a header, a
path, a nonce, a signature, a key, or a client address. The table is in
`README.md`.

Where it goes: `console.log` reaches `wrangler tail` and the dashboard's live
log stream, which keep nothing. `wrangler.jsonc` does not enable Workers Logs
(`observability`), so by default Cloudflare does not store these lines. If you
turn it on, Workers Logs keeps them for a few days (3 on the Free plan, 7 on
Paid, at the time of writing; check Cloudflare's current documentation).
Cloudflare's own edge analytics (request counts, status codes, and the
metadata Cloudflare records for any request it serves) are Cloudflare's, under
its terms, and outside what this code controls.

How long a Storm-run relay should keep anything is **Q15, still open**; it
matters for step 2 (a relay serving other people), not for your own servers.

## Free-tier limits, and what happens past them

At the time of writing (check Cloudflare's pricing page; these change):

| Limit (Workers Free) | Roughly | What uses it |
|---|---|---|
| Worker requests | 100,000 / day | every `/register` and `/connect` upgrade |
| Durable Object requests | 100,000 / day | upgrades, alarms, and incoming WebSocket messages (billed at 20 messages per request) |
| Durable Object duration | 13,000 GB-s / day | a room awake while frames flow; hibernation stops the clock when idle |
| SQLite rows written / read | 100,000 / 5 million per day | the stream table, nonces |
| Durable Object storage | 5 GB | a few rows per live stream; never payloads |
| CPU per request | 10 ms (Worker) | the `/register` hop, which carries a server trunk's frames |

**Unverified until a real deployment:** the `/register` hop is the one place
a stateless Worker carries frames for a trunk's whole life (decision 108b),
and whether a busy server trunk stays inside the Free plan's CPU limit there
is not something `wrangler dev` measures. If it does not, the symptom is a
server trunk dropped mid-transfer (`trunk_lost` to its clients) and the server
reconnecting; the Paid plan's limit, or an optional `server_id` on the upgrade
(so the hop is spliced like a client's), is the fix.

A heartbeat costs little: `PING` is answered by the runtime's auto-response
and never wakes the room. What counts is traffic: a large attachment is many
256 KiB frames, each a message.

Past a daily limit Cloudflare refuses requests until the day resets (00:00
UTC). Then the relay errors: servers cannot register or are dropped, and
clients that can only reach the server through this relay see it as down
(`relay_down`, or *Couldn't reach the server*). **Local Storm is unaffected**:
the server, the LAN and any self-hosted relay keep working, because a hosted
relay is only ever the last candidate and the server never depends on one
(R6). The Workers Paid plan raises every limit; nothing in the code changes.

## Updating and removing

Update: pull, `npm ci`, `npx wrangler deploy --env=""`. Live trunks are
dropped by the redeploy; servers reconnect on their own backoff (1 s, doubling
to a minute) and clients on their next request.

Remove: delete the relay in Settings › Connection › Relays first (the server
sends `DEREGISTER`), then `npx wrangler delete --env=""`, and take the host
out of `PUBLIC_RELAY_HOSTS` in the next release.
