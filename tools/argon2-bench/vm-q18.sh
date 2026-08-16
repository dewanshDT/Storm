#!/usr/bin/env bash
# Q18 on the homelab VM: measure Argon2id, and optionally stand up a staging
# storm-server to check auth slice 1 on the real box.
#
# PROD SAFETY, which is the whole point:
#   * never runs `storm-server up` / `down` or any systemctl verb — those write
#     /etc/storm/storm.env and the unit files, which IS prod;
#   * staging binds port 8585, not 8484;
#   * staging keeps its own vault root and state under /tmp/storm-staging, and
#     never touches /srv/storm/state or the NAS at /mnt/media/Docs/storm;
#   * everything is torn down at the end, and prod is re-checked afterwards.
#
# Needs to be run from a machine on the homelab LAN — the server is LAN-only
# (PLAN.md decision 4), so this fails fast and clearly when it is not.
#
#   ./vm-q18.sh                  # benchmark only: copies one static binary
#   ./vm-q18.sh --with-staging   # also deploy a throwaway server, needs
#                                # SERVER_BIN=... from `make build-server`
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST="${HOST:-proxmox-mcp-vm}"
TARGET="${CARGO_TARGET_DIR:-$HERE/target}"
BENCH="$TARGET/x86_64-unknown-linux-musl/release/argon2-bench"
SERVER="${SERVER_BIN:-}"
REMOTE=/tmp/storm-q18
STAGING=/tmp/storm-staging
PORT="${PORT:-8585}"
RESULTS="$HERE/q18-vm-results.txt"
WITH_STAGING=0
[ "${1:-}" = "--with-staging" ] && WITH_STAGING=1

say() { printf '\n=== %s ===\n' "$1"; }

# Teardown runs from a trap, not from the end of the script: anything that
# leaves state on someone's server has to clean up on *every* exit path,
# including the ones that were not planned. The prod re-check lives here too,
# so a failed run still tells you whether prod is fine.
#
# It kills by **pidfile, never `pkill -f`**. `pkill -f 'storm-server-staging
# serve'` matches on the whole command line — and the command line of the very
# ssh shell running the pkill contains that string, so it kills its own parent
# and the `rm` after it never runs. The symptom is a teardown that prints its
# heading, reports nothing, and quietly leaves a binary in /tmp on the server.
cleaned=0
teardown() {
    [ "$cleaned" = 1 ] && return
    cleaned=1
    say "teardown — nothing of ours is left running"
    ssh "$HOST" "if [ -f $STAGING/server.pid ]; then
        kill \$(cat $STAGING/server.pid) 2>/dev/null || true
        sleep 1
      fi
      rm -rf $REMOTE $STAGING
      echo 'leftover dirs:' \$(ls -d $REMOTE $STAGING 2>/dev/null | wc -l)" || true

    say "prod, after all of that"
    ssh "$HOST" "systemctl is-active storm-server
      curl -sf -o /dev/null -w 'prod health: HTTP %{http_code}\n' http://127.0.0.1:8484/v1/health
      pgrep -af 'storm-server serve' | head -1" || true
}

if [ ! -x "$BENCH" ]; then
    echo "no Linux benchmark binary at $BENCH" >&2
    echo "build it first:  ./build.sh" >&2
    exit 1
fi

if ! ssh -o ConnectTimeout=10 -o BatchMode=yes "$HOST" true 2>/dev/null; then
    echo "cannot reach $HOST over ssh." >&2
    echo "The Storm server is LAN-only, so this has to run from the homelab" >&2
    echo "network — check you are on the right Wi-Fi, not a guest or hotspot." >&2
    exit 1
fi

# Armed only now: before this point nothing has been copied to the VM, and a
# teardown that ran on a failed connectivity check would just be noise.
trap teardown EXIT

say "preflight: prod must be healthy before we touch anything"
ssh "$HOST" "set -e
  systemctl is-active storm-server
  pgrep -af 'storm-server serve' | head -1
  curl -sf -o /dev/null -w 'prod health: HTTP %{http_code}\n' http://127.0.0.1:8484/v1/health
  free -m | awk 'NR==2{print \"mem: \"\$3\" used / \"\$2\" MB, \"\$7\" available\"}'
  nproc | sed 's/^/vcpus: /'
