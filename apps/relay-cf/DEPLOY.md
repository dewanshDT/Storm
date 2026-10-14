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
- It runs in **allowlist** mode for your own servers. Storm's hosted relay
  switches to **open** mode (anyone may register, addressed by
  `(server_id, pubkey)`, decision 114f) only once the abuse controls (116k),
  key-addressed registration (116n) and the origin's no-hot-loop rule (116f)
  are deployed and a week of usage has been measured. **Never deploy it in
  `tofu` mode**: a `server_id` is public, and the first key to register it
  would own it for good (§4.1).
- **Cloudflare can read it.** Until end-to-end encryption (Q19), whoever runs
  a relay can read the traffic it carries, notes and sign-ins included; on
  this one that is you and Cloudflare. Public in reach, private in membership:
  anyone can connect, only listed servers can register.

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
classes and their migration. `wrangler.jsonc` sets `"workers_dev": false` and
`"preview_urls": false`, so the Worker has no address until step 3 gives it
yours.

Until the allowlist is set (step 4) the relay refuses every registration,
which is the safe state to be in.

## 3. A custom domain

The domain must be a zone on the same Cloudflare account.

Dashboard: Workers & Pages › `storm-relay` › Settings › Domains & Routes ›
Add › Custom domain › `relay.example.com`. Cloudflare creates the DNS record
and the certificate.

**Add the domain in the dashboard, not as `routes` in `wrangler.jsonc`.** An
`env` inherits top-level `routes`, so a `conformance` deploy would claim the
production hostname, and deleting that Worker afterwards deletes the domain
with it (Cloudflare error 1016 on every connect, until the production Worker
is deployed again). Both conformance envs set `"routes": []` and
`"workers_dev": true` so that cannot happen even if someone adds `routes` at
the top; before a conformance deploy, `npx wrangler deployments list --env
conformance` must not mention your hostname.

The production Worker has no `workers.dev` route, so the relay has one
address and scanners find one door. Keep `"workers_dev": false` in the file:
a config that leaves it out turns the route back on with every deploy,
whatever the dashboard says.

Decision 71 (no proxy in front of a relay) does not apply as written here:
Cloudflare's edge *is* the relay, and it sets `CF-Connecting-IP` on every
request. A client cannot supply its own: the edge **refuses** a request that
carries one, with 403 and `error code: 1000` (checked on the live relay,
2026-10-15). The relay takes each client's address from it (SRP §5.2). Do not
put another proxy or a Cloudflare Tunnel in front of the Worker.

## 4. The allowlist (a secret)

One line per server: `<server_id> <pubkey>`, storm-relay's allowlist format.
Both come from the server itself, unauthenticated:

```sh
curl -s https://storm.home.example/v1/server
# {"server_id":"srv_…","public_key":"…", …}
```

(`storm-server status` on the server prints the same `server` and `key`.)
`public_key` is base64url without padding; copy it exactly. Then:

```sh
printf '%s %s\n' srv_… <public_key> >> allowlist     # one line per server
npx wrangler secret put STORM_RELAY_ALLOWLIST --env="" < allowlist
```

**Keep `allowlist`.** A secret cannot be read back, so this file is the only
copy you can see; the next change edits it and puts it again. The secret is
replaced, not appended to.

**Putting a secret creates a new version of the Worker.** It takes about a
minute to reach every edge location (allow two), and as it lands **every live
trunk is dropped**: servers reconnect on their own backoff and clients on
their next request. A line that does not parse refuses **every** connection
with `protocol_error` and logs `config_invalid`, so check `GET /health` and
step 6 after each change.

### Changing or revoking an entry

The binding is the allowlist line and nothing else. To change a server's key
(a restored backup without `state/identity/`, a deliberate rotation): **add**
the new line first, put the secret, let the server re-register under the new
key, then remove the old line and put it again. To revoke a server, remove its
line and put the secret: the redeploy ends its live trunk, and a room that
wakes re-checks its server trunks against the current list (decision 116k), so
the server cannot come back.

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
registered with, so that is the check: the new URL appears there, and in the
app the relay loses its *Not connected* label (`storm-server status` lists the
same set). If it does
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

