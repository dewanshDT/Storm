# Storm Relay Protocol — wire spec (v1)

> **Status: spec only, nothing built.** The relay is designed in `PLAN.md`
> (decision 52 and the relay decisions R1–R13) and the vault note *Storm Relay Protocol*, which holds
> the rationale this file deliberately leaves out. This document is the
> citable artifact for §2–§4 of the implementation checklist: what goes on the
> wire, not why. Where the two disagree, the vault note and `PLAN.md` are
> current and this file is amended in the same change that moves them.
>
> Normative language: MUST / SHOULD / MAY as in RFC 2119.

## 1. Principle

**The relay is a reverse proxy of the existing Storm wire protocol, not a
second protocol.** A client connected through a relay speaks exactly what it
speaks on the LAN — same REST surface, same SSE stream, same framing where
framing exists. The relay terminates transport, never interpretation: it does
not parse note content, does not cache payloads, and cannot distinguish a note
sync from an auth call except where the protocol requires it to (routing,
rate limiting, peer attribution).

A consequence worth stating: **the protocol is the deliverable, not any one
deployment of it.** Nothing in this spec names a tunnel vendor, a hosting
provider, or a specific network topology. A protocol with one implementation
becomes that implementation; R7 forbids baking Cloudflare-specific assumptions
into the wire format.

## 2. Parties

| Party | Role |
|---|---|
| Client | A Storm app (Flutter) or MCP client, already paired with a server. |
| Relay | An untrusted forwarder. Terminates TLS, routes frames, attributes peers. Sees ciphertext-in-transit only where E2E is later added; today it sees plaintext and is trusted with availability, not confidentiality. |
| Origin server | The user's Storm sync server. Registered with the relay once; serves every trunk. |

## 3. Framing

All relay traffic after the HTTP upgrade is a sequence of binary frames:

```
type      1 byte
stream_id 4 bytes, big-endian
payload   remainder of the frame; length implied by the transport
```

Two body types only:

| Type | Direction | Payload |
|---|---|---|
| DATA | both | Opaque bytes belonging to the stream. Never inspected by the relay beyond routing. |
| CTRL | both | Control frames (stream open/close, errors). Structured per §6. |

The payload of a DATA frame is the proxied HTTP exchange verbatim — request or
response bytes as the origin would have produced them. The relay MUST NOT
rewrite them.

### Version pin

The handshake carries `{"v": 1}`. A client MUST advertise its protocol
version at registration; a server MUST refuse any peer whose major version is
not exactly `1`. Minor-version tolerance is deliberately unspecified until a
minor version exists to tolerate.

## 4. Registration and authentication

A device registers with the relay once, out of band of any trunk:

1. The client generates or loads its device keypair (Ed25519, the same pair
   pinned at pairing).
2. It sends the relay a registration request carrying its device id and public
   key.
3. The relay issues a challenge; the client signs it under the domain string

   ```
   storm-relay-auth:v1:
   ```

   concatenated with the challenge bytes. The domain string is load-bearing:
   it prevents a signature made for relay auth being replayed into any other
   Storm signature check (pairing challenges, server challenges), and vice
   versa. Every new signed context in the relay protocol MUST mint its own
   `storm-relay-auth:v1:<context>:` prefix rather than reusing an existing one.
4. On success the relay records the device's public key and issues credentials
   for opening trunks.

Re-registration with the same device id MUST require a signature from the
previously registered key, or the registration is treated as a new device.

## 4.1 Binding rules

Credentials are worthless if they float free of what they authenticate. Each
artifact binds to exactly these things, no more:

| Artifact | Binds to | Does not bind to |
|---|---|---|
| Device keypair | Device identity | Any vault, any session, any IP |
| Trunk credential | Device identity + origin server id | Any single vault |
| Stream | Its trunk + one vault id (fixed at open) | Client process lifetime |

Rules:

- A trunk credential MUST be rejected on any origin server other than the one
  named in it.
- A stream opened on vault V MUST NOT be reusable for vault W; a change of
  vault requires closing the stream and opening a new one.
- Nothing binds to an IP address. Devices roam; the relay attributes peers
  per-request instead (§5.2).

## 4.3 Trunk lifecycle

The trunk is the long-lived connection between one client and its origin
server, multiplexing all streams:

- **Establishment.** Client connects to the relay, presents its trunk
  credential, names its origin server. The relay validates the binding (§4.1)
  and either attaches to an existing server-side trunk endpoint or asks the
  origin to open one.
- **Keepalive.** Either side MAY send a keepalive control frame. Silence on
  the trunk is not health; a side that receives nothing (keepalive or data)
  for the agreed idle window SHOULD probe before declaring the trunk dead.
- **Teardown.** Either side tears down with a control frame carrying a reason
  code (§6). Tearing down the trunk implicitly closes every stream on it.
  Streams do not survive trunk loss; clients reconnect and resume via their
  existing sync cursor, not via trunk state.

