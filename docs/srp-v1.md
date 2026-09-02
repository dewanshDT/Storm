# Storm Relay Protocol — wire spec (v1)

> **Status: spec only, nothing built.** The relay is designed in `PLAN.md`
> (decision 52 and the relay decisions R1–R13) and the vault note *Storm Relay
> Protocol*, which holds the rationale this file deliberately leaves out. This
> document is the citable artifact for §2–§4 of the implementation checklist:
> what goes on the wire, not why. Where the two disagree, the vault note and
> `PLAN.md` are current and this file is amended in the same change that moves
> them.
>
> Normative language: MUST / SHOULD / MAY as in RFC 2119.

> **§3–§6 were rewritten on 2026-08-28 (`PLAN.md` decision 58).** The first
> draft described a *different protocol* — one in which the relay authenticates
> **clients**, registering "device keypairs". That is the precise thing **R12**
> forbids, and it would have been implemented had a relay author not read the
> design note alongside it. Every wire detail below is regenerated from the
> accepted design.

## 1. Principle

**The relay is a reverse proxy of the existing Storm wire protocol, not a
second protocol.** Storm's REST surface (`/v1/*`), the `/v1/stream` change feed
and Streamable HTTP for MCP (`/mcp`) **do not change**. The relay moves the
same HTTP requests and responses a LAN client would exchange, over one
persistent tunnel, routed by `server_id` instead of by IP.

The relay:

- **MUST** authenticate *servers*, so that whatever answers for a `server_id`
  provably holds that server's private key (§4).
- **MUST NOT** authenticate *clients* (**R12**). A client's credential — device
  credential, session token, or `stk_` MCP key — rides **inside** the tunnelled
  HTTP request and is checked by the origin server exactly as on the LAN. The
  relay never parses it. Any client speaking valid SRP gets a trunk.
- **MUST NOT** interpret request bodies. It cannot distinguish an MCP
  `create_note` from a login attempt from a note read, and this is deliberate:
  it is what makes login rate limiting a *server-side* problem.
- **MUST NOT** carry generic TCP, arbitrary host/port forwarding, or act as a
  SOCKS/VPN proxy. Its only destination concept is `server_id`.

> **Reachability is not authentication.** The relay substitutes for a public IP
> and an open port. Anyone may open a connection to any public HTTPS server;
> the server authenticates the caller afterwards. Treating the ability to
> *request* a tunnel as an access control is a category error — and it is the
> false premise that made the first draft of this document wrong.

A fully compromised relay can drop or delay traffic and read plaintext bodies
(there is no E2E encryption in v1), but **cannot impersonate a server to a
client** — that is caught end to end by the client's own identity challenge,
which is outside this protocol.

**The protocol is the deliverable, not any one deployment of it.** Nothing in
this spec names a tunnel vendor, a hosting provider, or a network topology.
A protocol with one implementation becomes that implementation; **R7** forbids
baking provider-specific assumptions into the wire format.

## 2. Parties and topology

| Party | Role |
|---|---|
| **Origin server** | The user's Storm sync server. Registers itself with each relay in its configuration and receives a `public_address` it can hand out. **The only party the relay authenticates.** |
| **Client** | A Storm app or MCP client, already paired with an origin server. Presents no credential to the relay. |
| **Relay** | An untrusted forwarder. Terminates TLS, routes frames by `server_id`, attributes peers. Sees plaintext today; trusted with availability, never confidentiality. |

```
                    ┌─ local_address (direct dial, LAN/static IP) ─┐
Storm Client ───────┤                                              ├──► Storm Server
(StormConnection)   └─ relay (self-hosted, then Storm-public) ─────┘
```

A server MAY hold **simultaneous trunks to several relays** — a self-hosted one
and a public one, say — for redundancy. Registration and supersession are
scoped **per relay**: superseding a trunk on relay A MUST NOT disturb a
concurrent trunk on relay B.

### Connection resolution order (client side)

```
1. dial local_address directly                 ─┐
2. dial the self-hosted relay's public_address  ├─ raced together
                                                ┘
   ↓ only if 1 + 2 both fail / time out (~2s)
3. dial the public relay's public_address
```

