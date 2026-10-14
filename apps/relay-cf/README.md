# storm-relay-cf

The Storm Relay Protocol v1 relay on **Cloudflare Workers + Durable Objects**:
the hosted half of the relay (`PLAN.md` decision 108, slice B = 108b). It is
wire-identical to `storm-relay` (`apps/relay`): the same paths, handshake,
frames and error codes. `docs/srp-v1.md` is the contract both implement, and
`tools/relay-conformance` (108a) is the black-box suite both pass.

Like `storm-relay` it is **never an authority** (R5): it authenticates
servers (Ed25519, `storm-relay-auth:v1:`), never clients (R12), holds no user
or vault data, and stores no payload.

## Shape

```
server ─wss─▶ Worker /register ──(reads REGISTER_SERVER, then carries frames)──┐
client ─wss─▶ Worker /connect/<server_id> ─(HELLO limit, then hands the socket)┼─▶ RelayRoom
                                                                               │   idFromName(server_id)
                                                                               │   open: idFromName(server_id + ":" + pubkey)
HELLO limit per source address ─▶ HelloLimiter idFromName("ip:" + CF-Connecting-IP)
```

- **`RelayRoom`**, one SQLite-backed Durable Object per `server_id` (in open
  mode, per `(server_id, pubkey)`), holds the server trunk (and a superseded
  one while it drains) and every client trunk routed to it, all through the
  WebSocket Hibernation API. Routing a frame never leaves the object.
- **Open mode routes on the key.** `/register` reads the key from
  `REGISTER_SERVER`, `/connect` from `?pubkey=`, so two keys of one
  `server_id` never share a room. A connect URL without `?pubkey=` lands in
  the bare `server_id` room, which holds no server and refuses the `HELLO`
  with `server_unreachable` at once.
- **`/register` names no `server_id`**, so the Worker reads the first frame,
  picks the room, and then carries that trunk's frames both ways. Client
  trunks are handed to the room whole.
- `public_address` is `<STORM_RELAY_PUBLIC_BASE>/connect/<server_id>`.

What survives hibernation, how the timers are alarms, and what "a client that
fell behind" means without a send buffer: `PLAN.md` decision 108b.

## Local development

Node 22. Everything runs locally; there is no Cloudflare account here.

```sh
npm ci
cp .dev.vars.example .dev.vars      # then put your server's key in the allowlist
npx wrangler dev                     # 127.0.0.1:18910 (wrangler.jsonc)
npm test                             # vitest inside workerd
npm run typecheck
```

Point a storm-server at it like any relay:

```sh
curl -X PUT -H "Authorization: $STORM_SESSION" -H 'Content-Type: application/json' \
  -d '{"relays":["ws://127.0.0.1:18910"]}' http://127.0.0.1:8484/v1/config/relays
```

From the repository root, `make relay-conformance-cf` runs the 108a suite
against two `wrangler dev` instances (allowlist on 18910, TOFU on 18911), and
`make test-live RELAY_IMPL=cf` runs every live suite with the server
registered to the Worker instead of storm-relay (decision 108c).
`scripts/dev-relay.sh` is what both use to start one.

`scripts/smoke.mjs` then makes real requests through it: the identity
challenge, a 1 MiB note written and read, a 20 MiB attachment both ways,
concurrency, and the change feed.

```sh
RELAY=ws://127.0.0.1:18910 ORIGIN=http://127.0.0.1:8484 STORM_SESSION="Bearer sta_…" \
  node scripts/smoke.mjs
```

Under `wrangler dev`, `CF-Connecting-IP` is the socket's address, but a value a
client sends in that header is passed through. Cloudflare's edge overwrites it;
only a deployed relay can rely on it.

## Configuration

Worker vars (or secrets). The names are `storm-relay`'s and the conformance
suite's fixture contract (`tools/relay-conformance/fixtures/relay.env`).

