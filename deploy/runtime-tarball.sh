#!/bin/sh
# The Linux Runtime Host tarball (decision 151 E1): for any distribution
# with systemd, run as yourself with `./storm-runtime install --user`. The
# .deb stays the service install. release.yml and installer-e2e.yml both
# build it here, so CI tests the layout that ships.
#
#   deploy/runtime-tarball.sh BINARY VERSION OUTDIR
set -eu

if [ $# -ne 3 ]; then
  echo "usage: $0 BINARY VERSION OUTDIR" >&2
  exit 2
fi
bin=$1 version=$2 out=$3
root=$(cd "$(dirname "$0")/.." && pwd)
stage=$(mktemp -d "${TMPDIR:-/tmp}/storm-runtime-tarball.XXXXXX")
trap 'rm -rf "$stage"' EXIT

cp "$bin" "$stage/storm-runtime"
chmod 0755 "$stage/storm-runtime"
cp "$root/LICENSE" "$stage/LICENSE"
cat >"$stage/README-linux.txt" <<TXT
Storm Runtime Host v$version for Linux (x86_64), for any distribution with
systemd. On Debian and Ubuntu the storm-runtime .deb runs it as its own
service account instead (deploy/README.md, "Runtime Hosts").

Run it as yourself, with no root:
  ./storm-runtime install --user

That copies it to ~/.local/bin, writes
~/.config/systemd/user/storm-runtime.service and enables it, and turns on
linger so it starts at boot without a login. Where linger needs an
administrator, it prints the one command for them to run; it never runs sudo.

In the Storm app: Settings > Hosts & default agent > Enroll a host, then
paste the string here (it is read from the prompt, never an argument):
  ~/.local/bin/storm-runtime enroll
  systemctl --user start storm-runtime
  ~/.local/bin/storm-runtime check

Workspaces are the directories under ~/.local/state/storm-runtime/workspaces,
or under the roots in ~/.config/storm-runtime/runtime.toml.

Upgrade: run the new binary's install --user again.
Remove: ~/.local/bin/storm-runtime uninstall --user (--purge also removes its
state and config; workspaces stay).
TXT

chmod 0644 "$stage/LICENSE" "$stage/README-linux.txt"
mkdir -p "$out"
tar -C "$stage" --owner=0 --group=0 -czf "$out/storm-runtime-$version-x86_64-linux.tar.gz" \
  storm-runtime LICENSE README-linux.txt
echo "$out/storm-runtime-$version-x86_64-linux.tar.gz"
