"""Getting a real credential, for the suites that used to send a shared token.

Before the cutover every integration suite authenticated with `testtoken`, so
none of them exercised authentication at all — they exercised a constant
compare. There is no shared token now, so each suite has to walk the path a
real client walks: read the bootstrap pairing nonce out of the server's own
log, pair a device, create the first account, and log in.

**One implementation, imported by every suite.** Three copies of a bootstrap
sequence is three things to update when the flow changes, and `ops.rs` exists
because that kind of duplication is how Storm's surfaces drift apart.

The bootstrap nonce is minted at boot *only when the user table is empty*, so
anything importing this needs a fresh server. `make test-live` gives each suite
one.
"""

import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request

PASSWORD = "correct horse battery staple"


def _call(base, method, path, body=None, auth=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{base}{path}", data=data, method=method)
    if auth:
        req.add_header("Authorization", auth)
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        raw = e.read() or b"{}"
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, {}


def bootstrap_nonce(log_path=None):
    """The pairing nonce the server printed at boot, or None."""
    log_path = log_path or os.environ.get("STORM_SERVER_LOG", ".dev/live-server.log")
    try:
        with open(log_path, encoding="utf-8", errors="replace") as fh:
            uri = None
            for line in fh:
                found = re.search(r"storm://pair\?\S+", line)
                if found:
                    uri = found.group(0).rstrip('"')
    except OSError:
        return None
    if uri is None:
        return None
    qs = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)
    return qs.get("n", [None])[0]


def inherited():
    """The credentials the harness already bootstrapped, if it did.

    `STORM_SESSION` / `STORM_DEVICE` are set by `bootstrap.py` via the
    Makefile. A suite run by hand against its own fresh server sets neither and
    claims the server itself.
    """
    session = os.environ.get("STORM_SESSION")
    device = os.environ.get("STORM_DEVICE")
    return (session, device) if session else (None, None)


def sign_in(base, username="e2e", log_path=None):
    """Pairs a device, creates the first account if needed, and logs in.

    Returns `(session_bearer, device_header, user_id)`. Raises with a message
    that says which step failed — a suite that cannot authenticate has nothing
    left to test, and "401 on everything" is a useless way to learn that.
    """
    session, device = inherited()
    if session:
        return session, device, ""

    nonce = bootstrap_nonce(log_path)
    if nonce is None:
        raise RuntimeError(
            "no bootstrap pairing nonce in the server log — this suite needs a "
            "server whose user table was empty at boot "
            f"(looked in {log_path or os.environ.get('STORM_SERVER_LOG', '.dev/live-server.log')})"
        )

    status, paired = _call(
        base, "POST", "/v1/pair", {"n": nonce, "name": "e2e", "platform": "linux"}
    )
    if status != 200:
        raise RuntimeError(f"pairing failed: {status} {paired}")
    device = f"StormDevice {paired['device_id']}:{paired['device_secret']}"

    # The first account, if this server has none. A second run against the same
    # server answers 409, which is correct and not a failure here.
    status, _ = _call(
        base,
        "POST",
        "/v1/users/first",
        {"username": username, "password": PASSWORD},
        auth=device,
    )
    # 201 on success, 409 once the server has an owner. Both are fine — this
    # helper only needs *an* account to log into, not to have made it.
    if status not in (200, 201, 409):
        raise RuntimeError(f"creating the first user failed: {status}")

    status, session = _call(
        base,
        "POST",
        "/v1/auth/login",
        {"username": username, "password": PASSWORD},
        auth=device,
    )
    if status != 200:
        raise RuntimeError(f"login failed: {status} {session}")

    return f"Bearer {session['access_token']}", device, session["user_id"]


def mint_mcp_key(base, session_bearer, name="e2e mcp key"):
    """An `stk_` key, for the suite that drives `/mcp`."""
    status, created = _call(
        base, "POST", "/v1/keys", {"name": name}, auth=session_bearer
    )
    if status != 200:
        raise RuntimeError(f"minting an MCP key failed: {status} {created}")
    return f"Bearer {created['secret']}"
