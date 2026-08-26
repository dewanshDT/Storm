#!/usr/bin/env python3
"""End-to-end exercise of the server's identity and authentication endpoints.

Third companion to `e2e.py` and `mcp_e2e.py`, same style and no dependencies.
`e2e.py`'s checks are deliberately left alone — an unchanged pass there is what
says this slice broke nothing — so the `none`-tier surface gets its own file.

Usage:

    cargo run -- serve --vault-root /tmp/vaults --state /tmp/s &
    python3 apps/server/tests/auth_e2e.py

What it is really for, beyond "the endpoints answer":

  * they answer with **no Authorization header at all**, which depends on them
    being registered *below* the auth layer in `api.rs` — axum applies a layer
    only to the routes above it, and getting this wrong makes pairing
    impossible in a way nothing else would catch;
  * the signature actually verifies against the public key the same server
    published, over the domain-separated message a client will rebuild;
  * an ordinary route still demands the bearer token, so "unauthenticated"
    stayed the size of two routes;
  * no private key material is in any payload.

And, from the second half, the path a real client actually walks: the bootstrap
QR out of the server's own log, pairing, the first account, login, refresh
rotation, logout, and the lockout. **Nothing in this repo sent a `StormDevice`
credential before that half existed** — the credential every authentication
route requires — which is how three defects sat undisturbed on it, one of them
wedging the server's entire authentication on the first login attempt.

That half needs a *fresh* server: the bootstrap pairing session is created at
boot only when the user table is empty, and it reads the URI out of
`STORM_SERVER_LOG` (default `.dev/live-server.log`, which is where
`make test-live` puts it).

Exits non-zero if any check fails.
"""
import base64
import hashlib
import json
import os
import re
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get("STORM_BASE", "http://127.0.0.1:8484")
# Kept only so the check below can present it and be refused. There is no
# shared token any more; this is the string that used to be one.
RETIRED_TOKEN = "testtoken"

ok = 0
fail = 0


def check(label, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {label}")
    else:
        fail += 1
        print(f"  FAIL  {label}   {detail}")


def call_full(method, path, body=None, token=None, auth=None):
    """As `call`, but hands back the response headers too.

    `auth` sends a raw Authorization value, which is what the device tier needs
    (`StormDevice <id>:<secret>`); `token` keeps the `Bearer ` convenience the
    identity checks were written against.
    """
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    if auth is not None:
        req.add_header("Authorization", auth)
    elif token is not None:
        req.add_header("Authorization", f"Bearer {token}")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            # 201 and 204 answer with no body at all, so "no bytes" has to mean
            # an empty result rather than a decode error.
            raw = r.read()
            return r.status, (json.loads(raw) if raw else {}), dict(r.headers)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw or b"{}"), dict(e.headers)
        except json.JSONDecodeError:
            return e.code, {"error": raw.decode(errors="replace")}, dict(e.headers)


def call(method, path, body=None, token=None, auth=None):
    """A request that sends the Authorization header only when asked to."""
    status, body_out, _ = call_full(method, path, body, token, auth)
    return status, body_out


def b64url(s):
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


# ---- minimal Ed25519 verification (RFC 8032) ---------------------------
#
# Vendored rather than imported so this suite keeps the no-dependency rule the
# other two follow. It exists for one check, and it is the check that matters:
# that the signature a *running* server returns verifies against the public key
# that same server published. Rust's own test proves the crypto; this proves
# the two endpoints agree with each other over HTTP.

