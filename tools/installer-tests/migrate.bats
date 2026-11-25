#!/usr/bin/env bats
# deploy/migrate-to-user.sh: detection, the dry-run plan on the three target
# fixtures (Linux systemd-service/omarchy, macOS LaunchDaemon, Debian/Ubuntu
# apt .deb), the workspace_roots edit, the Claude-memory key mapping,
# idempotent rerun, and the refusal cases.

load helpers

setup() {
  setup_migrate_world
}

# --- fixtures ---------------------------------------------------------

# A Linux systemd system service, binary at BIN (the omarchy-like default
# is /usr/local/bin; an apt .deb puts it at /usr/bin instead -- that is the
# only difference between the "linux-service" and "apt-deb" fixtures).
fixture_linux_service() {
  local bin=${1:-/usr/local/bin/storm-runtime}
  mkdir -p "$STORM_TEST_SYSROOT/etc/systemd/system" \
    "$STORM_TEST_SYSROOT/etc/storm-runtime" \
    "$STORM_TEST_SYSROOT/var/lib/storm-runtime/workspaces/scratch/Storm" \
    "$STORM_TEST_SYSROOT/var/lib/storm-runtime/workspaces/work" \
    "$(dirname "$STORM_TEST_SYSROOT$bin")"
  printf '[Service]\nUser=storm-runtime\nExecStart=%s serve\n' "$bin" \
    >"$STORM_TEST_SYSROOT/etc/systemd/system/storm-runtime.service"
  printf 'workspace_roots = ["%s/var/lib/storm-runtime/workspaces"]\n' "$STORM_TEST_SYSROOT" \
    >"$STORM_TEST_SYSROOT/etc/storm-runtime/runtime.toml"
  echo hello >"$STORM_TEST_SYSROOT/var/lib/storm-runtime/workspaces/scratch/Storm/README.md"
  echo hi >"$STORM_TEST_SYSROOT/var/lib/storm-runtime/workspaces/work/notes.txt"
  cp "$FAKE_SRC" "$STORM_TEST_SYSROOT$bin"
  chmod 0755 "$STORM_TEST_SYSROOT$bin"
}

fixture_apt_deb() {
  fixture_linux_service /usr/bin/storm-runtime
}

fixture_macos_daemon() {
  export FAKE_UNAME_S=Darwin
  mkdir -p "$STORM_TEST_SYSROOT/Library/LaunchDaemons" \
    "$STORM_TEST_SYSROOT/Library/StormRuntime/bin" \
    "$STORM_TEST_SYSROOT/Library/StormRuntime/workspaces/scratch/Storm"
  : >"$STORM_TEST_SYSROOT/Library/LaunchDaemons/dev.storm.runtime.plist"
  echo hello >"$STORM_TEST_SYSROOT/Library/StormRuntime/workspaces/scratch/Storm/README.md"
  cp "$FAKE_SRC" "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime"
  chmod 0755 "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime"
}

dry() {
  run sh "$MIGRATE" --dry-run --dest "$BATS_TEST_TMPDIR/newhome/storm/workspaces" "$@"
}

# --- plan / dry-run on each fixture -----------------------------------

@test "linux-service (omarchy-like): detects, plans, changes nothing" {
  fixture_linux_service
  dry
  [ "$status" -eq 0 ]
  [[ "$output" == *"OS: linux"* ]]
  [[ "$output" == *"Existing service install: service"* ]]
  [[ "$output" == *"account: storm-runtime"* ]]
  [[ "$output" == *"Workspaces found:"* ]]
  [[ "$output" == *"- scratch"* ]]
  [[ "$output" == *"- work"* ]]
  [[ "$output" == *"Stop and disable the old service host"* ]]
  [[ "$output" == *"Dry run: nothing was changed."* ]]
  # read-only: no mv, rm or chown -R against anything
  ! grep -qE ' (mv|chown -R|rm -rf) ' "$FAKE_LOG"
}

@test "apt-deb: binary at /usr/bin is still detected, leftovers say apt remove" {
  fixture_apt_deb
  dry
  [ "$status" -eq 0 ]
  [[ "$output" == *"Existing service install: apt"* ]]
  [[ "$output" == *"binary:  $STORM_TEST_SYSROOT/usr/bin/storm-runtime"* ]]
}

@test "macos-daemon: detects LaunchDaemon, plans, changes nothing" {
  fixture_macos_daemon
  dry
  [ "$status" -eq 0 ]
  [[ "$output" == *"OS: macos"* ]]
  [[ "$output" == *"Existing service install: service"* ]]
  [[ "$output" == *"account: _stormruntime"* ]]
  [[ "$output" == *"- scratch"* ]]
  [[ "$output" == *"LaunchAgent on macOS"* ]]
  [[ "$output" == *"Dry run: nothing was changed."* ]]
  ! grep -qE ' (mv|chown -R|rm -rf) ' "$FAKE_LOG"
}

@test "no old install: plan says so, still offers a fresh user install" {
  dry
  [ "$status" -eq 0 ]
  [[ "$output" == *"Existing service install: none found"* ]]
  [[ "$output" == *"Workspaces found: none"* ]]
  [[ "$output" == *"Install the latest Runtime Host as you"* ]]
}

# --- workspace_roots edit: no duplicate key (bug #466/#467) -----------

