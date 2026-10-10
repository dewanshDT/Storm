#!/usr/bin/env python3
"""Drive a command on a real pseudo-terminal, typing keys into it.

    pty_drive.py KEY... -- CMD [ARG...]

Each KEY is typed after a short pause; escapes like \\r and \\x1b[B work.
Prints everything the command drew (ANSI sequences stripped), then
"EXIT=<status>". The installer's raw-key TUI only runs on a terminal, which
is why this exists: the bats suites otherwise drive its numbered line mode.
"""
import os
import pty
import re
import select
import sys
import time

argv = sys.argv[1:]
split = argv.index("--")
keys, cmd = argv[:split], argv[split + 1:]

pid, fd = pty.fork()
if pid == 0:
    os.execvp(cmd[0], cmd)

out = bytearray()


def pump(seconds):
    """Read what the child draws for up to SECONDS; False once it's gone."""
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if not ready:
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            return False
        if not data:
            return False
        out.extend(data)
    return True


pump(1.5)
for key in keys:
    os.write(fd, key.encode().decode("unicode_escape").encode("latin-1"))
    if not pump(float(os.environ.get("PTY_KEY_PAUSE", "0.4"))):
        break
deadline = time.time() + float(os.environ.get("PTY_TIMEOUT", "30"))
while time.time() < deadline and pump(0.5):
    pass

status = None
for _ in range(50):
    done, raw = os.waitpid(pid, os.WNOHANG)
    if done:
        status = os.waitstatus_to_exitcode(raw)
        break
    time.sleep(0.1)
if status is None:
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    status = "timeout"

text = re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]", b"", bytes(out)).replace(b"\r", b"")
sys.stdout.write(text.decode("utf-8", "replace"))
print("\nEXIT=%s" % status)