Q = 2**255 - 19
D = -121665 * pow(121666, Q - 2, Q) % Q
I = pow(2, (Q - 1) // 4, Q)


def _xrecover(y):
    xx = (y * y - 1) * pow(D * y * y + 1, Q - 2, Q)
    x = pow(xx, (Q + 3) // 8, Q)
    if (x * x - xx) % Q != 0:
        x = (x * I) % Q
    if x % 2 != 0:
        x = Q - x
    return x


_BY = 4 * pow(5, Q - 2, Q) % Q
_BX = _xrecover(_BY)
B = (_BX % Q, _BY % Q, 1, (_BX * _BY) % Q)


def _add(p, r):
    x1, y1, z1, t1 = p
    x2, y2, z2, t2 = r
    a = (y1 - x1) * (y2 - x2) % Q
    b = (y1 + x1) * (y2 + x2) % Q
    c = t1 * 2 * D * t2 % Q
    dd = z1 * 2 * z2 % Q
    e, f, g, h = b - a, dd - c, dd + c, b + a
    return (e * f % Q, g * h % Q, f * g % Q, e * h % Q)


def _mul(p, e):
    r = (0, 1, 1, 0)
    while e > 0:
        if e & 1:
            r = _add(r, p)
        p = _add(p, p)
        e >>= 1
    return r


def _decode_point(s):
    y = int.from_bytes(s, "little") & ((1 << 255) - 1)
    x = _xrecover(y)
    if x & 1 != (s[31] >> 7) & 1:
        x = Q - x
    p = (x, y, 1, (x * y) % Q)
    x1, y1, z1, t1 = p
    on_curve = (
        z1 % Q != 0
        and x1 * y1 % Q == z1 * t1 % Q
        and (y1 * y1 - x1 * x1 - z1 * z1 - D * t1 * t1) % Q == 0
    )
    if not on_curve:
        raise ValueError("point is not on the curve")
    return p


def ed25519_verify(public_key, message, signature):
    try:
        r = _decode_point(signature[:32])
        a = _decode_point(public_key)
    except ValueError:
        return False
    s = int.from_bytes(signature[32:], "little")
    h = int.from_bytes(
        hashlib.sha512(signature[:32] + public_key + message).digest(), "little"
    )
    lhs = _mul(B, s)
    rhs = _add(r, _mul(a, h))
    x1, y1, z1, _ = lhs
    x2, y2, z2, _ = rhs
    return (x1 * z2 - x2 * z1) % Q == 0 and (y1 * z2 - y2 * z1) % Q == 0


# ---- the identity endpoints ---------------------------------------------

print("--- GET /v1/server (tier: none) ---")

status, info = call("GET", "/v1/server")
check("answers with no Authorization header at all", status == 200, (status, info))
if status != 200:
    print("  cannot continue without the server's identity")
    sys.exit(1)

check(
    "server_id is srv_ + 26 Crockford characters",
    isinstance(info.get("server_id"), str)
    and info["server_id"].startswith("srv_")
    and len(info["server_id"]) == 30
    and all(c in "0123456789ABCDEFGHJKMNPQRSTVWXYZ" for c in info["server_id"][4:]),
    info.get("server_id"),
)
check("key_id is key_ prefixed", str(info.get("key_id", "")).startswith("key_"), info)
check("algorithm is ed25519", info.get("algorithm") == "ed25519", info)
check("name is a non-empty label", bool(info.get("name")), info)

try:
    public_key = b64url(info.get("public_key", ""))
except Exception as e:  # noqa: BLE001 - any decode failure is the same failure
    public_key = b""
    print(f"  (public_key did not decode: {e})")
check("public_key is 32 base64url bytes", len(public_key) == 32, info.get("public_key"))

check(
    "exposes exactly the identity fields and nothing else",
    set(info) == {"server_id", "name", "key_id", "algorithm", "public_key"},
    sorted(info),
)

# The two endpoints are unauthenticated, not "authenticated by anything". A
# wrong token must not be treated as a reason to refuse — a client that has been
# signed out still has to be able to re-pair.
status, info_bad = call("GET", "/v1/server", token="not-the-token")
check("a wrong token does not change the answer", status == 200 and info_bad == info, status)

status, info_again = call("GET", "/v1/server", token=RETIRED_TOKEN)
check("the identity is stable across requests", info_again == info, info_again)

print("--- POST /v1/server/challenge (tier: none) ---")

NONCE = "0123456789abcdef0123456789abcdef"
status, answer = call("POST", "/v1/server/challenge", {"nonce": NONCE})
check("signs a nonce with no Authorization header", status == 200, (status, answer))
check(
    "names the same server and key as GET /v1/server",
    answer.get("server_id") == info["server_id"]
    and answer.get("key_id") == info["key_id"],
    answer,
)

signature = b64url(answer.get("signature", "")) if status == 200 else b""
check("signature is 64 base64url bytes", len(signature) == 64, answer.get("signature"))

message = f"storm-challenge:v1:{info['server_id']}:{NONCE}".encode()
check(
    "the signature verifies against the published public key",
    len(signature) == 64 and ed25519_verify(public_key, message, signature),
    "the server cannot prove it holds the key it publishes",
)
check(
    "it is NOT a signature over the bare nonce",
    len(signature) == 64 and not ed25519_verify(public_key, NONCE.encode(), signature),
    "an unauthenticated endpoint that signs arbitrary bytes is a signing oracle",
)
check(
    "the server id is bound into the signed message",
    len(signature) == 64
    and not ed25519_verify(
        public_key, b"storm-challenge:v1:srv_SOMEONEELSE:" + NONCE.encode(), signature
    ),
    "a signature that ignores the server id can be replayed by another server",
)

status, _ = call("POST", "/v1/server/challenge", {"nonce": "tooshort"})
check("a short nonce is refused", status == 400, status)

status, _ = call("POST", "/v1/server/challenge", {"nonce": "0123456789abcdef:oops"})
check("a nonce containing the field separator is refused", status == 400, status)

status, _ = call("POST", "/v1/server/challenge", {"nonce": "x" * 129})
check("an over-long nonce is refused", status == 400, status)

print("--- the tier boundary held ---")

status, _ = call("GET", "/v1/vaults")
check("an ordinary route demands a credential", status == 401, status)

# **The cutover, asserted from outside.** `testtoken` opened every session-tier
# route until this release. Nothing accepts a bare string now, and the section
# below goes on to earn a real session the way a client does.
status, _ = call("GET", "/v1/vaults", token=RETIRED_TOKEN)
check("the retired shared token is refused", status == 401, status)

status, _ = call("GET", "/v1/health")
check("health is unauthenticated as before", status == 200, status)


# ---- pairing, the first account, and login ------------------------------
#
# Everything above this line is slice 1: two unauthenticated routes. What
# follows is the path a real client walks on first run, and until it was
# written nothing in the repo had ever sent a `StormDevice` credential — the
# credential every authentication route requires. Three defects were sitting on
# it, one of which wedged the server's whole auth on the first login attempt.
#
# It needs a *fresh* server, because the bootstrap pairing session is created
# at boot only when the user table is empty. `make test-live` gives it one.

print("--- the bootstrap pairing QR ---")

LOG = os.environ.get("STORM_SERVER_LOG", ".dev/live-server.log")

bootstrap_uri = None
try:
    with open(LOG, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            found = re.search(r"storm://pair\?\S+", line)
            if found:
                bootstrap_uri = found.group(0).rstrip('"')
except OSError as e:
    print(f"  (could not read {LOG}: {e})")

check(
    "the server logged a bootstrap pairing URI at boot",
    bootstrap_uri is not None,
    f"looked in {LOG}; this section needs a server whose user table was empty at boot",
)
if bootstrap_uri is None:
    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1)

qr = urllib.parse.parse_qs(urllib.parse.urlparse(bootstrap_uri).query)
check("the QR is version 1", qr.get("v") == ["1"], qr)
check(
    "the QR names the same server and key the identity endpoint publishes",
    qr.get("sid") == [info["server_id"]] and qr.get("pk") == [info["public_key"]],
    qr,
)
check("the QR carries an expiry", bool(qr.get("exp", [""])[0]), qr)
nonce = qr.get("n", [""])[0]
check("the QR carries a pairing nonce", len(nonce) >= 24, nonce)

print("--- POST /v1/pair (tier: none) ---")

# A device pairs before anyone can log in, so this route cannot require a
# credential — there is nothing to present yet.
status, paired = call(
    "POST", "/v1/pair", {"n": nonce, "name": "e2e device", "platform": "linux"}
)
check("pairs with no Authorization header at all", status == 200, (status, paired))
if status != 200:
    print("  cannot continue without a device credential")
    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1)

check(
    "hands back a device id and secret",
    bool(paired.get("device_id")) and bool(paired.get("device_secret")),
    sorted(paired),
)
check(
    "repeats the identity the client is about to pin",
    paired.get("server_id") == info["server_id"]
    and paired.get("public_key") == info["public_key"]
    and paired.get("key_id") == info["key_id"],
    paired,
)

DEVICE = f"StormDevice {paired['device_id']}:{paired['device_secret']}"

# Single-use, or a QR photographed over someone's shoulder is a second device.
status, again = call(
    "POST", "/v1/pair", {"n": nonce, "name": "thief", "platform": "linux"}
)
check("the pairing nonce is single-use", status == 409, (status, again))

status, _ = call(
    "POST", "/v1/pair", {"n": "A" * 32, "name": "thief", "platform": "linux"}
)
check("an invented nonce is refused", status == 401, status)

print("--- the device tier ---")

status, _ = call("GET", "/v1/users")
check("the user list refuses an anonymous caller", status == 401, status)

# A10: the legacy shared token is owner-equivalent on *session* routes and must
# not reach the device tier. Since the cutover it reaches nothing at all, which
# is the stronger form of the same guarantee.
status, _ = call("GET", "/v1/users", token=RETIRED_TOKEN)
check("the retired shared token cannot reach the device tier", status == 401, status)

status, users = call("GET", "/v1/users", auth=DEVICE)
check("a paired device may list users", status == 200, (status, users))
check("a fresh server has no accounts yet", users == [], users)

print("--- POST /v1/users/first ---")

PASSWORD = "a-long-enough-password"

status, _ = call(
    "POST", "/v1/users/first", {"username": "dewansh", "password": "short"}, auth=DEVICE
)
check("a short password is refused", status == 422, status)

# Refused rather than truncated: accept 200 characters, hash the first 72, and
# every password sharing that prefix opens the account.
status, _ = call(
    "POST",
    "/v1/users/first",
    {"username": "dewansh", "password": "x" * 1100},
    auth=DEVICE,
)
check("an over-long password is refused, not truncated", status == 422, status)

status, _ = call(
    "POST", "/v1/users/first", {"username": "dewänsh", "password": PASSWORD}, auth=DEVICE
)
check("a non-ASCII username is refused", status == 422, status)

status, _ = call(
    "POST", "/v1/users/first", {"username": "dewansh", "password": PASSWORD}, auth=DEVICE
)
check("the first account is created", status == 201, status)

# The window closes and stays closed. A *different* username, so a
# duplicate-name refusal cannot be what makes this pass — which is exactly how
# the hole stayed open: this handler hardcodes the owner role, so a second
# account through it is a second owner.
status, second = call(
    "POST",
    "/v1/users/first",
    {"username": "someone-else", "password": PASSWORD},
    auth=DEVICE,
)
check("the bootstrap window is closed afterwards", status == 409, (status, second))

status, users = call("GET", "/v1/users", auth=DEVICE)
check("and no second account reached the table", len(users) == 1, users)
check(
    "the first account is an owner",
    bool(users) and str(users[0].get("role", "")).lower() == "owner",
    users,
)
check(
    "a listed user carries no password material",
    users and not any("hash" in k or "password" in k for k in users[0]),
    sorted(users[0]) if users else [],
)

print("--- POST /v1/auth/login ---")

status, refused = call(
    "POST",
    "/v1/auth/login",
    {"username": "dewansh", "password": "not-the-password"},
    auth=DEVICE,
)
check("a wrong password is refused", status == 401, (status, refused))

status, session = call(
    "POST", "/v1/auth/login", {"username": "dewansh", "password": PASSWORD}, auth=DEVICE
)
check("the right password issues a session", status == 200, (status, session))
if status != 200:
    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1)

