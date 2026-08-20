#!/usr/bin/env python3
"""Claims a fresh server once, and prints the credentials for everything else.

**The bootstrap pairing nonce is single-use**, so the suites cannot each pair
themselves — the first one to run would consume it and the rest would fail
authenticating rather than failing at what they test. One operator claims the
server; every suite then uses what that produced. Which is also how a real
deployment works.

Prints two shell assignments on stdout:

    STORM_SESSION=Bearer sta_…
    STORM_DEVICE=StormDevice dev_…:dvs_…

Usage: bootstrap.py <base-url> <server-log>
"""

import os
import shlex
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import storm_auth  # noqa: E402

if len(sys.argv) != 3:
    print(__doc__, file=sys.stderr)
    sys.exit(2)

base, log = sys.argv[1], sys.argv[2]
try:
    session, device, _user = storm_auth.sign_in(base, log_path=log)
except RuntimeError as e:
    print(f"bootstrap failed: {e}", file=sys.stderr)
    sys.exit(1)

# **Shell-quoted.** Both values contain a space (`Bearer sta_…`,
# `StormDevice dev_…:dvs_…`), so an unquoted assignment makes `eval` treat the
# second word as a command — which fails as `command not found` rather than as
# anything resembling an auth problem.
print(f"STORM_SESSION={shlex.quote(session)}")
print(f"STORM_DEVICE={shlex.quote(device)}")