**Timings on the real edge.** The fixture's 2 s challenge TTL and 1 MiB
client buffer are tuned for `wrangler dev`; across the internet they fail for
want of time, not for a bug. Deploy the conformance Worker with the edge
values and tell the suite the same:

```sh
npx wrangler deploy --env conformance \
  --var STORM_RELAY_CHALLENGE_TTL_MS:10000 \
  --var STORM_RELAY_MAX_CLIENT_BUFFER_BYTES:4194304
RELAY_ENV_OVERRIDES=STORM_RELAY_CHALLENGE_TTL_MS=10000,STORM_RELAY_MAX_CLIENT_BUFFER_BYTES=4194304 \
  make relay-conformance RELAY_URL=wss://storm-relay-conformance.<subdomain>.workers.dev
```

Two things only a real deployment shows, and `wrangler dev` cannot: that the
edge carries a 16 MiB WebSocket message (the suite's §5.2 frame checks, which
the run above covers), and that it refuses a client-sent `CF-Connecting-IP`
(under `wrangler dev` a client can set it):

```sh
curl -s -o /dev/null -w '%{http_code}\n' \
  -H 'CF-Connecting-IP: 203.0.113.9' https://relay.example.com/health   # 403
```

## Scanners

The relay answers three paths: `/register`, `/connect/<server_id>` and
`/health`. Everything else is a 404 from the Worker, but each one still costs
a request against the daily limit. A WAF custom rule (Security › WAF) that
blocks any request whose path is not one of those three keeps scanners off
the Worker entirely:

```
not (http.request.uri.path eq "/register" or
     starts_with(http.request.uri.path, "/connect/") or
     http.request.uri.path eq "/health")
```

## Watching it

- **Uptime:** point a check at `https://relay.example.com/health`. It answers
  `{"ok":true,"mode":"allowlist"}`, or 503 when the configuration does not
  load (a bad allowlist line). A plain `GET /register` answers 426 if you
  would rather check the upgrade path.
- **Your servers:** `GET /v1/server`'s `relays` (or `storm-server status`) lists
  the relay while the server holds a trunk with it.
- **Usage:** Account › Notifications › Workers usage alerts at 80 % of the
  daily request and Durable Object limits, so a busy day warns before it
  refuses. Each room also counts the bytes it moved per day (its `usage`
  table, logged with `server_closed`), which is what a week of measurement
  reads before any per-registration budget is set.

## The account

- Two-factor authentication on the Cloudflare account.
- Deploy with a **scoped API token** (Workers Scripts: Edit, Account Settings:
  Read, for this account only), not the global key; keep it out of the repo
  and out of chat.
- `npx wrangler logout` on a machine you only deployed from once.

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

**The rule (decision 114b):** lifecycle events and error codes only, no client
address, and nothing stored for more than 7 days. Leave Workers Logs off. A
`register_rate_limited` or `hello_rate_limited` line says a limit was hit,
never by whom.

**Privacy:** Cloudflare's own analytics record every request's client IP
address, outside this code and under Cloudflare's terms. A relay that carries
other people's servers must tell them so.

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

The relay's own limits sit inside these: `/register` and `HELLO` attempts per
source address (IPv6 per /64; `STORM_RELAY_REGISTER_RATE_LIMIT`,
`STORM_RELAY_HELLO_RATE_LIMIT`), streams per registration and per client, and
32 MiB unread per stream. A client that misses one gets `rate_limited` or
`client_backlogged`, never a dropped server.

**Free-plan spenders,** in order: attachment traffic (every 256 KiB frame is a
message), the change feed's keep-alives on long-lived clients, and
reconnect storms after a deploy. A server's heartbeat costs almost nothing.

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
to a minute, ±20 % jitter, and never reset by a trunk that lived under a
minute, so a crashing relay is not hammered) and clients on their next
request.

Remove: delete the relay in Settings › Connection › Relays first (the server
sends `DEREGISTER`), then `npx wrangler delete --env=""`, and take the host
out of `PUBLIC_RELAY_HOSTS` in the next release.
