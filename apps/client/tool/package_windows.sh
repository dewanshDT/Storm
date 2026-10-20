#!/usr/bin/env bash
# Zips the release Windows build (decision 125d). Run from apps/client in Git
# Bash on a Windows runner, after `flutter build windows --release`:
#
#   tool/package_windows.sh dist/storm-app-windows-x86_64.zip <version>
#
# The release workflow and the acceptance workflow both call this, so the zip a
# tester tries is the zip a release ships.
set -euo pipefail

out=$(realpath -m "$1")
version=$2
build=build/windows/x64/runner/Release

test -f "$build/storm.exe" || { echo "no $build/storm.exe: build first" >&2; exit 1; }

# The runner and its plugins link the MSVC runtime dynamically, and a clean
# Windows does not always have it: without these the app stops with a
# missing-DLL dialog before any Storm code runs. Copying them next to the exe
# is Microsoft's supported "app-local" deployment.
vs=$("/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe" -latest -property installationPath)
crt=$(ls -d "$(cygpath -u "$vs")"/VC/Redist/MSVC/14.*/x64/Microsoft.VC14*.CRT | sort -V | tail -1)
cp "$crt"/*.dll "$build/"
for dll in msvcp140.dll vcruntime140.dll vcruntime140_1.dll; do
  test -f "$build/$dll" || { echo "the MSVC runtime has no $dll" >&2; exit 1; }
done

cp ../../LICENSE "$build/LICENSE.txt"
cat > "$build/README-windows.txt" <<EOF
Storm ${version} for Windows (x86_64)

The easy way is the installer, in PowerShell (no administrator needed for the
app on its own):
  irm https://dewanshdt.github.io/Storm/install.ps1 | iex

By hand: keep this folder anywhere you like and run storm.exe. It needs nothing
else installed. Your notes cache and sign-in live in your user profile, so
replacing this folder with a newer one keeps them.

Not code-signed: SmartScreen may warn. "More info", then "Run anyway".
EOF

mkdir -p "$(dirname "$out")"
rm -f "$out"
(cd "$build" && 7z a -tzip "$out" . > /dev/null)
ls -la "$out"
