#!/usr/bin/env bash
# Checks for deploy/install.sh: shellcheck, then the bats suites.
#
#   tools/installer-tests/run.sh            # everything
#   tools/installer-tests/run.sh plan.bats  # one suite (shellcheck still runs)
#
# Needs shellcheck and bats-core on PATH, or SHELLCHECK=/path BATS=/path.
# The installer's body is bash inside a heredoc (so dash can read the file),
# so shellcheck runs on the extracted body too; its line N is install.sh's
# line N + the offset printed below.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
installer="$root/deploy/install.sh"
SHELLCHECK=${SHELLCHECK:-shellcheck}
BATS=${BATS:-bats}

for tool in "$SHELLCHECK" "$BATS"; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "run.sh: '$tool' not found. Install shellcheck and bats-core, or set SHELLCHECK= / BATS=." >&2
    exit 2
  fi
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/installer-check.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

start=$(grep -n "^  cat <<'__STORM_INSTALLER_BODY__'\$" "$installer" | cut -d: -f1)
if [ -z "$start" ]; then
  echo "run.sh: can't find the body heredoc in $installer" >&2
  exit 1
fi
sed -n "/^  cat <<'__STORM_INSTALLER_BODY__'\$/,/^__STORM_INSTALLER_BODY__\$/p" "$installer" |
  sed '1d;$d' >"$tmp/install-body.bash"

echo "== shellcheck (the body's line N is install.sh line N+$start)"
"$SHELLCHECK" -s sh "$installer"
"$SHELLCHECK" -s bash "$installer"
"$SHELLCHECK" -s bash "$tmp/install-body.bash"
"$SHELLCHECK" -s bash "$here/helpers.bash" "$here/fake/fake.sh" "$here/run.sh"
bash -n "$tmp/install-body.bash"
echo "shellcheck: clean"

echo "== bats"
if [ $# -gt 0 ]; then
  (cd "$here" && "$BATS" "$@")
else
  "$BATS" "$here"
fi