| Var | Default | What |
|---|---|---|
| `STORM_RELAY_BINDING` | `allowlist` | `allowlist` (production), `tofu` or `open`. TOFU exists for parity and the conformance suite only: a public `server_id` is squattable (§4.1). `open` is key-addressed and refuses every keyless `HELLO`; the hosted relay moves to it only after slice 116k. |
| `STORM_RELAY_ALLOWLIST` | empty | The allowlist **text**, in storm-relay's file format: `<server_id> <pubkey>` per line, `#` comments, no duplicates. A secret in production. Empty in allowlist mode refuses every registration. |
| `STORM_RELAY_PUBLIC_BASE` | from the request | `ws://` or `wss://` plus host, no path. |
| `STORM_RELAY_CHALLENGE_TTL_MS` | 30000 | A nonce is answerable this long; spent nonces stay spent for it. |
| `STORM_RELAY_HANDSHAKE_TIMEOUT_MS` | 30000 | An unfinished registration or a client that never says HELLO is closed. |
| `STORM_RELAY_HELLO_WAIT_MS` | 5000 | A HELLO with no live trunk is held this long, then `server_unreachable`. |
| `STORM_RELAY_STREAM_ACK_TIMEOUT_MS` | 5000 | `server_timeout` for an unacknowledged `STREAM_OPEN`. |
| `STORM_RELAY_HEARTBEAT_DEADLINE_MS` | 45000 | A server trunk silent this long is dead. |
| `STORM_RELAY_SUPERSESSION_DRAIN_MS` | 30000 | A superseded trunk's streams get this long, then `trunk_superseded`. |
| `STORM_RELAY_HELLO_RATE_LIMIT` / `_WINDOW_MS` | 10 / 60000 | Client trunks per source address per window. |
| `STORM_RELAY_MAX_IN_FLIGHT_STREAMS` | 20 | Unacknowledged opens per client trunk. |
| `STORM_RELAY_MAX_TOTAL_STREAMS` | 1000 | Open streams per server trunk. |
| `STORM_RELAY_MAX_CLIENT_BUFFER_BYTES` | 32 MiB | Unread bytes a client may hold before its stream is ended `client_backlogged`. |
| `STORM_RELAY_MAX_SERVER_BUFFER_BYTES` | 32 MiB | The same toward the origin: an upload past it is ended `rate_limited`. Worker only. |
| `STORM_RELAY_MAX_ROOM_BUFFER_BYTES` | 64 MiB | Unread bytes across one room. Worker only: a Durable Object has 128 MB. |

storm-relay reads `STORM_RELAY_ALLOWLIST` as a path; here it is the content,
since a Worker has no file system. A var that does not parse refuses every
connection with `protocol_error` and logs `config_invalid`.

## What it logs

One JSON line per event, through `console.log` (`src/log.ts` is the whole
surface): connection lifecycle and error codes only.

| Event | Fields |
|---|---|
| `server_registered`, `server_closed`, `server_superseded`, `server_drain_ended`, `server_heartbeat_expired` | `server_id`, `trunk_id`, a fixed `reason`, `code` |
| `registration_refused` | `server_id`, `reason` (`binding`, `nonce`, `signature`): ours to read, never sent to the peer |
| `tofu_recorded` | `server_id` |
| `client_opened`, `client_closed` | `server_id`, `client_trunk_id`, `code` |
| `client_backlogged`, `server_backlogged` | `client_trunk_id` or `trunk_id`, `stream_id` |
| `hello_rate_limited`, `protocol_error`, `relay_fault`, `config_invalid` | a fixed `reason` |

Never logged: a payload, a header, a request path or method, a nonce, a
signature, a public key, or a client address. Log retention is Q15,
deferred to step 2.

## Deploying

`DEPLOY.md`: the operator's own Cloudflare account, `wrangler deploy`, a
custom domain, the allowlist as a secret, verification, logging and the Free
plan's limits. `wrangler.jsonc` carries no `account_id` and no routes on
purpose. It is not part of `release.yml` or `apt-repo.yml`.