"

say "shipping the benchmark (one static binary, installs nothing)"
ssh "$HOST" "mkdir -p $REMOTE"
scp -q "$BENCH" "$HOST:$REMOTE/argon2-bench"
ssh "$HOST" "chmod +x $REMOTE/argon2-bench"

say "measuring Argon2id — this is Q18"
# `nice` so a memory-hard sweep never outranks the server actually serving notes.
ssh "$HOST" "nice -n 10 $REMOTE/argon2-bench 5" 2>&1 | tee "$RESULTS"

if [ "$WITH_STAGING" = 1 ]; then
  if [ -z "$SERVER" ] || [ ! -f "$SERVER" ]; then
    echo "no Linux storm-server given — skipping the staging deployment." >&2
    echo "Build one with \`make build-server\` and pass SERVER_BIN=<path>." >&2
  else
    say "staging storm-server on :$PORT, own dirs, prod untouched"
    ssh "$HOST" "rm -rf $STAGING && mkdir -p $STAGING/vaults/staging $STAGING/state"
    scp -q "$SERVER" "$HOST:$REMOTE/storm-server-staging"
    ssh "$HOST" "chmod +x $REMOTE/storm-server-staging
      printf '# Staging\n\nnot prod.\n' > $STAGING/vaults/staging/Seed.md
      nohup $REMOTE/storm-server-staging serve \
        --vault-root $STAGING/vaults --state $STAGING/state \
        --token stagingtoken --port $PORT > $STAGING/server.log 2>&1 &
      echo \$! > $STAGING/server.pid
      for i in \$(seq 1 40); do
        curl -sf -o /dev/null http://127.0.0.1:$PORT/v1/health && break
        sleep 0.5
      done"

    say "auth slice 1 on the real box"
    ssh "$HOST" "set -e
      echo '--- GET /v1/server, no Authorization header:'
      curl -sf http://127.0.0.1:$PORT/v1/server; echo
      echo '--- key file permissions:'
      ls -l $STAGING/state/identity/
      echo '--- auth.db:'
      ls -l $STAGING/state/auth.db
      echo '--- challenge:'
      curl -sf -X POST -H 'content-type: application/json' \
        -d '{\"nonce\":\"0123456789abcdef0123456789abcdef\"}' \
        http://127.0.0.1:$PORT/v1/server/challenge; echo
      echo '--- an ordinary route still needs the token:'
      curl -s -o /dev/null -w 'no token: HTTP %{http_code}\n' http://127.0.0.1:$PORT/v1/vaults
      curl -s -o /dev/null -w 'with token: HTTP %{http_code}\n' \
        -H 'Authorization: Bearer stagingtoken' http://127.0.0.1:$PORT/v1/vaults"

    say "backup -> wipe -> restore, on the VM's own filesystem"
    ssh "$HOST" "set -e
      BEFORE=\$(curl -sf http://127.0.0.1:$PORT/v1/server)
      $REMOTE/storm-server-staging backup-db --state $STAGING/state $STAGING/backup
      kill \$(cat $STAGING/server.pid) 2>/dev/null || true
      sleep 1
      rm -rf $STAGING/state && mkdir -p $STAGING/state
      cp -a $STAGING/backup/. $STAGING/state/
      nohup $REMOTE/storm-server-staging serve \
        --vault-root $STAGING/vaults --state $STAGING/state \
        --token stagingtoken --port $PORT > $STAGING/server2.log 2>&1 &
      echo \$! > $STAGING/server.pid
      for i in \$(seq 1 40); do
        curl -sf -o /dev/null http://127.0.0.1:$PORT/v1/health && break
        sleep 0.5
      done
      AFTER=\$(curl -sf http://127.0.0.1:$PORT/v1/server)
      echo \"before: \$BEFORE\"
      echo \"after:  \$AFTER\"
      [ \"\$BEFORE\" = \"\$AFTER\" ] && echo 'PASS identity survived a restore on the VM' \
        || { echo 'FAIL identity changed'; exit 1; }
      ls -l $STAGING/state/identity/"
  fi
fi

teardown

echo
echo "Q18 numbers saved to $RESULTS"
echo "Write the chosen parameters into the vault note Storm Auth Protocol (A1)."
