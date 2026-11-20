#!/bin/sh
# 163-7: proves deploy/apt-repo-build.sh keeps the apt repo cumulative and
# correct across a run of each kind, on tiny dummy .debs (no real packaging).
# Needs dpkg-deb and reprepro (both on the ubuntu-latest runner; apt-repo.yml
# already installs reprepro).
set -eu

cd "$(dirname "$0")/.."
real_script="$(pwd)/deploy/apt-repo-build.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

export GNUPGHOME="$work/gnupg"
mkdir -m 700 "$GNUPGHOME"
gpg --batch --pinentry-mode loopback --passphrase '' \
	--quick-generate-key 'apt-repo-build-test <test@example.com>' default default never >/dev/null 2>&1
KEYID=$(gpg --list-secret-keys --with-colons | awk -F: '/^sec:/ {print $5; exit}')
export STORM_APT_KEYID="$KEYID"

# A minimal installable .deb, just enough for dpkg-deb -f and reprepro to
# accept it — the content is never installed anywhere.
make_deb() {
	version="$1"
	out="$2"
	pkg_dir="$work/pkg-$version"
	rm -rf "$pkg_dir"
	mkdir -p "$pkg_dir/DEBIAN"
	cat >"$pkg_dir/DEBIAN/control" <<EOF
Package: storm-server
Version: $version
Architecture: amd64
Maintainer: Storm <test@example.com>
Section: utils
Priority: optional
Description: fixture for apt-repo-build-test
EOF
	dpkg-deb --build --root-owner-group "$pkg_dir" "$out" >/dev/null
}

repo_dir="$work/apt-repo"
install_src="$work/install-src"
mkdir -p "$install_src"
echo "echo stable install.sh fixture" >"$install_src/install.sh"
echo "echo stable install.ps1 fixture" >"$install_src/install.ps1"
# apt-repo-build.sh copies install.sh/install.ps1 from its own directory —
# stand in for deploy/ with a scratch copy of the real script plus fixtures.
cp "$real_script" "$install_src/apt-repo-build.sh"
script="$install_src/apt-repo-build.sh"

fail=0
check() {
	desc="$1"
	shift
	if "$@"; then
		echo "ok: $desc"
	else
		echo "FAIL: $desc" >&2
		fail=1
	fi
}
check_not() {
	desc="$1"
	shift
	if "$@"; then
		echo "FAIL: $desc" >&2
		fail=1
	else
		echo "ok: $desc"
	fi
}

# Called only indirectly, as check()/check_not()'s "$@" — shellcheck cannot
# see that and flags both as unused (SC2329).
# shellcheck disable=SC2329
version_in() {
	# version_in <suite> <version> — true if that version is published there.
	reprepro -b "$repo_dir" list "$1" storm-server 2>/dev/null | grep -q " $2\$"
}
# shellcheck disable=SC2329
installer_is() {
	grep -q "$1" "$repo_dir/install.sh"
}
# shellcheck disable=SC2329
pool_has() {
	find "$repo_dir/pool" -name "storm-server_$1*" 2>/dev/null | grep -q .
}

# 1. A stable release: v0.18.0.
deb_dir="$work/debs-1"
mkdir -p "$deb_dir"
make_deb "0.18.0-1" "$deb_dir/storm-server_0.18.0-1_amd64.deb"
"$script" stable "$deb_dir" "$repo_dir"

check "stable has 0.18.0 after the stable release" version_in stable 0.18.0-1
check "latest has 0.18.0 too (nothing there yet, so it is newest)" version_in latest 0.18.0-1
check "Pages install.sh is a stable copy" installer_is "stable install.sh fixture"

# 2. A latest release: v0.19.0 (not on main, ahead of stable).
deb_dir="$work/debs-2"
mkdir -p "$deb_dir"
make_deb "0.19.0-1" "$deb_dir/storm-server_0.19.0-1_amd64.deb"
mtime_before=$(stat -c %Y "$repo_dir/install.sh")
"$script" latest "$deb_dir" "$repo_dir"
mtime_after=$(stat -c %Y "$repo_dir/install.sh")

check "stable still has 0.18.0 (a latest release never touches it)" version_in stable 0.18.0-1
check_not "stable did not gain 0.19.0" version_in stable 0.19.0-1
check "latest now has 0.19.0" version_in latest 0.19.0-1
if [ "$mtime_before" = "$mtime_after" ]; then
	echo "ok: Pages install.sh is untouched by a latest run"
else
	echo "FAIL: Pages install.sh is untouched by a latest run" >&2
	fail=1
fi

# 3. A stable fix: v0.18.1 — newer than 0.18.0, still older than latest's 0.19.0.
deb_dir="$work/debs-3"
mkdir -p "$deb_dir"
make_deb "0.18.1-1" "$deb_dir/storm-server_0.18.1-1_amd64.deb"
"$script" stable "$deb_dir" "$repo_dir"

check "stable now has 0.18.1" version_in stable 0.18.1-1
check_not "stable never gained latest's 0.19.0" version_in stable 0.19.0-1
check "latest kept its newer 0.19.0" version_in latest 0.19.0-1
check_not "latest was not downgraded to 0.18.1" version_in latest 0.18.1-1
check "Pages install.sh was refreshed by the stable fix" installer_is "stable install.sh fixture"

# 0.18.0 is now superseded in BOTH suites (stable moved to 0.18.1 just above,
# latest moved to 0.19.0 in step 2) — reprepro's own replace-on-includedeb
# does not delete the orphaned .deb from pool/ by itself (confirmed by this
# assertion failing without the `deleteunreferenced` call in
# apt-repo-build.sh); explicitly running it is what keeps pool/ from growing
# by a full .deb every single release forever.
check_not "pool/ no longer holds the fully-superseded 0.18.0 .deb (deleteunreferenced)" pool_has 0.18.0-1
check "pool/ still holds stable's current 0.18.1" pool_has 0.18.1-1
check "pool/ still holds latest's current 0.19.0" pool_has 0.19.0-1

exit "$fail"
