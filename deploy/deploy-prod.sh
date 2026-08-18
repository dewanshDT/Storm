#!/usr/bin/env bash
#
# Upgrade **prod** on the VM to a published release.
#
#   ./deploy/deploy-prod.sh <version>       # e.g. 0.2.6-1
#   ./deploy/deploy-prod.sh --check         # what is installed / available
#
# **Prod only ever moves through a release.** There is no path in this script
# that copies a locally-built binary, and that is the point rather than an
# omission: a binary built on a laptop has no tag, no changelog, no CI run and
# no way to be reinstalled identically six months from now when something has
# gone wrong. `deploy-staging.sh` is where an unreleased build gets exercised.
#
# It needs sudo on the VM and will prompt for the password — it runs under
# `ssh -t` for exactly that reason. Do not add NOPASSWD to make this quieter;
# a prod upgrade asking a human for a password once is a feature.
set -euo pipefail

HOST="${HOST:-proxmox-mcp-vm}"
STATE="${PROD_STATE:-/srv/storm/state}"

usage() { sed -n '2,12p' "$0"; exit "${1:-1}"; }
[ $# -ge 1 ] || usage

if [ "$1" = "--check" ]; then
  ssh -t "$HOST" "set -e; \
    echo '--- installed ---'; \
    dpkg-query -W -f='\${Package} \${Version}\n' storm-server 2>/dev/null || echo 'not installed via apt'; \
    echo '--- available ---'; \
    sudo apt-get update -qq >/dev/null 2>&1 || true; \
    apt-cache policy storm-server | head -8; \
    echo '--- service ---'; \
    systemctl is-active storm-server"
  exit 0
fi

VERSION="$1"

# A version that is not in the archive fails *after* the backup and *during*
# the upgrade otherwise, which is the worst moment to discover a typo.
echo "=== checking $VERSION is published ==="
ssh -t "$HOST" "sudo apt-get update -qq && apt-cache madison storm-server | grep -q ' $VERSION ' " || {
  echo "storm-server $VERSION is not in the apt archive." >&2
  echo "Publish the release first, then re-run. ./deploy/deploy-prod.sh --check" >&2
  exit 1
}

# --- back up before touching anything ---------------------------------------
#
# `auth.db` and `state/identity/` are the only part of state/ that cannot be
# rebuilt by rescanning markdown. A restore is what makes an upgrade reversible,
# and *a backup that has not been restored is a hope, not a backup* — which is
# why this prints the archive path for a real restore rehearsal rather than
# only asserting the file exists.
echo "=== backing up $STATE ==="
ssh -t "$HOST" "set -e; \
  sudo /usr/bin/storm-backup.sh || { echo 'backup failed — refusing to upgrade' >&2; exit 1; }; \
  echo '--- newest archives ---'; \
  sudo ls -lt /srv/storm/backups 2>/dev/null | head -4 || true; \
  echo '--- state carries the unrebuildable bits? ---'; \
  sudo test -f '$STATE/auth.db' && echo '  auth.db present' || echo '  auth.db ABSENT (pre-auth server)'; \
  sudo test -d '$STATE/identity' && echo '  identity/ present' || echo '  identity/ ABSENT (pre-auth server)'"

# --- upgrade ----------------------------------------------------------------
echo "=== upgrading to $VERSION ==="
ssh -t "$HOST" "set -e; \
  sudo apt-get install -y --allow-downgrades storm-server=$VERSION; \
  sudo systemctl restart storm-server; \
  sleep 2; \
  systemctl is-active --quiet storm-server \
    && echo '  storm-server active' \
    || { echo '  FAILED — journalctl -u storm-server -n 50' >&2; exit 1; }"

# --- verify -----------------------------------------------------------------
#
# `is-active` says the process started, not that it serves. Ask it something.
echo "=== verifying ==="
ADDR="$(ssh "$HOST" hostname -I | awk '{print $1}')"
curl -sf -m 8 "http://$ADDR:8484/v1/health" || {
  echo "  health check failed after upgrade" >&2; exit 1; }
echo
echo "=== prod is $VERSION ==="
echo "    identity: curl -s http://$ADDR:8484/v1/server"
echo "    rollback: ./deploy/deploy-prod.sh <previous-version>"
