#!/usr/bin/env bats
# Release assets: names, versions, checksums.txt, and "unavailable, because".

load helpers

setup() {
  setup_world
}

@test "asset names, in one place" {
  [ "$(asset_name server-macos 0.6.0)" = storm-server-0.6.0-macos-universal.tar.gz ]
  [ "$(asset_name runtime-macos 0.6.0)" = storm-runtime-0.6.0-macos-universal.tar.gz ]
  [ "$(asset_name app-macos 0.6.0)" = Storm-0.6.0-macos-arm64.zip ]
  [ "$(asset_name apk 0.6.0)" = storm-0.6.0.apk ]
  run asset_name nope 1.0.0
  [ "$status" -eq 1 ]
}

@test "release URLs" {
  REL_VERSION=0.6.0
  [ "$(release_url checksums.txt)" = https://github.com/dewanshDT/Storm/releases/download/v0.6.0/checksums.txt ]
}

@test "versions: normalize and compare" {
  [ "$(normalize_version v0.6.0)" = 0.6.0 ]
  [ "$(normalize_version 1.2.3-rc1)" = 1.2.3-rc1 ]
  run normalize_version "v1.2"
  [ "$status" -eq 1 ]
  run normalize_version "1.2.3;rm"
  [ "$status" -eq 1 ]
  ver_lt 0.5.0 0.6.0
  ver_lt 0.6.0 0.10.0
  ver_lt 0.9.9 1.0.0
  ! ver_lt 0.6.0 0.6.0
  ! ver_lt 0.6.1 0.6.0
  ! ver_lt 0.6.0-rc1 0.6.0
}

@test "version output parsing" {
  [ "$(echo 'storm-server 0.6.0' | parse_version_out)" = 0.6.0 ]
  [ "$(echo 'storm-runtime v0.5.2-rc1 (abc)' | parse_version_out)" = 0.5.2-rc1 ]
}

@test "checksums.txt: two-space and binary-mode lines; a missing name is empty" {
  f="$BATS_TEST_TMPDIR/checksums.txt"
  printf 'aaa  storm-0.6.0.apk\nbbb *Storm-0.6.0-macos-arm64.zip\n' >"$f"
  [ "$(checksum_for storm-0.6.0.apk "$f")" = aaa ]
  [ "$(checksum_for Storm-0.6.0-macos-arm64.zip "$f")" = bbb ]
  [ -z "$(checksum_for storm-server-0.6.0-macos-universal.tar.gz "$f")" ]
  checksum_has storm-0.6.0.apk "$f"
  ! checksum_has storm-9.apk "$f"
}

@test "the latest tag from the API, without jq" {
  [ "$(printf '{\n  "tag_name": "v0.7.1",\n  "x": 1\n}\n' | parse_tag_name)" = v0.7.1 ]
  [ "$(printf '{"url":"u","tag_name":"v0.7.2","name":"n"}' | parse_tag_name)" = v0.7.2 ]
  platform
  export FAKE_LATEST=0.7.3
  [ "$(release_latest)" = 0.7.3 ]
}

@test "the version of --from-dir files, from their names" {
  make_assets 0.6.1 server-tar
  [ "$(version_from_dir "$ASSETS")" = 0.6.1 ]
  make_assets 0.6.2 server-deb
  [ "$(version_from_dir "$ASSETS")" = 0.6.2 ]
}

@test "verify_file accepts a match and refuses a mismatch or an unlisted file" {
  platform
  make_assets 0.6.0 apk
  CHECKSUMS_FILE="$ASSETS/checksums.txt"
  run verify_file "$ASSETS/storm-0.6.0.apk" storm-0.6.0.apk
  [ "$status" -eq 0 ]
  echo changed >"$ASSETS/storm-0.6.0.apk"
  run verify_file "$ASSETS/storm-0.6.0.apk" storm-0.6.0.apk
  [ "$status" -eq 1 ]
  [[ "$output" == *"checksum mismatch for storm-0.6.0.apk"* ]]
  run verify_file "$ASSETS/storm-0.6.0.apk" other.apk
  [ "$status" -eq 1 ]
  [[ "$output" == *"isn't listed in checksums.txt"* ]]
}

# Availability on macOS, given a release's checksums.txt.
mac_release() {
  use_macos "${2:-arm64}"
  platform
  OPT_FROM_DIR=$ASSETS OPT_VERSION=$1
  WORK_DIR="$BATS_TEST_TMPDIR/work"
  mkdir -p "$WORK_DIR"
  release_init
  availability_all
}

@test "macOS: a release without the server tarball says from which version it ships" {
  make_assets 0.5.0 runtime-tar app-zip apk
  mac_release 0.5.0
  ! comp_available server
  [ "$(comp_reason server)" = "the macOS server ships from v0.6.0; this release is v0.5.0" ]
  comp_available runtime
  comp_available app
}

@test "macOS: a later release missing an asset names it" {
  make_assets 0.7.0 server-tar
  mac_release 0.7.0
  comp_available server
  ! comp_available runtime
  [ "$(comp_reason runtime)" = "storm-runtime-0.7.0-macos-universal.tar.gz isn't in v0.7.0's checksums.txt" ]
  [ "$(comp_reason app)" = "Storm-0.7.0-macos-arm64.zip isn't in v0.7.0's checksums.txt" ]
}

@test "macOS Intel: no app; the relay is always coming soon" {
  make_assets 0.6.0
  mac_release 0.6.0 x86_64
  ! comp_available app
  [[ "$(comp_reason app)" == *"Apple silicon only"* ]]
  ! comp_available relay
  [ "$(comp_reason relay)" = "coming soon — reach this machine from anywhere" ]
}

@test "--from-dir without checksums.txt makes every download unavailable" {
  make_assets 0.6.0
  rm "$ASSETS/checksums.txt"
  mac_release 0.6.0
  ! comp_available server
  [[ "$(comp_reason server)" == *"has no checksums.txt"* ]]
}

@test "offline with no version: the reason says to pass --version" {
  use_macos
  platform
  curl() { return 6; }
  WORK_DIR="$BATS_TEST_TMPDIR/work"
  mkdir -p "$WORK_DIR"
  release_init
  availability_all
  ! comp_available server
  [[ "$(comp_reason server)" == *"pass --version vX.Y.Z"* ]]
}

@test "linux --from-dir: a deb missing from checksums.txt is unavailable" {
  make_assets 0.6.0 server-deb
  echo deb >"$ASSETS/storm-runtime_0.6.0-1_amd64.deb"
  platform
  OPT_FROM_DIR=$ASSETS
  WORK_DIR="$BATS_TEST_TMPDIR/work"
  mkdir -p "$WORK_DIR"
  release_init
  availability_all
  comp_available server
  ! comp_available runtime
  [ "$(comp_reason runtime)" = "storm-runtime_0.6.0-1_amd64.deb isn't in $ASSETS/checksums.txt" ]
}
