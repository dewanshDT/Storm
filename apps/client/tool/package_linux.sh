#!/usr/bin/env bash
# Packages the release Linux build (decision 125f). Run from apps/client after
# `flutter build linux --release`:
#
#   tool/package_linux.sh <version> <outdir>
#
# writes <outdir>/storm-app-<version>-x86_64-linux.tar.gz, which install.sh
# unpacks into the user's home on any distribution, and
# <outdir>/storm-app_<version>-1_amd64.deb for the apt repository. The deb
# needs dpkg-deb; without it only the tarball is written.
#
# STORM_MAX_GLIBC (e.g. 2.35) fails the run if any binary in the bundle needs
# a newer glibc: the runner and plugins link against the build machine's, and
# a bundle built on a new distribution refuses to start on an older one.
set -euo pipefail
# Packages carry their own modes: a builder's umask must not reach them, and
# dpkg-deb refuses a tree it would leave unreadable.
umask 022

version=$1
out=$(realpath -m "$2")
bundle=build/linux/x64/release/bundle
pkg=linux/packaging

test -x "$bundle/storm" || { echo "no $bundle/storm: build first" >&2; exit 1; }

glibc=$(find "$bundle" -type f \( -name storm -o -name '*.so*' \) -exec objdump -T {} + 2>/dev/null |
  grep -o 'GLIBC_[0-9][0-9.]*' | sort -uV | tail -n 1)
echo "highest glibc the bundle needs: ${glibc:-none}"
if [ -n "${STORM_MAX_GLIBC:-}" ] && [ -n "$glibc" ]; then
  newest=$(printf '%s\n' "GLIBC_$STORM_MAX_GLIBC" "$glibc" | sort -V | tail -n 1)
  if [ "$newest" != "GLIBC_$STORM_MAX_GLIBC" ]; then
    echo "the bundle needs $glibc, newer than the $STORM_MAX_GLIBC it must run on" >&2
    exit 1
  fi
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# The share/ tree is laid out as it is installed, so install.sh copies it into
# ~/.local/share and the deb into /usr/share without renaming anything.
share() {
  local root=$1 exec=$2
  install -Dm644 "$pkg/dev.storm.storm.desktop" "$root/applications/dev.storm.storm.desktop"
  sed -i "s|^Exec=.*|Exec=$exec|" "$root/applications/dev.storm.storm.desktop"
  for size in 256 512; do
    install -Dm644 "$pkg/dev.storm.storm-$size.png" \
      "$root/icons/hicolor/${size}x$size/apps/dev.storm.storm.png"
  done
}

mkdir -p "$out"

# --- the tarball ---------------------------------------------------------------

app="$stage/tar/storm-app"
mkdir -p "$app"
cp -R "$bundle"/. "$app"/
chmod -R u=rwX,go=rX "$app"
printf '%s\n' "$version" > "$app/VERSION"
cp ../../LICENSE "$app/LICENSE"
share "$app/share" storm-app
cat > "$app/README.txt" <<EOF
Storm ${version} for Linux (x86_64)

The easy way is the installer, which puts this in your home directory with a
launcher entry, no root needed:
  curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh -s -- --app

By hand: keep this folder anywhere and run ./storm. For a launcher entry, copy
share/ into ~/.local/share and set Exec= in
~/.local/share/applications/dev.storm.storm.desktop to this folder's storm.

It needs GTK 3 and libsecret. Without a Secret Service (gnome-keyring,
KWallet) running, sign-ins are kept in Storm's own settings instead, and
Settings > Advanced says so. LAN discovery needs Avahi.
EOF
tar -C "$stage/tar" -czf "$out/storm-app-$version-x86_64-linux.tar.gz" storm-app
echo "wrote $out/storm-app-$version-x86_64-linux.tar.gz"

# --- the deb -------------------------------------------------------------------

if ! command -v dpkg-deb > /dev/null; then
  echo "no dpkg-deb: skipped the .deb"
  exit 0
fi

root="$stage/deb"
mkdir -p "$root/usr/lib/storm-app" "$root/usr/bin" "$root/DEBIAN"
cp -R "$bundle"/. "$root/usr/lib/storm-app"/
chmod -R u=rwX,go=rX "$root/usr/lib/storm-app"
ln -s ../lib/storm-app/storm "$root/usr/bin/storm-app"
share "$root/usr/share" storm-app
install -Dm644 ../../LICENSE "$root/usr/share/doc/storm-app/copyright"

cat > "$root/DEBIAN/control" <<EOF
Package: storm-app
Version: ${version}-1
Architecture: amd64
Maintainer: Dewansh Singh Thakur <dewansh.dt@gmail.com>
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: libgtk-3-0t64 | libgtk-3-0, libsecret-1-0
Recommends: avahi-daemon
Suggests: gnome-keyring
Section: misc
Priority: optional
Homepage: https://github.com/dewanshDT/Storm
Description: Storm desktop app
 The Storm app for Linux: your notes and agent sessions, synced with your
 Storm server. It does not include the server (storm-server) and needs none
 on this machine.
EOF

deb="$out/storm-app_${version}-1_amd64.deb"
dpkg-deb --root-owner-group --build "$root" "$deb" > /dev/null
echo "wrote $deb"
