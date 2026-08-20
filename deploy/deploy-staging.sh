#!/usr/bin/env bash
#
# Deploy the working tree's server to **staging** on the VM.
#
#   ./deploy/deploy-staging.sh [--no-build] [--no-web]
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
# Ship the web client too, unless asked not to. Without it the server answers
# 404 for everything that is not an API route, and staging silently lacks a
# surface prod has — which is exactly the kind of difference that makes a
# staging pass mean nothing. `--no-web` skips the (slow) Flutter web build when
# only the server changed.
WITH_WEB=1
TARGET=x86_64-unknown-linux-musl


HERE="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HERE/apps/server/target/$TARGET/release/storm-server"

# --- build ------------------------------------------------------------------
#
# `cargo` on a Mac with Homebrew's rust first on PATH is the wrong cargo: that
# toolchain ships no musl std, and the failure blames a missing target that
# rustup has actually installed. Pin both the toolchain and ~/.cargo/bin (where
# cargo-zigbuild lives) rather than trusting PATH order.
BUILD=1
WEB_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    --no-web)   WITH_WEB=0 ;;
    # Ship only the web bundle and restart. The Makefile keeps a `deploy-web`
    # for prod for the same reason: a presentation change touches neither the
    # binary nor the wire format, and a Rust release build is minutes and
    # gigabytes.
    --web-only) BUILD=0; WEB_ONLY=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

if [ "$BUILD" = 1 ]; then
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

if [ "$WITH_WEB" = 1 ]; then
  echo "=== building the web client ==="
  ( cd "$HERE/apps/client" && flutter build web --release )
fi

if [ "$WEB_ONLY" = 0 ]; then
  test -x "$BIN" || {
    echo "no binary at $BIN" >&2
    echo "build one, or use --web-only to ship just the web client" >&2
    exit 1
  }
  echo "=== binary $(du -h "$BIN" | cut -f1) ==="
fi

# --- ship -------------------------------------------------------------------
echo "=== shipping to $HOST:$REMOTE ==="
ssh "$HOST" "mkdir -p '$REMOTE/vaults/scratch' '$REMOTE/state'"
[ "$WEB_ONLY" = 1 ] || scp -q "$BIN" "$HOST:$REMOTE/storm-server.new"
if [ "$WITH_WEB" = 1 ]; then
  rsync -az --delete "$HERE/apps/client/build/web/" "$HOST:$REMOTE/web/"
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

# Only when a new one was shipped; --web-only keeps what is running.
if [ -f storm-server.new ]; then
  mv -f storm-server.new storm-server
fi
chmod +x storm-server

[ -f vaults/scratch/Seed.md ] || printf '# Seed\n\nStaging scratch note.\n' > vaults/scratch/Seed.md

WEB_ARG=""
[ -d '$REMOTE/web' ] && WEB_ARG="--web $REMOTE/web"
setsid ./storm-server serve \
  --vault-root '$REMOTE/vaults' \
  --state '$REMOTE/state' \
  --host 0.0.0.0 --port $PORT \
  \$WEB_ARG \
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
