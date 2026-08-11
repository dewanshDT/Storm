#!/usr/bin/env bash
# One-shot cutover from run.sh to systemd on the homelab VM.
# Needs sudo. Run from a machine that can scp the musl binary, or on the VM
# after placing storm-server next to this script.
#
#   ./deploy/cutover-vm.sh /path/to/storm-server
set -euo pipefail

BIN="${1:?usage: $0 /path/to/storm-server}"
test -x "$BIN"
HERE="$(cd "$(dirname "$0")" && pwd)"

echo "=== installing binary and units ==="
sudo install -m755 "$BIN" /usr/bin/storm-server
sudo mkdir -p /lib/systemd/system /etc/storm /usr/bin
sudo cp "$HERE/storm-server.service" /lib/systemd/system/storm-server.service
if [ -f "$HERE/storm-backup.service" ]; then
  sudo cp "$HERE/storm-backup.service" "$HERE/storm-backup.timer" /lib/systemd/system/
fi
if [ -f "$HERE/storm-backup.sh" ]; then
  sudo install -m755 "$HERE/storm-backup.sh" /usr/bin/storm-backup.sh
fi

echo "=== stopping hand-started server ==="
# Match the binary path, not this shell's command line.
pkill -f '/home/dewansh/storm/storm-server' 2>/dev/null || true
sleep 1

echo "=== storm-server up (NAS vaults + local state) ==="
sudo /usr/bin/storm-server up \
  --data-root /home/dewansh/storm \
  --vault-root /mnt/media/Docs/storm \
  --state /home/dewansh/storm/state \
  --web /home/dewansh/storm/web

echo "=== status ==="
/usr/bin/storm-server status
echo
echo "Token is in /etc/storm/storm.env — update phone and browser."
echo "Retire /home/dewansh/storm/run.sh once this looks good across a reboot."
