#!/usr/bin/env bash
#
# Deploy the working tree's server to **staging** on the VM.
#
#   ./deploy/deploy-staging.sh [--no-build]
#
# Staging exists so auth work can be exercised against a real server, over a
# real network, without risking the vault. Three properties make that true, and
# each is deliberate:
#
#   * **No sudo, ever.** Staging installs under the login user's home, runs on
#     its own port and is supervised by a pidfile rather than systemd. Nothing
#     it does can require root, so nothing it does can reach a root-owned path.
#   * **Its own state directory**, so its own `auth.db` and `state/identity/`.
#     Prod's state is the one thing in Storm that cannot be rebuilt by
#     rescanning markdown; staging must never open it.
#   * **Its own vault directory**, seeded with a throwaway note. It does not
#     mount the NAS.
#
# Prod is not deployable from here on purpose — see deploy-prod.sh.
set -euo pipefail

HOST="${HOST:-proxmox-mcp-vm}"
PORT="${STAGING_PORT:-8585}"
REMOTE="${STAGING_DIR:-/home/dewansh/storm-staging}"
TARGET=x86_64-unknown-linux-musl

# The staging token is a credential, so it is never a literal in this file and
# never an argument. Precedence: $STAGING_TOKEN from the environment, else the
# one already on the VM, else a fresh random one written mode 600. Nothing here
# is committed and nothing lands in shell history.
TOKEN="${STAGING_TOKEN:-}"

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HERE/apps/server/target/$TARGET/release/storm-server"

# --- build ------------------------------------------------------------------
#
# `cargo` on a Mac with Homebrew's rust first on PATH is the wrong cargo: that
# toolchain ships no musl std, and the failure blames a missing target that
# rustup has actually installed. Pin both the toolchain and ~/.cargo/bin (where
# cargo-zigbuild lives) rather than trusting PATH order.
if [ "${1:-}" != "--no-build" ]; then
  RUSTUP_BIN="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin"
  if [ ! -x "$RUSTUP_BIN/cargo" ]; then
    echo "no rustup toolchain at $RUSTUP_BIN" >&2
    echo "install rustup, then: rustup target add $TARGET" >&2
    exit 1
  fi
  command -v "$HOME/.cargo/bin/cargo-zigbuild" >/dev/null || {
    echo "cargo-zigbuild not found in ~/.cargo/bin" >&2
    echo "  brew install zig && cargo install cargo-zigbuild" >&2
    exit 1
  }
  echo "=== building $TARGET ==="
  ( cd "$HERE/apps/server" \
    && PATH="$HOME/.cargo/bin:$RUSTUP_BIN:$PATH" \
       cargo zigbuild --release --target "$TARGET" )
fi

test -x "$BIN" || { echo "no binary at $BIN" >&2; exit 1; }
echo "=== binary $(du -h "$BIN" | cut -f1) ==="

# --- ship -------------------------------------------------------------------
echo "=== shipping to $HOST:$REMOTE ==="
ssh "$HOST" "mkdir -p '$REMOTE/vaults/scratch' '$REMOTE/state'"
scp -q "$BIN" "$HOST:$REMOTE/storm-server.new"

# Establish the token on the VM without it ever crossing argv. If the caller
# exported one, plant it; otherwise keep what is there, or mint one.
if [ -n "$TOKEN" ]; then
  printf 'STORM_TOKEN=%s\n' "$TOKEN" \
    | ssh "$HOST" "cat > '$REMOTE/staging.env' && chmod 600 '$REMOTE/staging.env'"
else
  ssh "$HOST" "set -e; \
    if [ ! -s '$REMOTE/staging.env' ]; then \
      umask 077; \
      printf 'STORM_TOKEN=%s\n' \"\$(head -c 24 /dev/urandom | base64 | tr -d '/+=' )\" \
        > '$REMOTE/staging.env'; \
      echo '  minted a new staging token'; \
    fi"
fi

# --- restart ----------------------------------------------------------------
#
# Stop by pidfile, not by pattern: `pkill -f storm-server` on this box would
# also match the *prod* process, which is the one thing this script must never
# touch.
ssh "$HOST" "bash -se" <<REMOTE_SCRIPT
set -euo pipefail
cd '$REMOTE'

if [ -f staging.pid ] && kill -0 "\$(cat staging.pid)" 2>/dev/null; then
  echo "  stopping staging pid \$(cat staging.pid)"
  kill "\$(cat staging.pid)"
  for _ in \$(seq 1 20); do
    kill -0 "\$(cat staging.pid)" 2>/dev/null || break
    sleep 0.5
  done
fi
rm -f staging.pid

mv -f storm-server.new storm-server
chmod +x storm-server

[ -f vaults/scratch/Seed.md ] || printf '# Seed\n\nStaging scratch note.\n' > vaults/scratch/Seed.md

# STORM_TOKEN comes from the mode-600 env file, never argv: arguments are
# world-readable through /proc on a shared box.
set -a
. '$REMOTE/staging.env'
set +a
setsid ./storm-server serve \
  --vault-root '$REMOTE/vaults' \
  --state '$REMOTE/state' \
  --host 0.0.0.0 --port $PORT \
  > '$REMOTE/staging.log' 2>&1 &
echo \$! > staging.pid
disown || true

for i in \$(seq 1 40); do
  if curl -sf -o /dev/null "http://127.0.0.1:$PORT/v1/health"; then
    echo "  staging healthy on :$PORT (pid \$(cat staging.pid))"
    exit 0
  fi
  sleep 0.5
done
echo "  staging did not come up; last log lines:" >&2
tail -20 '$REMOTE/staging.log' >&2
exit 1
REMOTE_SCRIPT

# --- prove it is the build we just shipped ----------------------------------
#
# A restart that silently kept the old binary looks identical to a successful
# deploy until a test fails for the wrong reason.
echo "=== verifying ==="
curl -sf -m 8 "http://$(ssh "$HOST" hostname -I | awk '{print $1}'):$PORT/v1/server" \
  | head -c 200 || {
    echo "  /v1/server did not answer JSON — is this build pre-auth?" >&2
    exit 1
  }
echo
echo "=== staging deployed: port $PORT, state $REMOTE/state ==="
echo "    log:  ssh $HOST tail -f $REMOTE/staging.log"
echo "    stop: ssh $HOST 'kill \$(cat $REMOTE/staging.pid)'"