**Self-hosted races with direct, and the public relay runs only on failure.**
This is a privacy rule, not a latency optimisation: without E2E encryption,
whichever path carries traffic reads note content — self-hosted means the
operator sees it, public means Storm does. Racing all three and taking the
first answer would silently prefer Storm's infrastructure over the operator's
own.

**Racing decides transport only, never trust.** Whichever candidate wins MUST
still pass the client's end-to-end identity challenge before the connection is
treated as established.

There is **no mid-session transport migration**: a dropped connection re-races
from scratch rather than hot-swapping under a live session.

## 3. Transport and framing

**Transport is WSS.** One persistent connection per server (*server trunk*),
one per client (*client trunk*). Every REST call, the change feed and every MCP
call from one client ride the **same** connection, distinguished by
`stream_id` — not one relay connection per request.

Two frame kinds, and the distinction is the WebSocket frame type itself:

- **Text frames** carry JSON control messages — registration, stream lifecycle,
  errors, heartbeats.
- **Binary frames** carry payload bytes. No base64: WS already length-delimits
  each message.

A binary frame is:

```
┌──────────┬───────────────┬───────────────────┐
│ type (1) │ stream_id (4) │ payload (rest)    │
└──────────┴───────────────┴───────────────────┘

0x01 = HTTP_REQUEST_BODY_CHUNK
0x02 = HTTP_RESPONSE_BODY_CHUNK
```

`stream_id` is `u32`, big-endian.

**There are exactly two body types, and there MUST NOT be a third.** `0x02` also
carries the change feed's events, because the change feed is an ordinary
streamed HTTP response rather than a special case (§5.3). An earlier design
draft had a third type for it; that draft was wrong.

> **SRP carries no WebSocket frames.** On the LAN the change feed is a
> WebSocket; over the relay it is an ordinary streamed HTTP response, so every
> payload in the tunnel is an HTTP head plus body chunks. There is no
> WebSocket-inside-WebSocket anywhere in this protocol, deliberately.

**`stream_id` is relay-assigned, never client-asserted**, and scoped per server
trunk. This is a security property, not a convenience: the relay multiplexes
many clients onto one server trunk, so a client that could assert its own
`stream_id` could forge another client's and receive its responses. `u32`
rather than `u16` so id-reuse safety never has to be reasoned about on a
long-lived trunk.

### Version pin

Every JSON control message carries a version envelope:

```json
{ "v": 1, "type": "...", "...": "fields" }
```

`v` is **hard-pinned**. An implementation speaking `v:1` MUST reject anything
else outright — no in-band negotiation, no multi-version support. An
incompatible change is a new relay deployment, not a runtime negotiation. A
message with a missing or non-`1` `v` is `protocol_error`, and `v` MUST be
checked before the rest of the body is interpreted.

## 4. Server registration

```
Server → Relay:  REGISTER_SERVER { server_id, pubkey }
Relay → Server:  CHALLENGE { nonce }                 // single-use, 30s TTL
Server → Relay:  CHALLENGE_RESPONSE { sig }
Relay → Server:  REGISTERED { trunk_id, public_address,
                              heartbeat_interval_secs: 15 }
```

The signature covers exactly:

```
sig = Ed25519_sign(priv_key, "storm-relay-auth:v1:" + server_id + ":" + nonce)
```

**`server_id` and the colon separators are part of the signed bytes.** A
signature over the domain prefix concatenated with the nonce alone is a
different message and MUST NOT verify.

**The domain prefix is deliberately not `storm-challenge:v1:`**, which is the
client-facing challenge. A signature proving relay registration must never be
mistakable for one proving identity to a client; distinct domains make that
structurally impossible rather than merely unlikely. Any future signed context
in this protocol MUST mint its own domain rather than reusing one.

**Both interpolated fields MUST be validated, not just the nonce.**
`server_id` and `nonce` are two colon-delimited fields in one signed string, so
constraining only one leaves the split ambiguous: `("a:b", "c")` and
`("a", "b:c")` produce **byte-identical** messages, and a signature over one is
a signature over the other.