check(
    "the session carries an access and a refresh token",
    bool(session.get("access_token")) and bool(session.get("refresh_token")),
    sorted(session),
)
check(
    "the session names the user and the device it is bound to",
    bool(session.get("user_id")) and session.get("device_id") == paired["device_id"],
    session,
)
check(
    "the login answer carries no password material",
    not any("hash" in k or "password" in k for k in session),
    sorted(session),
)

status, _ = call("GET", "/v1/vaults", token=session["access_token"])
check("the access token opens a session-tier route", status == 200, status)

# Two tokens with different jobs. A refresh token that authenticates requests
# is an access token with a 180-day life.
status, _ = call("GET", "/v1/vaults", token=session["refresh_token"])
check("the refresh token is not an access token", status == 401, status)

print("--- refresh rotation ---")

# Refresh is device tier: rotation is bound to the device that holds the
# session, so the device credential travels with it. Sending it without one
# answers 401 — which also makes the replay check below pass for the wrong
# reason if you forget, since a refresh that never happened cannot be replayed.
status, rotated = call(
    "POST",
    "/v1/auth/refresh",
    {"refresh_token": session["refresh_token"]},
    auth=DEVICE,
)
check("a refresh token buys a new pair", status == 200, (status, rotated))
if status != 200:
    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1)

