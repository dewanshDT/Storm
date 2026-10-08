#!/usr/bin/env python3
"""The v6 single-user migration against a v5 auth.db made by the real pre-v6
binary (OLD_SERVER_BIN), then booted by this build (SERVER_BIN).
`make test-migration` runs it."""

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

OLD_BIN = os.environ.get("OLD_SERVER_BIN")
NEW_BIN = os.environ.get("SERVER_BIN")
if not OLD_BIN or not NEW_BIN:
    print("set OLD_SERVER_BIN (pre-v6) and SERVER_BIN (this build)", file=sys.stderr)
    sys.exit(2)

PASSWORD = "the owner's long password"
MEMBER_PASSWORD = "the member's long password"

ok = fail = 0


def check(name, cond, detail=""):
    global ok, fail
    if cond:
        ok += 1
        print(f"  PASS  {name}")
    else:
        fail += 1
        print(f"  FAIL  {name}  {detail}")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


PORT = free_port()
BASE = f"http://127.0.0.1:{PORT}"
WORK = tempfile.mkdtemp(prefix="storm-migration-e2e-")
STATE = os.path.join(WORK, "state")
VAULTS = os.path.join(WORK, "vaults")
os.makedirs(os.path.join(VAULTS, "notes"))
with open(os.path.join(VAULTS, "notes", "Seed.md"), "w") as fh:
    fh.write("# Seed\n")
LOG = os.path.join(WORK, "server.log")


def call(method, path, body=None, auth=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method)
    if auth:
        req.add_header("Authorization", auth)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    req.add_header("Accept", "application/json, text/event-stream")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
            try:
                return r.status, json.loads(raw) if raw else {}
            except json.JSONDecodeError:
                return r.status, {}
    except urllib.error.HTTPError as e:
        raw = e.read() or b"{}"
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, {}


def start(binary):
    log = open(LOG, "a")
    p = subprocess.Popen(
        [binary, "serve", "--vault-root", VAULTS, "--state", STATE, "--port", str(PORT), "--mcp"],
        stdout=log, stderr=subprocess.STDOUT,
    )
    for _ in range(240):
        if p.poll() is not None:
            raise RuntimeError(f"{binary} exited; log:\n{open(LOG).read()[-3000:]}")
        try:
            if call("GET", "/v1/health")[0] == 200:
                return p
        except Exception:
            pass
        time.sleep(0.25)
    raise RuntimeError("server did not start")


def stop(p):
    p.terminate()
    p.wait(timeout=30)


def bootstrap_nonce():
    nonce = None
    for line in open(LOG, encoding="utf-8", errors="replace"):
        if "storm://pair?" in line:
            uri = line[line.index("storm://pair?"):].split()[0].rstrip('"')
            nonce = urllib.parse.parse_qs(urllib.parse.urlparse(uri).query)["n"][0]
    return nonce


def pair(nonce, name):
    status, paired = call("POST", "/v1/pair", {"n": nonce, "name": name, "platform": "linux"})
    assert status == 200, (status, paired)
    return paired["device_id"], f"StormDevice {paired['device_id']}:{paired['device_secret']}"


def old_cli(*args, password):
    subprocess.run([OLD_BIN, *args, "--state", STATE, "--password-stdin"],
                   input=password.encode(), check=True, capture_output=True)