| Field | Rule |
|---|---|
| `nonce` | 16–128 printable ASCII, containing neither `:` nor `"` |
| `server_id` | 1–128 characters, ASCII alphanumeric plus `_` and `-` |

`server_id`'s charset is what Storm's id generator already produces (`srv_`
plus Crockford base32); it is stated here as a **contract** rather than left as
a coincidence of the current implementation.

**Where each rule binds.** A relay generates the nonce and receives the
`server_id`; an origin server receives the nonce and supplies its own
`server_id`. So each party validates **what it receives as well as what it
emits** — the relay must not accept a `server_id` carrying `:`, and a server
must not sign a nonce carrying one. Validating only at generation assumes the
only values you ever see are your own, and on the relay the `server_id` is
chosen by whoever is registering.

The server reuses its **existing** Ed25519 identity. Relay registration is one
more thing that key proves possession of; there is no new key material, and no
client keypair is involved anywhere in this exchange.

### 4.0 Where a server connects

A server opens its trunk at **`ws(s)://<relay-host>/register`**. This is a wire
commitment: the origin's tunnel client and the relay must agree on it exactly,
and it is distinct from the client-facing `/connect/<server_id>` of §4.3.

`pubkey` and `sig` are **base64url without padding**, matching every other key
and signature Storm puts on a wire. An implementation using hex or standard
base64 would be conformant to a spec that failed to say this, and completely
non-interoperable — with `auth_failed` as the only symptom.

**A decoder MUST reject padding and the standard `+/` alphabet, not merely
avoid emitting them.** One key has one spelling. Lenient decoding sounds
harmless — the bytes are the same either way — but §4.1 binds a `server_id` to
a pubkey *for ever*, so two implementations that disagree about whether a
spelling is valid disagree about whether a key has changed, and there is no way
back from a refused binding. The hazard is not hypothetical: a decoder that
accepts both alphabets is the default in at least one standard library, so this
is a rule an implementation has to opt into. `docs/srp-vectors.json` carries the
cases.

### 4.1 Binding `server_id` to a pubkey

**The challenge alone binds nothing.** It proves the caller holds the private
key for *whatever pubkey it just sent* — not that this is the right key for
that `server_id`. An attacker can generate a keypair, send
`{server_id: <victim's>, pubkey: <attacker's>}`, and sign perfectly. Clients
remain safe, because the client's own identity challenge catches it, so the
blast radius is denial of service and routing capture on that relay. It is
closed at the relay too:

| Deployment | Binding |
|---|---|
| **Self-hosted, allowlist configured** | The allowlist **is** the binding. `REGISTER_SERVER` MUST be refused unless `pubkey` matches the operator's entry for `server_id`. **No TOFU**, even on first sight. |
| **Self-hosted, no allowlist** | **Trust-on-first-use.** The first *successful* registration records `(server_id, pubkey)`; a later differing key is `auth_failed`. Safe here because the operator already controls who can reach the relay at all. |
| **Storm-hosted (public)** | **Account-owned, not wire-only.** Wire-TOFU is unsafe when `server_id` is designed to be public: anyone learning it before the owner's first registration — a photographed QR, a leaked config, a log line — could squat it permanently, and a refused rebind makes that unrecoverable. First registration MUST authenticate to a relay account which owns the claim. |

A relay account governs quota, billing and claim ownership **only**. It is
explicitly **not** a Storm identity (**R1**).

**A spent nonce stays spent until it expires.** "Single-use" alone permits an
implementation that forgets a nonce on use and re-arms it if the same value is
issued again — which silently undoes single-use. A relay MUST keep a spent
marker for the remainder of the TTL, and MUST NOT resurrect a spent nonce by
re-issuing it.

The binding MUST be checked at `REGISTER_SERVER` time, **before** a challenge
is issued — there is no reason to spend a nonce on a registration that cannot
succeed. TOFU MUST record the pair only after a *successful* signature.

> A permanently-refused rebind is what makes squatting hard, and it also means
> a legitimate operator cannot rotate their key. That is a known v1 limitation
> (§7), not an oversight.

### 4.2 Trunk lifecycle

