#!/usr/bin/env bash
# Nightly backup of the vault and the index.
#
# Both matter, for different reasons. The vault is the notes. The index holds
# version history, which the 3-way merge uses as its base and which cannot be
# rebuilt from the markdown — everything else in state/ can.
#
# Run by storm-backup.timer; safe to run by hand.

set -euo pipefail

ENV_FILE="${STORM_ENV_FILE:-/etc/storm/storm.env}"
[ -r "$ENV_FILE" ] || { echo "cannot read $ENV_FILE" >&2; exit 1; }
# shellcheck disable=SC1090
set -a; . "$ENV_FILE"; set +a

: "${STORM_VAULT_ROOT:?}" "${STORM_STATE:?}" "${STORM_BACKUP_DEST:?}"
KEEP_DAYS="${STORM_BACKUP_KEEP_DAYS:-30}"
SERVER_BIN="${STORM_SERVER_BIN:-/usr/local/bin/storm-server}"

stamp="$(date -u +%Y-%m-%d)"
dest="$STORM_BACKUP_DEST/$stamp"
mkdir -p "$dest"

echo "storm-backup: $stamp -> $dest"

# The vault is ordinary files; rsync is exactly right for it. --delete keeps
# the mirror honest about deletions, and the dated directory means an
# accidental wipe is still recoverable from yesterday's copy.
rsync -a --delete "$STORM_VAULT_ROOT/" "$dest/vaults/"
echo "  vaults: $(find "$dest/vaults" -type f | wc -l | tr -d ' ') files across \
$(find "$dest/vaults" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ') vault(s)"

# The indexes must NOT be rsynced. They run in WAL mode with the server
# holding them open, so a file copy can catch committed pages still sitting in
# the -wal and produce a database that opens but is missing history. The server
# writes proper snapshots instead, which is safe against a live database.
#
# One per vault, plus vaults.json — without the registry the snapshots are a
# pile of UUID-named files nobody can match to a vault.
"$SERVER_BIN" --state "$STORM_STATE" --backup-db "$dest/index" >/dev/null
echo "  index:  $(du -sh "$dest/index" | cut -f1)"

# Prove the snapshot opens before trusting it. A backup nobody has read is a
# guess, not a backup.
#
# Verify into a temp file, never /dev/null: the snapshot path is deleted first
# if it exists, and pointing that at a device node would remove it.
verify_tmp="$(mktemp -d "${TMPDIR:-/tmp}/storm-verify-XXXXXX")"
if ! "$SERVER_BIN" --state "$dest/index" --backup-db "$verify_tmp" >/dev/null 2>&1; then
    rm -rf "$verify_tmp"
    echo "  ERROR: the snapshots did not open — keeping them, but investigate" >&2
    exit 1
fi
rm -rf "$verify_tmp"
echo "  verified: snapshots open"

find "$STORM_BACKUP_DEST" -mindepth 1 -maxdepth 1 -type d -mtime "+$KEEP_DAYS" \
    -print -exec rm -rf {} + | sed 's/^/  pruned: /'

echo "storm-backup: done"