def mcp(auth):
    return call("POST", "/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}, auth=auth)[0]


try:
    print("=== a v5 server, made by the pre-v6 binary ===")
    old = start(OLD_BIN)
    nonce = bootstrap_nonce()
    _, owner_device = pair(nonce, "owner phone")
    status, _ = call("POST", "/v1/users/first", {"username": "owner", "password": PASSWORD}, auth=owner_device)
    assert status == 201, status
    status, owner_login = call("POST", "/v1/auth/login", {"username": "owner", "password": PASSWORD},
                               auth=owner_device)
    assert status == 200, owner_login
    owner = f"Bearer {owner_login['access_token']}"
    owner_refresh = owner_login["refresh_token"]
    call("PUT", "/v1/config/mcp", {"enabled": True, "writable": False}, auth=owner)
    status, key = call("POST", "/v1/keys", {"name": "laptop"}, auth=owner)
    assert status == 200, key
    owner_key = f"Bearer {key['secret']}"

    old_cli("user", "add", "member", "--role", "member", password=MEMBER_PASSWORD)
    old_cli("user", "add", "owner2", "--role", "owner", password=MEMBER_PASSWORD)
    # The member signs in on a device of their own, paired by the owner.
    status, qr = call("POST", "/v1/pairings", {"purpose": "add_device"}, auth=owner)
    assert status == 200, qr
    member_device_id, member_device = pair(qr["n"], "member phone")
    status, member_login = call("POST", "/v1/auth/login",
                                {"username": "member", "password": MEMBER_PASSWORD}, auth=member_device)
    assert status == 200, member_login
    member = f"Bearer {member_login['access_token']}"
    status, mkey = call("POST", "/v1/keys", {"name": "member laptop"}, auth=member)
    assert status == 200, mkey
    member_key = f"Bearer {mkey['secret']}"
    check("the v5 server serves the member before the upgrade", call("GET", "/v1/vaults", auth=member)[0] == 200)
    stop(old)

    print("\n=== the same state, booted by this build ===")
    new = start(NEW_BIN)
    check("auth.db.pre-v6 was written", os.path.exists(os.path.join(STATE, "auth.db.pre-v6")))
    check("the owner's existing session still works", call("GET", "/v1/vaults", auth=owner)[0] == 200)
    check("the owner's existing key still reaches /mcp", mcp(owner_key) != 401)
    check("the member's session is refused", call("GET", "/v1/vaults", auth=member)[0] == 401)
    check("the member's key is refused", mcp(member_key) == 401)
    status, _ = call("POST", "/v1/auth/login", {"password": MEMBER_PASSWORD}, auth=member_device)
    check("the member's own device is revoked", status == 401, status)
    status, fresh = call("POST", "/v1/auth/login", {"password": PASSWORD}, auth=owner_device)
    check("the owner signs in with only the password", status == 200, (status, fresh))
    status, _ = call("POST", "/v1/auth/login", {"username": "owner", "password": PASSWORD}, auth=owner_device)
    check("an old client's username is ignored", status == 200, status)
    status, _ = call("POST", "/v1/auth/login", {"password": MEMBER_PASSWORD}, auth=owner_device)
    check("a removed account's password opens nothing", status == 401, status)
    status, refreshed = call("POST", "/v1/auth/refresh", {"refresh_token": owner_refresh}, auth=owner_device)
    check("the owner's refresh token still rotates", status == 200, (status, refreshed))
    status, config = call("GET", "/v1/config", auth=f"Bearer {refreshed.get('access_token', '')}")
    check("registration is gone from the config", status == 200 and "allow_registration" not in config, config)
    check("the user list is gone", call("GET", "/v1/users", auth=owner_device)[0] != 200)
    stop(new)

    print("\n=== a second boot ===")
    backup_mtime = os.path.getmtime(os.path.join(STATE, "auth.db.pre-v6"))
    new = start(NEW_BIN)
    signed_in = f"Bearer {fresh['access_token']}"
    check("it still serves the owner", call("GET", "/v1/vaults", auth=signed_in)[0] == 200)
    status, again = call("POST", "/v1/auth/login", {"password": PASSWORD}, auth=owner_device)
    check("and the password still signs in", status == 200, (status, again))
    check("the backup was not rewritten", os.path.getmtime(os.path.join(STATE, "auth.db.pre-v6")) == backup_mtime)
    stop(new)
except Exception as e:
    fail += 1
    print(f"  FAIL  the suite crashed: {e}")
    print(open(LOG).read()[-4000:])
finally:
    shutil.rmtree(WORK, ignore_errors=True)

print(f"\n{ok} passed, {fail} failed")
sys.exit(1 if fail else 0)