- **Heartbeat.** `PING`/`PONG` every 15 s. Three missed (~45 s) and the relay
  MUST treat the trunk as dead, stop routing new `HELLO`/`OPEN_STREAM` to it,
  and send every client with an open stream `ERROR{trunk_lost}`.
- **Clean shutdown.** A server SHOULD send `DEREGISTER {}` before closing, so
  the relay frees the `server_id` immediately rather than making every client
  wait out a timeout for what was a deliberate restart.
- **Supersession.** A new `REGISTER_SERVER` for a `server_id` that already has
  a live trunk *on this relay* succeeds once the challenge completes — anyone
  who can complete it already holds the private key, so this is a reconnect,
  not a hijack. The old trunk MUST drain gracefully: in-flight streams get up
  to 30 s, then anything still open is force-closed with
  `ERROR{trunk_superseded}`. Scoped per relay.

### 4.3 The relay set, and how a client learns it

A server MUST advertise only the relays it is **currently registered with**,
never the ones merely present in its configuration. A relay a server failed to
register with is a dead path, and a client that dials it spends its whole
connection-race budget on a candidate that cannot answer.

Two carriers, and both are required:

| Where | Freshness |
|---|---|
| The pairing payload | Frozen at issuance |
| `GET /v1/server` | Live; refreshed whenever the set changes |

**The pairing payload alone is insufficient.** It is a snapshot taken when the
QR was generated, so a client whose server later changes relays — while that
client is away from the LAN — has no path back to a working address. The
`none`-tier `GET /v1/server` response therefore carries the current set, and a
client refreshes from it whenever the server is reachable **by any path at
all**, including a relay it already knows.

`public_address` is `<scheme>://<relay-host>/connect/<server_id>` — **derived,
not allocated**. Any client holding a `server_id` can construct it, and the
relay hands out no opaque identifier.

**The scheme is deployment configuration, not part of the derivation.** `wss`
for anything reachable off the machine; a relay running plaintext for local
development advertises `ws`, and hard-coding `wss` here would mean no client
could dial it.

## 5. Client trunk and stream lifecycle

```
Client → Relay:  HELLO { server_id }
Relay → Client:  READY { client_trunk_id }
                 // no live trunk for server_id: hold up to 5s (covers a
                 // server mid-restart), then ERROR{server_unreachable}
```

**There is no challenge here, on purpose** — the relay authenticates servers,
never clients (**R12**). Any client speaking valid SRP gets a trunk. What
client connections need is resource limits (§6), not identity.

### 5.1 Opening one stream

One REST call, one MCP call, or the change feed.

```
Client → Relay:  OPEN_STREAM { attempt_id }
Relay → Client:  STREAM_READY { attempt_id, stream_id }

Relay → Server:  STREAM_OPEN { stream_id }
Server → Relay:  STREAM_ACK  { stream_id }
                 // relay waits up to 5s. No ACK →
                 // ERROR{server_timeout, stream_id}, no retry.
```

**`STREAM_ACK` is the acknowledgement, and it is a distinct message.** Earlier
revisions required the server to "ACK `STREAM_OPEN`" and named nothing that
does — an origin server and a relay written independently would each have
invented one, which is the interop failure this document exists to prevent.

The ACK says only *this trunk has accepted this `stream_id`*. It does not mean
the request has been dispatched or that a response is coming; those are
`HTTP_RESPONSE_HEAD` and the time-to-first-byte rule in §5.2. A server MUST
refuse a `stream_id` it already has open, with `ERROR{stream_closed}`, rather
than acknowledging it twice.

`attempt_id` is client-generated and echoed back **only** so the client can
correlate concurrent opens before it learns the relay-assigned `stream_id`. It
has no meaning on the wire afterwards, and the relay never routes on it.

In-flight `STREAM_OPEN`s MUST be capped per trunk (e.g. 20); past that,
`OPEN_STREAM` gets `ERROR{rate_limited}` immediately rather than queueing — a
slow server and an overloaded one should fail differently, not compound.

### 5.2 HTTP request and response over a stream

