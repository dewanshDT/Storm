#!/usr/bin/env python3
"""A stand-in for Claude Code in token_sync_e2e.py.

storm-runtime starts it exactly as it would start `claude`. It writes
`claude-stub.json` in its working directory (the session's workspace):

- its pid and argv;
- the names of every variable in its environment, and the values of the
  credential and routing variables the suite plants (never the synced token:
  `CLAUDE_CODE_OAUTH_TOKEN` is recorded as its sha256 only, so the suite's
  file scan stays meaningful);
- the JSON of the `--settings` file it was given.

Then it idles, so the suite can read its `/proc/<pid>/environ`. A test
fixture, not Storm.
"""
import hashlib
import json
import os
import sys
import time

RECORDED = [
    "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN", "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "DISABLE_TELEMETRY", "NODE_EXTRA_CA_CERTS",
]


def main():
    settings = None
    if "--settings" in sys.argv:
        path = sys.argv[sys.argv.index("--settings") + 1]
        with open(path) as f:
            settings = {"path": path, "json": json.load(f), "mode": oct(os.stat(path).st_mode & 0o777)}
    token = os.environ.get("CLAUDE_CODE_OAUTH_TOKEN")
    record = {
        "pid": os.getpid(),
        "argv": sys.argv[1:],
        "env_names": sorted(os.environ),
        "env": {k: os.environ.get(k) for k in RECORDED},
        "token_sha256": hashlib.sha256(token.encode()).hexdigest() if token is not None else None,
        "settings": settings,
    }
    tmp = "claude-stub.json.tmp"
    with open(tmp, "w") as f:
        json.dump(record, f)
    os.rename(tmp, "claude-stub.json")
    print("claude stub: ready", flush=True)
    while True:
        time.sleep(1)


if __name__ == "__main__":
    main()