check(
    "and the pair is actually new",
    rotated.get("access_token") != session["access_token"]
    and rotated.get("refresh_token") != session["refresh_token"],
    "rotation that returns the same token is not rotation",
)

status, replayed = call(
    "POST",
    "/v1/auth/refresh",
    {"refresh_token": session["refresh_token"]},
    auth=DEVICE,
)
check("the old refresh token is refused after rotation", status == 401, (status, replayed))

# A replayed refresh token means one of two things: a stolen token, or a client
# that lost the race. Neither is safe to keep serving, so the *whole session*
# dies rather than just the request — which is why the logout check below needs
# a fresh login rather than reusing this one.
status, _ = call("GET", "/v1/vaults", token=rotated["access_token"])
check("a replayed refresh token revokes the whole session", status == 401, status)

print("--- logout ---")

status, fresh = call(
    "POST", "/v1/auth/login", {"username": "dewansh", "password": PASSWORD}, auth=DEVICE
)
check("logging in again after the replay revocation works", status == 200, status)
if status != 200:
    print(f"\n{ok} passed, {fail} failed")
    sys.exit(1)

status, _ = call("GET", "/v1/vaults", token=fresh["access_token"])
check("the new session's access token works before logout", status == 200, status)

status, _ = call("POST", "/v1/auth/logout", token=fresh["access_token"])
check("logout answers 204", status == 204, status)