```
Client → Relay → Server:
  HTTP_REQUEST_HEAD { stream_id, method, path, headers }
  [binary 0x01, stream_id] body chunk(s)

Server → Relay → Client:
  HTTP_RESPONSE_HEAD { stream_id, status, headers }
  [binary 0x02, stream_id] body chunk(s)
```

The body chunks are the proxied HTTP exchange verbatim. The relay MUST NOT
rewrite them.

**`relay_peer_ip`.** The relay attaches the client trunk's real source address
as a sibling field on `HTTP_REQUEST_HEAD` when it re-emits the message
server-ward. Two rules, **both required**, or this is `X-Forwarded-For` under a
new name:

1. The relay **MUST overwrite it unconditionally**, whatever the client sent.
2. The relay **MUST reject** (`protocol_error`) any `HTTP_REQUEST_HEAD`
   arriving *from a client* that already carries the field. It is only ever
   valid on the relay→server hop.

Both halves live at the relay because the relay is the only party that sees the
client hop. The relay derives the address from the socket, never from a header:
forwarding headers are client-forgeable, and the origin server strips them at
dispatch for that reason.

A request arriving with no attested peer falls back to the origin's
unattributed rate-limit bucket — bounded, not unlimited.

**Streaming responses.** `HTTP_RESPONSE_HEAD` goes out as soon as status and
headers are known, then body chunks are forwarded **as the server produces
them**, unbuffered. The 5 s `STREAM_OPEN` timeout is **time to first byte, not
time to completion**: a slow-starting response is timed out, a slow-finishing
one that already started is not.

### 5.3 The change feed — SSE, not a tunnelled WebSocket

`GET /v1/stream` is tunnelled as an **ordinary §5.2 request**: a plain `GET`
with a normal `Authorization` header, so the origin's auth middleware runs
unchanged in the place it always has. `HTTP_RESPONSE_HEAD` once, then the SSE
body streamed as `0x02` chunks.

Body encoding is SSE (`text/event-stream`), specified rather than implied —
inside SRP, WS length-delimits each frame, but the HTTP body is a byte stream
once it leaves the tunnel, so a non-SRP consumer needs a delimiter.

```
event: change
id: <vault_id>:<seq>
data: <json-encoded Change>

```

The `Lagged` case becomes `event: resync` with an empty `data:`.

> **The `id:` MUST carry the vault, not the bare `seq`.** `change_log` lives in
> each vault's own index database and `seq` comes from that database's
> `last_insert_rowid()`, so **`seq` is per vault**, while the feed is
> cross-vault — two vaults both emit `seq` 1, 2, 3. A bare `id: <seq>` would
> collide across vaults and make a `Last-Event-ID` resume land at the wrong
> position, **silently**.

**Unresolved:** whether `Last-Event-ID` is honoured by replaying missed events,
or forwarded to the origin untouched. Deferred on purpose — replay implies
buffering, which touches the no-relay-storage rule (§7). **Do not implement
either semantics until it is decided.** Until then an implementation MUST
ignore the header *deliberately and say so*: an EventSource client sends it
automatically, so silence reads as support.

**No inactivity timeout on this stream** — a quiet change feed is normal, not
stalled.

### 5.4 Closing

`CLOSE { stream_id }` from either side ends one stream without touching the
trunk. `CLOSE {}` with no `stream_id` ends the whole trunk. Tearing down a
trunk implicitly closes every stream on it; streams do not survive trunk loss,
and clients resume via their existing sync cursor rather than via trunk state.

## 6. Message catalog and error codes

Every control message is a JSON text frame carrying `{"v": 1, "type": ...}`.

