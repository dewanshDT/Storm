#!/usr/bin/env python3
"""The bridge's three restart rules, proved by breaking them (spec §17, §19).

For each of the gates' mutations — `leak_init`, `retry` and `no_cancel` — this
rebuilds `storm-runtime` with the bridge broken that way, runs
`gateway_e2e.py` against it, and requires the matching check to FAIL. Then it
restores the source and rebuilds. A mutation the suite does not catch is a
check that proves nothing.

Not in `make test-live`: three rebuilds and three suite runs take minutes.
Run it with `make test-gateway-mutations` when the bridge or the suite
changes (decision 81i).

The source is always restored, even on failure or Ctrl-C.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER = os.path.dirname(HERE)
RUNTIME = os.path.join(os.path.dirname(SERVER), "runtime")
BRIDGE = os.path.join(RUNTIME, "src", "bridge.rs")

MUTATIONS = [
    (
        "leak_init: the replayed initialize result reaches the agent",
        "                ok = true; // swallowed: the agent already has its one result",
        "                ok = true;\n                self.emit(m.clone());",
        "FAIL  the agent saw exactly one initialize result (R5)",
    ),
    (
        "retry: a failed call is retried until the server is back",
        "        if !answered {\n"
        "            // The stream ended without a response: one error, never a retry.\n"
        "            self.emit(error_for(&id, \"no_response\"));\n"
        "        }",
        "        if !answered {\n"
        "            for _ in 0..30 {\n"
        "                tokio::time::sleep(std::time::Duration::from_secs(1)).await;\n"
        "                let mut rx = self.daemon.exchange(req.clone()).await;\n"
        "                let mut good = false;\n"
        "                let mut unknown = false;\n"
        "                while let Some(l) = rx.recv().await {\n"
        "                    if l.get(\"message\").is_some() { good = true; }\n"
        "                    if l.get(\"storm_error\") == Some(&json!(\"session_unknown\")) { unknown = true; }\n"
        "                }\n"
        "                if unknown && self.reinitialize().await {\n"
        "                    let mut rx = self.daemon.exchange(req.clone()).await;\n"
        "                    while let Some(l) = rx.recv().await { if l.get(\"message\").is_some() { good = true; } }\n"
        "                }\n"
        "                if good { break; }\n"
        "            }\n"
        "            self.emit(error_for(&id, \"no_response\"));\n"
        "        }",
        "FAIL  the upstream executed it once, never retried (R7)",
    ),
    (
        "no_cancel: an open elicitation is not cancelled when its call fails",
        "        for up in stale {",
        "        for up in stale.into_iter().take(0) {",
        "FAIL  the open elicitation is cancelled toward the agent (R8)",
    ),
]


def build():
    r = subprocess.run(["cargo", "build", "-q"], cwd=RUNTIME, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-3000:])
    return r.returncode == 0


def main():
    original = open(BRIDGE).read()
    caught = 0
    try:
        for name, old, new, expect in MUTATIONS:
            if original.count(old) != 1:
                print(f"  FAIL  {name}: the mutation no longer applies — update this file")
                continue
            open(BRIDGE, "w").write(original.replace(old, new))
            if not build():
                print(f"  FAIL  {name}: the mutated bridge does not build")
                continue
            r = subprocess.run(
                [sys.executable, os.path.join(HERE, "gateway_e2e.py")],
                cwd=SERVER, capture_output=True, text=True, timeout=1200,
                env={**os.environ,
                     "SERVER_BIN": os.environ.get("SERVER_BIN", os.path.join(SERVER, "target/debug/storm-server")),
                     "RUNTIME_BIN": os.path.join(RUNTIME, "target/debug/storm-runtime")},
            )
            if expect in r.stdout:
                caught += 1
                print(f"  PASS  {name} — caught by: {expect.removeprefix('FAIL  ')}")
            else:
                print(f"  FAIL  {name} — NOT caught")
                print("\n".join(l for l in r.stdout.splitlines() if "FAIL" in l))
    finally:
        open(BRIDGE, "w").write(original)
        build()
    print(f"\n  {caught} of {len(MUTATIONS)} mutations caught")
    sys.exit(0 if caught == len(MUTATIONS) else 1)


if __name__ == "__main__":
    main()