status, _ = call("GET", "/v1/vaults", token=fresh["access_token"])
check("the access token is dead after logout", status == 401, status)

print("--- the login lockout (last: it locks the account for a minute) ---")

# Deliberately last. The account is locked for a minute afterwards, so anything
# needing a working login has to have run already.
locked = None
for attempt in range(8):
    status, body, headers = call_full(
        "POST",
        "/v1/auth/login",
        {"username": "dewansh", "password": "not-the-password"},
        auth=DEVICE,
    )
    if status == 429:
        locked = (body, headers, attempt + 1)
        break
    if attempt < 3:
        check(f"attempt {attempt + 1} is a plain refusal, not a lockout", status == 401, status)

check("repeated wrong passwords end in a lockout", locked is not None, "no 429 in 8 attempts")
if locked:
    body, headers, attempts = locked
    retry_header = next(
        (v for k, v in headers.items() if k.lower() == "retry-after"), None
    )
    # The header is the correct HTTP answer; the body is what the client
    # renders. "Too many attempts" without a number invites the retry it is
    # trying to stop.
    check("the lockout carries a Retry-After header", retry_header is not None, sorted(headers))
    check(
        "the header parses as a number of seconds",
        retry_header is not None and retry_header.isdigit() and int(retry_header) > 0,
        retry_header,
    )
    check(
        "and the body says how long too",
        isinstance(body.get("retry_after"), int) and body["retry_after"] > 0,
        body,
    )
    check("the lockout is a 429, never a 401", body.get("error") == "rate_limited", body)

