#!/usr/bin/env python3
"""Layer 2 of decision 119 against a real server: an untrusted network.

Starts its own server with loopback marked untrusted (the hidden
`--test-untrusted-interface`, standing in for a firewalld `public` zone or a
Windows Public network), claims it, and checks from the outside that it does
not answer an mDNS query on loopback and refuses password pairing from it with
403 `lan_only`, while everything else still works. `lan_discovery_e2e.py` is
the control: the same query against a trusted loopback finds the server.

Env: SERVER_BIN. Ports: the first free one in 19800–19899.
"""

import os
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault("STORM_BASE", "http://127.0.0.1:0")
os.environ.setdefault("STORM_SESSION", "")
import lan_discovery_e2e  # noqa: E402
import storm_auth  # noqa: E402

SERVER_BIN = os.environ["SERVER_BIN"]
failures = []


def check(name, ok, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {name}" + ("" if ok else f"   {detail}"))
    if not ok:
        failures.append(name)


def free_port():
    for port in range(19899, 19799, -1):
        s = socket.socket()
        try:
            s.bind(("127.0.0.1", port))
            return port
        except OSError:
            continue
        finally:
            s.close()
    raise RuntimeError("no free port in 19800-19899")


def main():
    # This server is claimed here, not with the harness's session.
    os.environ.pop("STORM_SESSION", None)
    os.environ.pop("STORM_DEVICE", None)
    work = tempfile.mkdtemp(prefix="storm-untrusted-e2e-")
    os.makedirs(os.path.join(work, "vaults", "primary"))
    log_path = os.path.join(work, "server.log")
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    with open(log_path, "w") as log:
        server = subprocess.Popen(
            [
                SERVER_BIN, "serve",
                "--vault-root", os.path.join(work, "vaults"),
                "--state", os.path.join(work, "state"),
                "--host", "127.0.0.1", "--port", str(port),
                "--test-untrusted-interface", "lo",
                "--test-untrusted-interface", "lo0",
            ],
            stdout=log, stderr=subprocess.STDOUT,
        )
    try:
        for _ in range(120):
            try:
                if storm_auth._call(base, "GET", "/v1/health")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.25)
        session, _, _ = storm_auth.sign_in(base, log_path)
        status, info = storm_auth._call(base, "GET", "/v1/server")
        sid = info["server_id"]

        time.sleep(1.0)
        status, config = storm_auth._call(base, "GET", "/v1/config", auth=session)
        untrusted = (config.get("discovery") or {}).get("untrusted", [])
        check("GET /v1/config names the untrusted interface", any(n in ("lo", "lo0") for n in untrusted), f"{config.get('discovery')}")
        check("…while discovery itself stays on", (config.get("discovery") or {}).get("active") is True, f"{config.get('discovery')}")

        found = lan_discovery_e2e.browse(3.0)
        check("an untrusted interface is not advertised", sid not in found, f"saw {list(found)}")

        status, body = storm_auth._call(
            base,
            "POST",
            "/v1/pairings/password",
            {"password": storm_auth.PASSWORD, "device_name": "e2e untrusted"},
        )
        check(
            "password pairing from it is 403 lan_only",
            status == 403 and body.get("error") == "lan_only",
            f"{status} {body}",
        )
        status, devices = storm_auth._call(base, "GET", "/v1/auth/devices", auth=session)
        check("…and minted no device", not any(d.get("name") == "e2e untrusted" for d in devices), f"{devices}")
        status, _ = storm_auth._call(base, "GET", "/v1/vaults", auth=session)
        check("a paired device is still served", status == 200, f"{status}")

        with open(log_path, encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        check("the server logs why", "untrusted network" in text, "no 'untrusted network' line")
    finally:
        server.terminate()
        server.wait(timeout=10)

    print(f"\n{'FAILED' if failures else 'OK'}: {len(failures)} failure(s)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
