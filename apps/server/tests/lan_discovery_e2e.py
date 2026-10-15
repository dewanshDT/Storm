#!/usr/bin/env python3
"""LAN discovery pairing against a real, claimed server (decision 118).

Asks for `_storm._tcp` with a real mDNS query on loopback (the server under
`make test-live` binds 127.0.0.1, so loopback is where it advertises), checks
the TXT, pairs with the password over the direct address, signs in with what
that returned, and follows the "Discoverable on this network" switch both
ways. The relayed refusal is `relay::tests`' job: it needs a trunk.

Env: STORM_BASE (http://127.0.0.1:<port>), STORM_SESSION.
"""

import os
import socket
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import storm_auth  # noqa: E402

BASE = os.environ["STORM_BASE"]
SESSION = os.environ["STORM_SESSION"]
PORT = int(BASE.rsplit(":", 1)[1])
SERVICE = "_storm._tcp.local"

failures = []


def check(name, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {name}{'' if ok else f' — {detail}'}")
    if not ok:
        failures.append(name)


def _name(data, at):
    labels, jumped, end = [], False, at
    while True:
        n = data[at]
        if n & 0xC0 == 0xC0:
            if not jumped:
                end = at + 2
            at, jumped = ((n & 0x3F) << 8) | data[at + 1], True
            continue
        if n == 0:
            return ".".join(labels), (end if jumped else at + 1)
        labels.append(data[at + 1 : at + 1 + n].decode("utf-8", "replace"))
        at += 1 + n


def _records(data):
    _, _, qd, an, ns, ar = struct.unpack("!6H", data[:12])
    at = 12
    for _ in range(qd):
        _, at = _name(data, at)
        at += 4
    out = []
    for _ in range(an + ns + ar):
        name, at = _name(data, at)
        rtype, _, _, length = struct.unpack("!HHIH", data[at : at + 10])
        at += 10
        out.append((name, rtype, data[at : at + length], data, at))
        at += length
    return out


def browse(seconds=3.0):
    """Every advertised instance answering on loopback: {sid: (txt, port)}."""
    q = struct.pack("!6H", 0x5354, 0, 1, 0, 0, 0)
    for label in SERVICE.split("."):
        q += bytes([len(label)]) + label.encode()
    q += b"\x00" + struct.pack("!HH", 12, 1)  # PTR, IN

    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton("127.0.0.1"))
    s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
    s.bind(("127.0.0.1", 0))
    s.settimeout(0.3)
    found = {}
    until = time.time() + seconds
    next_query = 0.0
    while time.time() < until:
        if time.time() >= next_query:
            s.sendto(q, ("224.0.0.251", 5353))
            next_query = time.time() + 1.0
        try:
            data, _ = s.recvfrom(9000)
        except socket.timeout:
            continue
        txt, port = {}, None
        for _, rtype, rdata, whole, at in _records(data):
            if rtype == 16:  # TXT
                i = 0
                while i < len(rdata):
                    n = rdata[i]
                    k, _, v = rdata[i + 1 : i + 1 + n].decode().partition("=")
                    txt[k] = v
                    i += 1 + n
            elif rtype == 33:  # SRV
                port = struct.unpack("!H", rdata[4:6])[0]
        if "sid" in txt:
            found[txt["sid"]] = (txt, port)
    s.close()
    return found


def main():
    status, info = storm_auth._call(BASE, "GET", "/v1/server")
    check("GET /v1/server", status == 200, f"{status}")
    sid, pk = info["server_id"], info["public_key"]

    found = browse()
    entry = found.get(sid)
    check("the server answers an mDNS query on loopback", entry is not None, f"saw {list(found)}")
    if entry:
        txt, port = entry
        check("SRV carries the HTTP port", port == PORT, f"{port} != {PORT}")
        check(
            "TXT is v, sid, pk, ver, claimed, lan",
            set(txt) >= {"v", "sid", "pk", "ver", "claimed", "lan"},
            f"{txt}",
        )
        check("v=1", txt.get("v") == "1", f"{txt}")
        check("pk is the server's key", txt.get("pk") == pk, f"{txt.get('pk')} != {pk}")
        check("claimed=1 on a set-up server", txt.get("claimed") == "1", f"{txt}")
        check("lan=1 while discoverable", txt.get("lan") == "1", f"{txt}")
        check("no secret in the TXT", "password" not in str(txt).lower())

    status, body = storm_auth._call(
        BASE,
        "POST",
        "/v1/pairings/password",
        {"password": "not the password", "device_name": "e2e lan"},
    )
    check(
        "a wrong password is 401 invalid_credentials",
        status == 401 and body.get("error") == "invalid_credentials",
        f"{status} {body}",
    )

    status, paired = storm_auth._call(
        BASE,
        "POST",
        "/v1/pairings/password",
        {
            "password": storm_auth.PASSWORD,
            "device_name": "e2e lan",
            "platform": "linux",
            "version": "e2e",
        },
    )
    check("password pairing from loopback returns a device", status == 200, f"{status} {paired}")
    if status == 200:
        check("…pinned to this server", paired.get("server_id") == sid and paired.get("public_key") == pk)
        device = f"StormDevice {paired['device_id']}:{paired['device_secret']}"
        status, session = storm_auth._call(
            BASE, "POST", "/v1/auth/login", {"password": storm_auth.PASSWORD}, auth=device
        )
        check("…whose credential signs in", status == 200, f"{status} {session}")
        status, devices = storm_auth._call(BASE, "GET", "/v1/auth/devices", auth=SESSION)
        mine = [d for d in devices if d.get("id") == paired["device_id"]]
        check(
            "…listed as paired on the local network",
            bool(mine) and mine[0].get("paired_by") == "lan_password",
            f"{mine}",
        )

    status, view = storm_auth._call(
        BASE, "PUT", "/v1/config/discovery", {"enabled": False}, auth=SESSION
    )
    check("discovery switches off", status == 200 and view.get("active") is False, f"{status} {view}")
    status, body = storm_auth._call(
        BASE,
        "POST",
        "/v1/pairings/password",
        {"password": storm_auth.PASSWORD, "device_name": "e2e lan off"},
    )
    check("…and password pairing is 403 lan_only", status == 403 and body.get("error") == "lan_only", f"{status} {body}")
    time.sleep(1.5)
    check("…and the advertisement is gone", sid not in browse(2.0))

    status, view = storm_auth._call(
        BASE, "PUT", "/v1/config/discovery", {"enabled": True}, auth=SESSION
    )
    check("discovery switches back on", status == 200 and view.get("active") is True, f"{status} {view}")
    check("…and it advertises again", sid in browse(4.0))

    print(f"\n{'FAILED' if failures else 'OK'}: {len(failures)} failure(s)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
