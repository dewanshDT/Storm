#!/bin/sh
# ONE-TIME bootstrap for decision 163e's cumulative apt repo. Run by a human
# with push access, from a checkout of this repo — never by CI, and never
# automatically. It mirrors the apt repo that is live today at
# https://dewanshdt.github.io/Storm/ (stable only, v0.18.0's debs), rebuilds
# reprepro state for it under both `stable` and `latest` codenames, and
# pushes that as the new `apt-repo` branch. From then on, apt-repo.yml
# restores this branch at the start of every run and persists its updates
# back to it — this script never runs again unless that branch is lost.
#
# Needs: wget, dpkg-deb (for the version used only in a log line),
# reprepro, gpg with the repo's existing signing key imported
# (deploy/release-secrets.md "Apt signing key"), and push access to origin.
#
# Usage: deploy/apt-repo-bootstrap.sh <keyid>
set -eu

keyid="${1:-}"
if [ -z "$keyid" ]; then
	echo "usage: apt-repo-bootstrap.sh <gpg-keyid>" >&2
	echo "  (the key deploy/storm-archive-keyring.gpg was exported from)" >&2
	exit 1
fi

site="https://dewanshdt.github.io/Storm/"
work=$(mktemp -d)
echo "mirroring $site into $work/mirror ..."
wget -m -np -nH -q -P "$work/mirror" "$site"

pool_dir=$(find "$work/mirror" -type d -name pool -print -quit)
if [ -z "$pool_dir" ]; then
	echo "apt-repo-bootstrap: no pool/ found under the mirrored site — is Pages up?" >&2
	exit 1
fi

repo_dir="$work/apt-repo"
deb_dir="$work/debs"
mkdir -p "$deb_dir"
find "$pool_dir" -name '*.deb' -exec cp {} "$deb_dir/" \;
count=$(find "$deb_dir" -name '*.deb' | wc -l)
echo "found $count existing .deb(s) to re-include"
if [ "$count" -eq 0 ]; then
	echo "apt-repo-bootstrap: nothing to bootstrap from" >&2
	exit 1
fi

cd "$(dirname "$0")/.."
STORM_APT_KEYID="$keyid" ./deploy/apt-repo-build.sh stable "$deb_dir" "$repo_dir"

echo
echo "Built $repo_dir. Next, push it as the apt-repo branch by hand:"
echo
echo "  git -C \"$repo_dir\" init -q -b apt-repo"
echo "  git -C \"$repo_dir\" add -A"
echo "  git -C \"$repo_dir\" -c user.name=you -c user.email=you@example.com \\"
echo "    commit -q -m 'apt-repo: bootstrap from live Pages content'"
echo "  git -C \"$repo_dir\" push <your-remote-url> apt-repo:apt-repo"
echo
echo "Then re-run the apt-repo workflow once (workflow_dispatch, channel=stable)"
echo "to confirm it restores this branch and republishes Pages unchanged."