@test "toml_add_workspace_root never duplicates the key" {
  local cfg="$BATS_TEST_TMPDIR/runtime.toml"
  printf 'workspace_roots = ["/a/b"]\n' >"$cfg"
  toml_add_workspace_root "$cfg" "/x/y"
  [ "$(grep -c '^workspace_roots' "$cfg")" -eq 1 ]
  [[ "$(cat "$cfg")" == 'workspace_roots = ["/a/b", "/x/y"]' ]]
  # calling it again with the same path is a no-op, still one key
  toml_add_workspace_root "$cfg" "/x/y"
  [ "$(grep -c '^workspace_roots' "$cfg")" -eq 1 ]
  [[ "$(cat "$cfg")" == 'workspace_roots = ["/a/b", "/x/y"]' ]]
}

@test "toml_add_workspace_root handles an empty array and a missing file" {
  local cfg="$BATS_TEST_TMPDIR/empty.toml"
  printf 'workspace_roots = []\n' >"$cfg"
  toml_add_workspace_root "$cfg" "/x/y"
  [[ "$(cat "$cfg")" == 'workspace_roots = ["/x/y"]' ]]

  local cfg2="$BATS_TEST_TMPDIR/none/runtime.toml"
  toml_add_workspace_root "$cfg2" "/z"
  [[ "$(cat "$cfg2")" == 'workspace_roots = ["/z"]' ]]
}

@test "toml_add_workspace_root refuses a multi-line array rather than corrupt it" {
  local cfg="$BATS_TEST_TMPDIR/multiline.toml"
  printf 'workspace_roots = [\n  "/a",\n]\n' >"$cfg"
  run toml_add_workspace_root "$cfg" "/x/y"
  [ "$status" -ne 0 ]
  [[ "$output" == *"spans multiple lines"* ]]
  # unmodified
  [ "$(grep -c '^workspace_roots' "$cfg")" -eq 1 ]
}

# --- Claude memory key mapping -----------------------------------------

@test "memory_key replaces every non-alphanumeric character with -" {
  [ "$(memory_key "/home/me/storm/workspaces/scratch")" = "-home-me-storm-workspaces-scratch" ]
  [ "$(memory_key "/var/lib/storm-runtime/workspaces/scratch")" = "-var-lib-storm-runtime-workspaces-scratch" ]
}

# --- idempotent rerun ---------------------------------------------------

@test "a step already marked done is skipped on rerun" {
  : >"$STATE_FILE"
  run step_done move_workspaces
  [ "$status" -ne 0 ]
  mark_done move_workspaces
  run step_done move_workspaces
  [ "$status" -eq 0 ]
}

@test "--resume with no state file yet just notes it and continues" {
  fixture_linux_service
  run sh "$MIGRATE" --dry-run --resume --dest "$BATS_TEST_TMPDIR/newhome/storm/workspaces"
  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing to resume yet"* ]]
  [[ "$output" == *"Dry run: nothing was changed."* ]]
}

# --- refusals ------------------------------------------------------------

@test "refuses to run as root" {
  FAKE_USER=root
  run sh "$MIGRATE" --dry-run --dest "$BATS_TEST_TMPDIR/d"
  [ "$status" -ne 0 ]
  [[ "$output" == *"not as root"* ]]
}

@test "refuses to run as the service account" {
  FAKE_USER=storm-runtime
  run sh "$MIGRATE" --dry-run --dest "$BATS_TEST_TMPDIR/d"
  [ "$status" -ne 0 ]
  [[ "$output" == *"not the service account"* ]]

  FAKE_USER=_stormruntime
  run sh "$MIGRATE" --dry-run --dest "$BATS_TEST_TMPDIR/d"
  [ "$status" -ne 0 ]
  [[ "$output" == *"not the service account"* ]]
}

@test "refuses a cross-filesystem move without --allow-copy" {
  fixture_linux_service
  # A stat that reports a different device for the old workspace root than
  # for everything else, so SAME_FS comes out false without touching a
  # real second filesystem.
  cat >"$FAKE_DIR/bin/stat" <<EOF
#!/bin/sh
if [ "\$1" = -c ] && [ "\$2" = %d ]; then
  case "\$3" in
    "$STORM_TEST_SYSROOT"/var/lib/storm-runtime/workspaces*) echo 111 ;;
    *) echo 222 ;;
  esac
  exit 0
fi
exit 1
EOF
  chmod 0755 "$FAKE_DIR/bin/stat"
  run sh "$MIGRATE" --yes --dest "$BATS_TEST_TMPDIR/newhome/storm/workspaces"
  [ "$status" -ne 0 ]
  [[ "$output" == *"not on the same filesystem"* ]]
  [[ "$output" == *"--allow-copy"* ]]
  # refused before anything was touched
  ! grep -qE ' (mv|rm -rf) ' "$FAKE_LOG"
}

@test "a cross-filesystem move says so in the plan, and --allow-copy is accepted" {
  fixture_linux_service
  cat >"$FAKE_DIR/bin/stat" <<EOF
#!/bin/sh
if [ "\$1" = -c ] && [ "\$2" = %d ]; then
  case "\$3" in
    "$STORM_TEST_SYSROOT"/var/lib/storm-runtime/workspaces*) echo 111 ;;
    *) echo 222 ;;
  esac
  exit 0
fi
exit 1
EOF
  chmod 0755 "$FAKE_DIR/bin/stat"
  dry --allow-copy
  [ "$status" -eq 0 ]
  [[ "$output" == *"DIFFERENT from destination"* ]]
  [[ "$output" == *"rsync-copy (--allow-copy)"* ]]
}