print("--- login rate limiting (after the lockout: it spends the bucket) ---")

# The throttle and the per-user lockout answer with the *same* 429 shape, so a
# test using a real username could not say which one refused it. Every attempt
# below uses a username that does not exist: no account means no lockout is
# possible, so a 429 here can only be the rate limiter.
#
# That is also the case the limiter exists for. A junk username still pays for
# a full Argon2id verify — deliberately, so response time cannot enumerate
# accounts — and can never trigger a lockout, which is what makes it the
# cheapest way to take the login path down.
#
# **The burst has to be concurrent.** Sent one at a time, each attempt costs a
# whole Argon2id verify (seconds, in the debug build `make test-live` uses) and
# the bucket refills faster than the requests arrive — so the limiter correctly
# allows every one of them and the suite proves nothing. A real flood does not
# wait for its own responses, and neither does this.
#
# Runs last for the same reason the lockout does: it leaves the caller's bucket
# empty for a couple of minutes.
FLOOD = 45

flood_results = []
flood_lock = threading.Lock()


def one_junk_login(n):
    try:
        result = call_full(
            "POST",
            "/v1/auth/login",
            {"username": f"no-such-user-{n}", "password": "not-the-password"},
            auth=DEVICE,
        )
    except Exception as e:  # a dropped connection is a result to report, not a crash
        result = (None, {"error": str(e)}, {})
    with flood_lock:
        flood_results.append(result)


flood = [threading.Thread(target=one_junk_login, args=(n,)) for n in range(FLOOD)]
for thread in flood:
    thread.start()
for thread in flood:
    thread.join()

seen = sorted({str(status) for status, _, _ in flood_results})
statuses = [status for status, _, _ in flood_results]
throttled = next(
    ((body, headers) for status, body, headers in flood_results if status == 429),
    None,
)

check(f"a concurrent flood of {FLOOD} junk logins is throttled", throttled is not None, seen)
check("the flood is throttled rather than refused outright", 401 in statuses, seen)
if throttled:
    body, headers = throttled
    retry_header = next((v for k, v in headers.items() if k.lower() == "retry-after"), None)
    check(
        "the throttle carries a Retry-After header",
        retry_header is not None,
        sorted(headers),
    )
    check(
        "the throttle's Retry-After parses as seconds",
        retry_header is not None and retry_header.isdigit() and int(retry_header) > 0,
        retry_header,
    )
    check(
        "the throttle reuses the rate_limited body",
        body.get("error") == "rate_limited",
        body,
    )
    check(
        "and the body says how long to wait",
        isinstance(body.get("retry_after"), int) and body["retry_after"] > 0,
        body,
    )

# What this suite cannot cover, stated rather than silently skipped: every
# request here comes from one address, so the *global* ceiling and the
# per-caller isolation ("another address is unaffected") are exercised only by
# the Rust tests in `auth/ratelimit.rs` and `api.rs`, which can name the caller.

print(f"\n{ok} passed, {fail} failed")
sys.exit(1 if fail else 0)
