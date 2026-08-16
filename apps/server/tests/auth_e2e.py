#!/usr/bin/env python3
"""End-to-end exercise of the server's identity endpoints.

Third companion to `e2e.py` and `mcp_e2e.py`, same style and no dependencies.
`e2e.py`'s checks are deliberately left alone — an unchanged pass there is what
says this slice broke nothing — so the `none`-tier surface gets its own file.

Usage:

    cargo run -- serve --vault-root /tmp/vaults --state /tmp/s --token testtoken &
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

Exits non-zero if any check fails.
"""
import base64
import hashlib
import json
import os
import sys
import urllib.error
import urllib.request

BASE = os.environ.get("STORM_BASE", "http://127.0.0.1:8484")
TOKEN = os.environ.get("STORM_TOKEN", "testtoken")

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


def call(method, path, body=None, token=None):
    """A request that sends the Authorization header only when asked to."""
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    if token is not None:
        req.add_header("Authorization", f"Bearer {token}")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw or b"{}")
        except json.JSONDecodeError:
            return e.code, {"error": raw.decode(errors="replace")}


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

status, info_again = call("GET", "/v1/server", token=TOKEN)
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
check("an ordinary route still demands the token", status == 401, status)

status, _ = call("GET", "/v1/vaults", token=TOKEN)
check("and still answers with it", status == 200, status)

status, _ = call("GET", "/v1/health")
check("health is unauthenticated as before", status == 200, status)

print(f"\n{ok} passed, {fail} failed")
sys.exit(1 if fail else 0)
