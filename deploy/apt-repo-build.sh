#!/bin/sh
# Adds a release's .debs to the cumulative two-suite apt repo (decision
# 163e). <repo-dir> is the reprepro base (conf/, db/, dists/, pool/) carried
# over from the previous run — this script only ever adds to it, never
# rebuilds it from scratch, so one release's run can never erase another's.
#
# - A stable release's debs go into the `stable` suite, and into `latest`
#   too, but only if they are newer than everything already there (apt
#   always serves the newest version in a suite anyway, so this just makes
#   `latest` describe that honestly).
# - A latest release's debs go into `latest` only.
# - The Pages install.sh/install.ps1 (deploy/install.sh, deploy/install.ps1,
#   next to this script) are written into <repo-dir> only on a stable run.
set -eu

channel="${1:-}"
deb_dir="${2:-}"
repo_dir="${3:-}"

case "$channel" in
stable | latest) ;;
*)
	echo "apt-repo-build: channel must be 'stable' or 'latest', got '$channel'" >&2
	exit 1
	;;
esac

if [ -z "$deb_dir" ] || [ -z "$repo_dir" ]; then
	echo "usage: apt-repo-build.sh <stable|latest> <deb-dir> <repo-dir>" >&2
	exit 1
fi

first_deb=$(find "$deb_dir" -maxdepth 1 -name '*.deb' -print -quit)
if [ -z "$first_deb" ]; then
	echo "apt-repo-build: no .deb files in $deb_dir" >&2
	exit 1
fi

keyid="${STORM_APT_KEYID:-}"
if [ -z "$keyid" ]; then
	echo "apt-repo-build: STORM_APT_GPG keyid must be in STORM_APT_KEYID" >&2
	exit 1
fi

mkdir -p "$repo_dir/conf"
# Both codenames are always declared, whichever one this run touches — the
# repo is one cumulative tree with two suites in it, never rebuilt from a
# single release's codename alone.
cat >"$repo_dir/conf/distributions" <<EOF
Origin: Storm
Label: Storm
Codename: stable
Architectures: amd64
Components: main
Description: Storm server apt repository — stable (public, fixes only)
SignWith: ${keyid}

Origin: Storm
Label: Storm
Codename: latest
Architectures: amd64
Components: main
Description: Storm server apt repository — latest (operator's line, all features)
SignWith: ${keyid}
EOF

# Every .deb in one release is stamped from the same tag (release.yml's
# "Stamp Cargo version from tag" steps), so any one's Version is the release's.
new_version=$(dpkg-deb -f "$first_deb" Version)

add_all() {
	suite="$1"
	for deb in "$deb_dir"/*.deb; do
		reprepro -b "$repo_dir" includedeb "$suite" "$deb"
	done
}

add_all "$channel"

if [ "$channel" = stable ]; then
	# The highest version already published to latest, if any.
	latest_version=$(reprepro -b "$repo_dir" list latest 2>/dev/null | awk '{print $NF}' | sort -V | tail -1)
	if [ -z "$latest_version" ] || dpkg --compare-versions "$new_version" gt "$latest_version"; then
		add_all latest
	fi
	script_dir=$(cd "$(dirname "$0")" && pwd)
	cp "$script_dir/install.sh" "$repo_dir/install.sh"
	cp "$script_dir/install.ps1" "$repo_dir/install.ps1"
fi

echo "apt-repo-build: $channel release $new_version added to $repo_dir"