## 4.4 The relay set, and how a client learns it

A server MAY register with several relays at once — a self-hosted one and a
public one, say — and maintains one trunk per relay. Registration, and
supersession (§4.3), are scoped **per relay**: superseding a trunk on relay A
MUST NOT disturb a concurrent trunk on relay B.

A server MUST advertise only the relays it is **currently registered with**,
never the ones merely present in its configuration. A relay that a server
failed to register with is a dead path, and a client that dials it spends its
whole connection-race budget on a candidate that cannot answer.

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

`public_address` is `wss://<relay-host>/connect/<server_id>` — derived, not
allocated. Any client holding a `server_id` can construct it, and the relay
hands out no opaque identifier.

## 5. Stream lifecycle

Streams are the unit of multiplexing over a trunk, identified by `stream_id`:

- **Open.** A side allocates an unused `stream_id` on its trunk and sends a
  CTRL open frame naming the target vault. The peer acknowledges or refuses.
- **Data.** DATA frames flow in both directions. Ordering within a stream is
  guaranteed by the transport; ordering across streams is not defined and MUST
  NOT be relied on.
- **Close.** Either side closes with a CTRL frame. Half-open states are not
  modelled: a close ends the stream in both directions.
- `stream_id` reuse on the same trunk is forbidden until the previous stream
  bearing that id has closed on both sides.

## 5.2 Peer attribution (`relay_peer_ip`)

Two rules, and they exist because the login limiter needs a caller identity
that survives the relay:

1. **The relay derives caller IP from the socket, never from headers.**
   `X-Forwarded-For` and friends are client-forgeable; a relay that trusts them
   converts the login rate limiter into a suggestion. This is the same reason
   the origin server itself refuses to read forwarding headers for security
   decisions (`FORWARDING_HEADERS` in `api.rs` is load-bearing for a different,
   non-security check).
2. **The derived peer IP travels to the origin out of band.** The mechanism is
   relay-internal — a signed header, a metadata frame, or an in-process call
   when relay and origin share a process. The spec requires only that the
   origin receive an attested peer address per request and that a client
   cannot forge it. The origin's `CallerKey::Unattributed` bucket (see the
   login limiter) is what a missing attestation falls back to: bounded, not
   unlimited.

## 5.3 Change notification (SSE)

Vault change notification rides the existing SSE shape end to end; the relay
proxies it as data. Event ids are composite:

```
id: <vault_id>:<seq>
```

where `<seq>` is the vault's `change_log.seq`. A client resuming after a drop
replays its per-vault cursor against the origin's existing changes endpoint;
the SSE stream itself is not the cursor store.

**Unresolved:** whether the relay honours `Last-Event-ID` on reconnect by
replaying missed events, or forwards it to the origin untouched. This is a
§2 checklist item, deferred on purpose — replay implies buffering, which
touches the no-relay-storage rule. Do not implement either semantics until
it is decided.

## 6. Message catalog and error codes

Control frame payloads are JSON. Every CTRL frame carries a `kind`; error
kinds carry a numeric `code`.

### Control kinds

| Kind | Direction | Meaning |
|---|---|---|
| `hello` | c→r | Handshake; carries `{"v": 1}`, device id, origin server id. |
| `ready` | r→c | Trunk established. |
| `open` | both | Open a stream; names the vault. |
| `open_ack` / `open_refuse` | both | Stream accepted / refused (`code`). |
| `data` | both | Wrapper when DATA must ride a control context; ordinary DATA frames need none. |
| `close` | both | Close a stream or the trunk; carries a reason `code`. |
| `ping` / `pong` | both | Keepalive. |

### Error codes

| Code | Name | Meaning |
|---|---|---|
| 1 | `auth_failed` | Signature or credential invalid. |
| 2 | `version_mismatch` | Major protocol version is not 1. |
| 3 | `unknown_origin` | Named origin server is not registered. |
| 4 | `binding_violation` | Credential used outside its §4.1 binding. |
| 5 | `vault_not_found` | Stream open named a vault the origin does not know. |
| 6 | `trunk_gone` | Trunk torn down; streams died with it. |
| 7 | `overloaded` | Relay refusing new work; retry with backoff. |

Numeric values are assigned here and MUST NOT be reused or reordered; add new
codes at the end.

## Non-goals

- **End-to-end encryption inside the relay protocol.** TLS terminates at the
  relay; content is plaintext to it today. E2E is deferred and, when it lands,
  layers above this spec rather than amending it.
- **Automatic relay selection by quality.** A client races its candidates and
  takes the first that answers (§4.4); it does not measure latency, rank
  relays, or migrate mid-session to a better one. A dropped connection
  re-races from scratch.
- **Relay-side storage.** No note content, no cursors, no replay buffers at
  rest on the relay. (This is also why Last-Event-ID replay is unresolved.)
- **Vendor-specific wire behaviour.** No tunnel vendor, host, or topology
  appears anywhere in the framing (R7).