| Type | Direction | Fields |
|---|---|---|
| `REGISTER_SERVER` | server→relay | `server_id`, `pubkey` |
| `CHALLENGE` | relay→server | `nonce` |
| `CHALLENGE_RESPONSE` | server→relay | `sig` |
| `REGISTERED` | relay→server | `trunk_id`, `public_address`, `heartbeat_interval_secs` |
| `DEREGISTER` | server→relay | — |
| `HELLO` | client→relay | `server_id` |
| `READY` | relay→client | `client_trunk_id` |
| `OPEN_STREAM` | client→relay | `attempt_id` |
| `STREAM_READY` | relay→client | `attempt_id`, `stream_id` |
| `STREAM_OPEN` | relay→server | `stream_id` |
| `STREAM_ACK` | server→relay | `stream_id` — the trunk has accepted it (§5.1) |
| `HTTP_REQUEST_HEAD` | client→relay→server | `stream_id`, `method`, `path`, `headers`; **relay sets `relay_peer_ip` on the server-ward hop** (§5.2) |
| `HTTP_RESPONSE_HEAD` | server→relay→client | `stream_id`, `status`, `headers` |
| *(binary `0x01` / `0x02`)* | either | `stream_id`, bytes |
| `PING` / `PONG` | either | — |
| `CLOSE` | either | `stream_id`? (absent = whole trunk) |
| `ERROR` | relay→either | `code`, `message`, `stream_id`? |

### Error codes

Codes are **strings**, not numbers.

| Code | Meaning |
|---|---|
| `protocol_error` | Malformed frame, bad or missing version, unknown type, or a client-sent `relay_peer_ip`. Close the connection. |
| `auth_failed` | Registration challenge failed, or a pubkey that does not match the recorded binding (§4.1). |
| `server_unreachable` | No live trunk for `server_id` after the 5 s `HELLO` wait. |
| `server_timeout` | Server did not ACK `STREAM_OPEN` within 5 s. |
| `trunk_lost` | Server trunk died — heartbeat timeout, closed, or superseded. |
| `trunk_superseded` | Replaced by a newer registration; drain window elapsed. |
| `rate_limited` | An abuse-control limit was hit. |
| `stream_closed` | Operation on a dead `stream_id`. |

**There is deliberately no code for a relay-side fault.** Every code above is
peer-attributable, so a relay that cannot proceed for its own reasons — a
failed nonce generator, a broken config — has nothing honest to send. It SHOULD
send `protocol_error`, which misattributes blame but leaks the least, and log
the real cause locally. Introducing an `internal_error` code would tell an
unauthenticated caller when it has found a way to break the relay.

**`protocol_error` MUST carry minimal detail**, and `auth_failed` MUST NOT say
*which* check failed. A malformed-frame scanner should not be told which part
of its framing was wrong, and a registration attacker should not learn whether
a signature was bad, a nonce expired, or a binding was refused.

### Abuse controls

Because the relay authenticates no clients, its real security surface is
resource exhaustion. A relay MUST be able to bound:

- concurrent streams per `server_id`;
- `HELLO` rate per source IP — note this bounds new *trunks*, not login
  attempts, since one trunk carries many streams;
- bandwidth per `server_id`.

A self-hosted relay MAY leave these uncapped by default — the operator only
ever spends their own resource. A public relay MUST enforce them. The specific
numbers are not fixed by this spec.

## 7. Non-goals for v1

- **End-to-end encryption of tunnelled payloads.** TLS terminates at the relay
  and content is plaintext to it. E2E is deferred and, when it lands, layers
  above this spec rather than amending it. Until then, self-hosted-first
  ordering (§2) is the mitigation.
- **Server key rotation.** §4.1 makes a differing pubkey for an already-bound
  `server_id` `auth_failed`, which is what makes squatting hard and also means
  a legitimate operator cannot rotate. Deferred deliberately, so the absence is
  a decision rather than something found missing later. A future version needs
  an authenticated rotation — plausibly signed by the *old* key, proving
  continuity — distinct from ordinary registration.
- **Automatic relay selection by quality.** A client races its candidates in
  the fixed order of §2 and takes the first that answers; it does not measure
  latency, rank relays, or migrate mid-session.
- **Relay-side storage.** No note content, no cursors, no replay buffers at
  rest. This is also why `Last-Event-ID` replay is unresolved.
- **NAT traversal / hole-punching.** Storm's transport is HTTP/WS over TCP, and
  real traversal reliability needs UDP underneath — a transport rewrite, not a
  relay feature.
- **mDNS / LAN discovery.** `local_address` stays plain configuration.
- **Vendor-specific wire behaviour.** No tunnel vendor, host, or topology
  appears anywhere in the framing (**R7**).
