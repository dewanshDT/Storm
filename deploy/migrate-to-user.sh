#!/bin/sh
# migrate-to-user.sh: move a Storm Runtime Host from its dedicated service
# account (systemd system service on Linux, LaunchDaemon on macOS) to running
# as the invoking user, on the latest channel.
#
#   curl -fsSL https://raw.githubusercontent.com/dewanshDT/Storm/staging/deploy/migrate-to-user.sh | sh -s -- [flags]
#
# Runs piped (stdin is the script itself), so every prompt reads /dev/tty.
set -eu

INSTALL_SH_URL="https://dewanshdt.github.io/Storm/install.sh"
STATE_FILE="${HOME:-}/.storm-migrate.state"
LOG_FILE="${HOME:-}/storm-migrate-$(date +%Y%m%d-%H%M%S).log"

# Prefixes every old-install path this script reads (not $HOME, which a
# test already swaps). tools/installer-tests sets STORM_TEST_SYSROOT to a
# scratch root (the same variable deploy/install.sh's own tests use);
# production never sets it, so ROOT is empty and paths are the real ones.
ROOT=${STORM_TEST_SYSROOT:-}

OPT_YES=0
OPT_DRY_RUN=0
OPT_DEST=""
OPT_TOOLCHAINS=auto
OPT_CLAUDE_MEMORY=auto
OPT_ALLOW_COPY=0
OPT_ENROLL=""
OPT_RESUME=0

usage() {
  cat <<'EOF'
Usage: migrate-to-user.sh [flags]

Moves this machine's Storm Runtime Host from its dedicated service account
to running as you, on the latest channel. Detects, prints a plan, asks to
confirm, then migrates with step-by-step progress.

Flags:
  --dest PATH          where workspaces move to (default: ~/storm/workspaces)
  -y, --yes            skip the confirmation prompt
  --dry-run            print the plan and exit; nothing is changed
  --toolchains         move .cargo, .rustup, etc. into your home (default: on, Linux, if found)
  --no-toolchains      never move toolchains
  --claude-memory      copy Claude Code project memory for moved workspaces (default: on, if found)
  --no-claude-memory   never copy Claude memory
  --allow-copy         if workspaces are not on the same filesystem as --dest,
                        rsync-copy instead of refusing
  --enroll STRING      the enrollment string from Settings > Agents & hosts
                        (skips the interactive prompt for it)
  --resume             rerun after a failure; every step already done is
                        skipped automatically, with or without this flag
  -h, --help           this help
EOF
}

# ---------------------------------------------------------------- output ---

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != "dumb" ]; then
  IS_TTY=1
  C_BOLD=$(printf '\033[1m'); C_DIM=$(printf '\033[2m')
  C_RED=$(printf '\033[31m'); C_GREEN=$(printf '\033[32m')
  C_YELLOW=$(printf '\033[33m')
  C_RESET=$(printf '\033[0m')
else
  IS_TTY=0
  C_BOLD=""; C_DIM=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_RESET=""
fi

say()  { printf '%s\n' "$*"; }
note() { printf '%s%s%s\n' "$C_DIM" "$*" "$C_RESET"; }
head_line() { printf '\n%s%s%s\n' "$C_BOLD" "$*" "$C_RESET"; }
ok()   { printf '  %s%s%s %s\n' "$C_GREEN" "✓" "$C_RESET" "$*"; }
warn() { printf '  %s%s%s %s\n' "$C_YELLOW" "!" "$C_RESET" "$*"; }
fail_line() { printf '  %s%s%s %s\n' "$C_RED" "✗" "$C_RESET" "$*"; }
die()  { fail_line "$*"; exit 1; }

# ------------------------------------------------------------------ time ---

now_s() { date +%s; }

fmt_duration() {
  s=$1
  if [ "$s" -lt 60 ]; then
    printf '%ss' "$s"
  else
    printf '%sm%ss' "$((s / 60))" "$((s % 60))"
  fi
}

commify() {
  printf '%s' "$1" | rev | sed -E 's/([0-9]{3})/\1,/g' | rev | sed 's/^,//'
}

# --------------------------------------------------------------- spinner ---

# spin_run DESC CMD... : runs CMD in the background, shows a spinner while
# it runs (a plain "DESC ..." line when stdout isn't a tty), prints CMD's
# output, and returns CMD's exit status.
spin_run() {
  desc=$1; shift
  out=$(mktemp)
  ("$@" >"$out" 2>&1; echo $? >"$out.rc") &
  pid=$!
  if [ "$IS_TTY" = 1 ]; then
    frames='-\|/'
    i=0
    while kill -0 "$pid" 2>/dev/null; do
      frame=$(printf '%s' "$frames" | cut -c$(((i % 4) + 1)))
      printf '\r  %s %s' "$frame" "$desc" >&2
      i=$((i + 1))
      sleep 1
    done
    printf '\r' >&2
  else
    printf '  %s ...\n' "$desc" >&2
    wait "$pid" 2>/dev/null || true
  fi
  wait "$pid" 2>/dev/null || true
  rc=$(cat "$out.rc" 2>/dev/null || echo 1)
  cat "$out"
  rm -f "$out" "$out.rc"
  return "$rc"
}

# ---------------------------------------------------------------- state ---

step_done() {
  [ -f "$STATE_FILE" ] && grep -qx "$1" "$STATE_FILE" 2>/dev/null
}

mark_done() {
  printf '%s\n' "$1" >>"$STATE_FILE"
}

# -------------------------------------------------------- arg parsing ---

while [ $# -gt 0 ]; do
  case "$1" in
    --dest)
      [ $# -ge 2 ] || die "--dest needs a path"
      OPT_DEST=$2; shift
      ;;
    --dest=*) OPT_DEST=${1#--dest=} ;;
    -y | --yes) OPT_YES=1 ;;
    --dry-run) OPT_DRY_RUN=1 ;;
    --toolchains) OPT_TOOLCHAINS=1 ;;
    --no-toolchains) OPT_TOOLCHAINS=0 ;;
    --claude-memory) OPT_CLAUDE_MEMORY=1 ;;
    --no-claude-memory) OPT_CLAUDE_MEMORY=0 ;;
    --allow-copy) OPT_ALLOW_COPY=1 ;;
    --enroll)
      [ $# -ge 2 ] || die "--enroll needs the enrollment string"
      OPT_ENROLL=$2; shift
      ;;
    --enroll=*) OPT_ENROLL=${1#--enroll=} ;;
    --resume) OPT_RESUME=1 ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown flag: $1 (see --help)" ;;
  esac
  shift
done

[ -n "$OPT_DEST" ] || OPT_DEST="$HOME/storm/workspaces"
case "$OPT_DEST" in
  /*) ;;
  *) OPT_DEST="$PWD/$OPT_DEST" ;;
esac

# ------------------------------------------------------------- safety ---

ME=$(id -un)
[ "$ME" != root ] || die "run this as yourself, not as root (it uses sudo where it needs to)"
case "$ME" in
  storm-runtime | _stormruntime) die "run this as your own account, not the service account ($ME)" ;;
esac

read_tty() {
  prompt=$1
  if [ -r /dev/tty ]; then
    printf '%s' "$prompt" >/dev/tty
    IFS= read -r REPLY_TTY </dev/tty
  else
    die "no /dev/tty to read from; pass --yes (and --enroll, if needed) for a non-interactive run"
  fi
}

confirm() {
  if [ "$OPT_YES" = 1 ]; then
    return 0
  fi
  read_tty "$1 [y/N] "
  case "$REPLY_TTY" in
    y | Y | yes | YES) return 0 ;;
    *) return 1 ;;
  esac
}

# ------------------------------------------------------------- sudo ---

SUDO_KEEPALIVE_PID=""
SUDO_KEEPALIVE_INTERVAL=${STORM_MIGRATE_SUDO_KEEPALIVE_INTERVAL:-60}

sudo_up() {
  sudo -v || die "sudo access is needed to stop the old service and move its files"
  ( while true; do sudo -n true 2>/dev/null; sleep "$SUDO_KEEPALIVE_INTERVAL"; done ) &
  SUDO_KEEPALIVE_PID=$!
}

# shellcheck disable=SC2329  # invoked from the EXIT trap string, not by name
sudo_down() {
  [ -n "$SUDO_KEEPALIVE_PID" ] && kill "$SUDO_KEEPALIVE_PID" 2>/dev/null || true
}

# -------------------------------------------------------- detection ---

OS=""
case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *) die "unsupported OS: $(uname -s) (this script covers Linux and macOS)" ;;
esac

OLD_KIND=none          # service | apt | none
OLD_BIN=""
OLD_VERSION=""
OLD_STATE_DIR=""
OLD_CONFIG=""
OLD_HOME=""
OLD_SERVICE_ACCOUNT=""
OLD_WORKSPACE_ROOTS=""  # newline-separated

dev_of() {
  if [ "$OPT_DRY_RUN" != 1 ]; then
    sudo stat -c %d "$1" 2>/dev/null || sudo stat -f %d "$1" 2>/dev/null
  else
    stat -c %d "$1" 2>/dev/null || stat -f %d "$1" 2>/dev/null
  fi
}

parse_workspace_roots() {
  cfg=$1
  if [ -r "$cfg" ]; then
    text=$(cat "$cfg")
  elif [ "$OPT_DRY_RUN" != 1 ] && sudo test -f "$cfg" 2>/dev/null; then
    text=$(sudo cat "$cfg")
  else
    return 0
  fi
  printf '%s\n' "$text" | grep -E '^[[:space:]]*workspace_roots[[:space:]]*=' | head -1 \
    | grep -o '"[^"]*"' | sed 's/^"//; s/"$//'
}

# Any systemd unit for the service, wherever it was written: an admin's own
# unit (as a hand-rolled, non-apt install like omarchy's uses, in /etc), or
# a package's (in /lib or /usr/lib).
service_unit_exists() {
  [ -f "$ROOT/etc/systemd/system/storm-runtime.service" ] ||
    [ -f "$ROOT/lib/systemd/system/storm-runtime.service" ] ||
    [ -f "$ROOT/usr/lib/systemd/system/storm-runtime.service" ]
}

detect_linux_old() {
  OLD_SERVICE_ACCOUNT=storm-runtime
  OLD_STATE_DIR="$ROOT/var/lib/storm-runtime"
  OLD_CONFIG="$ROOT/etc/storm-runtime/runtime.toml"
  OLD_HOME="$ROOT/var/lib/storm-runtime/home"

  if service_unit_exists; then
    if [ -x "$ROOT/usr/bin/storm-runtime" ]; then
      OLD_KIND=apt
      OLD_BIN="$ROOT/usr/bin/storm-runtime"
    else
      OLD_KIND=service
      OLD_BIN=$(command -v storm-runtime 2>/dev/null || echo "$ROOT/usr/local/bin/storm-runtime")
    fi
  fi

  if [ "$OLD_KIND" != none ] && [ -x "$OLD_BIN" ]; then
    OLD_VERSION=$("$OLD_BIN" --version 2>/dev/null | head -1 || true)
  fi
  OLD_WORKSPACE_ROOTS=$(parse_workspace_roots "$OLD_CONFIG")
  [ -n "$OLD_WORKSPACE_ROOTS" ] || OLD_WORKSPACE_ROOTS="$OLD_STATE_DIR/workspaces"
}

detect_macos_old() {
  OLD_SERVICE_ACCOUNT=_stormruntime
  OLD_STATE_DIR="$ROOT/Library/StormRuntime/state"
  OLD_CONFIG="$ROOT/Library/StormRuntime/runtime.toml"
  OLD_HOME="$ROOT/Library/StormRuntime/state/home"
  OLD_BIN="$ROOT/Library/StormRuntime/bin/storm-runtime"

  if [ -f "$ROOT/Library/LaunchDaemons/dev.storm.runtime.plist" ]; then
    OLD_KIND=service
  fi
  if [ "$OLD_KIND" != none ] && [ -x "$OLD_BIN" ]; then
    OLD_VERSION=$("$OLD_BIN" --version 2>/dev/null | head -1 || true)
  fi
  OLD_WORKSPACE_ROOTS=$(parse_workspace_roots "$OLD_CONFIG")
  [ -n "$OLD_WORKSPACE_ROOTS" ] || OLD_WORKSPACE_ROOTS="$ROOT/Library/StormRuntime/workspaces"
}

NEW_CONFIG=""
NEW_BIN=""
NEW_UNIT_OR_PLIST=""
USER_INSTALL_EXISTS=0

detect_user_install() {
  if [ "$OS" = linux ]; then
    NEW_CONFIG="$HOME/.config/storm-runtime/runtime.toml"
    NEW_BIN="$HOME/.local/bin/storm-runtime"
    NEW_UNIT_OR_PLIST="$HOME/.config/systemd/user/storm-runtime.service"
  else
    NEW_CONFIG="$HOME/Library/Application Support/StormRuntime/runtime.toml"
    NEW_BIN="$HOME/Library/Application Support/StormRuntime/bin/storm-runtime"
    NEW_UNIT_OR_PLIST="$HOME/Library/LaunchAgents/dev.storm.runtime.plist"
  fi
  [ -e "$NEW_UNIT_OR_PLIST" ] && USER_INSTALL_EXISTS=1 || USER_INSTALL_EXISTS=0
}

# A user unit over the apt .deb runs the package's /usr/bin binary in place,
# so there is no copy in ~/.local/bin to call.
resolve_new_bin() {
  [ -x "$NEW_BIN" ] && return 0
  if [ "$OS" = linux ] && [ -x "$ROOT/usr/bin/storm-runtime" ]; then
    NEW_BIN="$ROOT/usr/bin/storm-runtime"
  elif command -v storm-runtime >/dev/null 2>&1; then
    NEW_BIN=$(command -v storm-runtime)
  fi
}

# workspace sizes ------------------------------------------------------

WS_NAMES=""      # newline-separated workspace directory names found
WS_SIZES=""      # matching newline-separated human sizes
WS_ROOT_FOR=""   # matching newline-separated source root for each name
WS_UNLISTABLE="" # dry run only: roots it couldn't list without sudo

# Sized with sudo normally; --dry-run must never prompt for sudo or change
# anything, so it tries a plain du and says so when it can't read enough to
# measure, rather than guessing a size or falling back to sudo.
measure_dir_size() {
  dir=$1 label=$2
  if [ "$OPT_DRY_RUN" = 1 ]; then
    size=""
    if [ -r "$dir" ] && [ -x "$dir" ]; then
      size=$(du -sh "$dir" 2>/dev/null | awk '{print $1}')
    fi
    [ -n "$size" ] || size="(needs sudo to measure)"
    printf '%s' "$size"
    return 0
  fi
  size=$(spin_run "du -sh $label" sudo du -sh "$dir" 2>/dev/null | awk '{print $1}')
  [ -n "$size" ] || size="?"
  printf '%s' "$size"
}

collect_workspaces() {
  WS_NAMES=""; WS_SIZES=""; WS_ROOT_FOR=""
  old_ifs=$IFS
  IFS='
'
  for root in $OLD_WORKSPACE_ROOTS; do
    IFS=$old_ifs
    # The service's roots are 0700 to its own account: list them with sudo,
    # or (dry run, no sudo) say they couldn't be listed rather than "none".
    if [ "$OPT_DRY_RUN" = 1 ]; then
      if [ ! -r "$root" ] || [ ! -x "$root" ]; then
        # Absent (its parent says so) is "none"; hidden behind a 0700 parent
        # or itself unreadable is "couldn't list".
        parent=$(dirname "$root")
        if [ -e "$root" ] || { [ -e "$parent" ] && { [ ! -r "$parent" ] || [ ! -x "$parent" ]; }; }; then
          WS_UNLISTABLE="$WS_UNLISTABLE
$root"
        fi
        IFS='
'
        continue
      fi
      entries=$(find "$root" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)
    else
      sudo test -d "$root" 2>/dev/null || { IFS='
'; continue; }
      entries=$(sudo find "$root" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)
    fi
    IFS='
'
    for entry in $entries; do
      IFS=$old_ifs
      name=$(basename "$entry")
      size=$(measure_dir_size "$entry" "$name")
      WS_NAMES="$WS_NAMES
$name"
      WS_SIZES="$WS_SIZES
$size"
      WS_ROOT_FOR="$WS_ROOT_FOR
$root"
    done
    IFS='
'
  done
  IFS=$old_ifs
  WS_NAMES=$(printf '%s' "$WS_NAMES" | sed '/^$/d')
  WS_SIZES=$(printf '%s' "$WS_SIZES" | sed '/^$/d')
  WS_ROOT_FOR=$(printf '%s' "$WS_ROOT_FOR" | sed '/^$/d')
}

wants_toolchains() {
  if [ "$OPT_TOOLCHAINS" = 0 ]; then
    echo 0
  elif [ -n "$FOUND_TOOLCHAINS" ]; then
    echo 1
  else
    echo 0
  fi
}

wants_claude_memory() {
  if [ "$OPT_CLAUDE_MEMORY" = 0 ]; then
    echo 0
  elif [ -n "$FOUND_MEMORY" ]; then
    echo 1
  else
    echo 0
  fi
}

TOOLCHAIN_DIRS=".cargo .rustup .pub-cache .local/share/flutter .local/node"
FOUND_TOOLCHAINS=""
OLD_HOME_UNREADABLE=0  # --dry-run couldn't look under $OLD_HOME without sudo

old_home_readable() {
  [ -r "$OLD_HOME" ] && [ -x "$OLD_HOME" ]
}

collect_toolchains() {
  FOUND_TOOLCHAINS=""
  [ "$OS" = linux ] || return 0
  [ -d "$OLD_HOME" ] || return 0
  if [ "$OPT_DRY_RUN" = 1 ]; then
    if ! old_home_readable; then
      OLD_HOME_UNREADABLE=1
      return 0
    fi
    for d in $TOOLCHAIN_DIRS; do
      [ -e "$OLD_HOME/$d" ] && FOUND_TOOLCHAINS="$FOUND_TOOLCHAINS
$d"
    done
  else
    for d in $TOOLCHAIN_DIRS; do
      if sudo test -e "$OLD_HOME/$d" 2>/dev/null; then
        FOUND_TOOLCHAINS="$FOUND_TOOLCHAINS
$d"
      fi
    done
  fi
  FOUND_TOOLCHAINS=$(printf '%s' "$FOUND_TOOLCHAINS" | sed '/^$/d')
}

memory_key() {
  printf '%s' "$1" | sed 's/[^A-Za-z0-9]/-/g'
}

FOUND_MEMORY=""  # newline-separated workspace names with memory to copy

collect_claude_memory() {
  FOUND_MEMORY=""
  [ "$OS" = linux ] || return 0
  if [ "$OPT_DRY_RUN" = 1 ] && [ -d "$OLD_HOME" ] && ! old_home_readable; then
    OLD_HOME_UNREADABLE=1
    return 0
  fi
  old_ifs=$IFS; IFS='
'
  for name in $WS_NAMES; do
    IFS=$old_ifs
    root=$(printf '%s\n' "$WS_ROOT_FOR" | sed -n "$(printf '%s\n' "$WS_NAMES" | grep -n "^$name\$" | head -1 | cut -d: -f1)p")
    key=$(memory_key "$root/$name")
    if [ "$OPT_DRY_RUN" = 1 ]; then
      [ -d "$OLD_HOME/.claude/projects/$key/memory" ] && FOUND_MEMORY="$FOUND_MEMORY
$name"
    elif sudo test -d "$OLD_HOME/.claude/projects/$key/memory" 2>/dev/null; then
      FOUND_MEMORY="$FOUND_MEMORY
$name"
    fi
    IFS='
'
  done
  IFS=$old_ifs
  FOUND_MEMORY=$(printf '%s' "$FOUND_MEMORY" | sed '/^$/d')
}

SAME_FS=1
FREE_SPACE_KB=0

check_filesystem_and_space() {
  SAME_FS=1
  first_root=$(printf '%s\n' "$OLD_WORKSPACE_ROOTS" | head -1)
  if [ -d "$first_root" ] || { [ "$OPT_DRY_RUN" != 1 ] && sudo test -d "$first_root" 2>/dev/null; }; then
    dest_probe=$OPT_DEST
    while [ ! -d "$dest_probe" ] && [ "$dest_probe" != "/" ]; do
      dest_probe=$(dirname "$dest_probe")
    done
    old_dev=$(dev_of "$first_root")
    new_dev=$(dev_of "$dest_probe")
    [ "$old_dev" = "$new_dev" ] || SAME_FS=0
  fi
  dest_probe=$OPT_DEST
  while [ ! -d "$dest_probe" ] && [ "$dest_probe" != "/" ]; do
    dest_probe=$(dirname "$dest_probe")
  done
  FREE_SPACE_KB=$(df -Pk "$dest_probe" 2>/dev/null | awk 'NR==2{print $4}')
  [ -n "$FREE_SPACE_KB" ] || FREE_SPACE_KB=0
}

# -------------------------------------------------------------- plan ---

print_plan() {
  head_line "Storm Runtime Host migration: $ME@$(uname -n)"
  say "  OS: $OS"
  if [ "$OLD_KIND" = none ]; then
    say "  Existing service install: none found"
  else
    say "  Existing service install: $OLD_KIND, $OLD_VERSION"
    say "    binary:  $OLD_BIN"
    say "    state:   $OLD_STATE_DIR"
    say "    config:  $OLD_CONFIG"
    say "    account: $OLD_SERVICE_ACCOUNT"
    say "    workspace roots:"
    printf '%s\n' "$OLD_WORKSPACE_ROOTS" | sed 's/^/      /'
  fi
  if [ -n "$WS_NAMES" ]; then
    say "  Workspaces found:"
    old_ifs=$IFS; IFS='
'
    n=1
    for name in $WS_NAMES; do
      size=$(printf '%s\n' "$WS_SIZES" | sed -n "${n}p")
      say "    - $name ($size)"
      n=$((n + 1))
    done
    IFS=$old_ifs
  elif [ -n "$WS_UNLISTABLE" ]; then
    say "  Workspaces found: (needs sudo to list: $(printf '%s' "$WS_UNLISTABLE" | sed '/^$/d' | tr '\n' ' '))"
  else
    say "  Workspaces found: none"
  fi
  say "  Destination: $OPT_DEST"
  if [ "$SAME_FS" = 1 ]; then
    say "  Filesystem: same as destination (move will be instant renames)"
  else
    if [ "$OPT_ALLOW_COPY" = 1 ]; then
      say "  Filesystem: DIFFERENT from destination — will rsync-copy (--allow-copy)"
    else
      say "  Filesystem: DIFFERENT from destination — refusing (pass --allow-copy to rsync-copy instead)"
    fi
  fi
  say "  Free space at destination: $(( FREE_SPACE_KB / 1024 / 1024 )) GB"
  say "  Existing user install: $([ "$USER_INSTALL_EXISTS" = 1 ] && echo "yes, at $NEW_UNIT_OR_PLIST (will be upgraded in place)" || echo "none")"
  if [ "$OS" = linux ]; then
    if [ -n "$FOUND_TOOLCHAINS" ]; then
      say "  Toolchains found under the old home:"
      printf '%s\n' "$FOUND_TOOLCHAINS" | sed 's/^/    - /'
    elif [ "$OLD_HOME_UNREADABLE" = 1 ]; then
      say "  Toolchains found: (needs sudo to detect)"
    else
      say "  Toolchains found: none"
    fi
    if [ -n "$FOUND_MEMORY" ]; then
      say "  Claude memory found for: $(printf '%s' "$FOUND_MEMORY" | tr '\n' ' ')"
    elif [ "$OLD_HOME_UNREADABLE" = 1 ]; then
      say "  Claude memory found: (needs sudo to detect)"
    else
      say "  Claude memory found: none"
    fi
  fi

  head_line "Plan"
  n=1
  if [ "$OLD_KIND" != none ]; then
    say "  $n. Stop and disable the old $OLD_KIND host ($OLD_SERVICE_ACCOUNT). This ends every Storm session on this host, including any agent running under it."
    n=$((n + 1))
  fi
  if [ -n "$WS_NAMES" ]; then
    say "  $n. Move each workspace above into $OPT_DEST."
    n=$((n + 1))
  fi
  want_toolchains=$(wants_toolchains)
  if [ "$want_toolchains" = 1 ]; then
    say "  $n. Move toolchains (.cargo, .rustup, ...) into your home, skipping any you already have."
    n=$((n + 1))
  fi
  want_memory=$(wants_claude_memory)
  if [ "$want_memory" = 1 ]; then
    say "  $n. Copy Claude Code project memory for each moved workspace (merged, never overwritten)."
    n=$((n + 1))
  fi
  if [ -n "$WS_NAMES" ] || [ "$want_toolchains" = 1 ]; then
    say "  $n. chown everything moved to you ($ME)."
    n=$((n + 1))
  fi
  if [ -n "$WS_NAMES" ]; then
    say "  $n. Repair and prune git worktrees in any moved repo that has them."
    n=$((n + 1))
  fi
  say "  $n. Install the latest Runtime Host as you (systemd --user + linger on Linux; a LaunchAgent on macOS)."
  n=$((n + 1))
  if [ -n "$WS_NAMES" ]; then
    say "  $n. Add $OPT_DEST to the new runtime.toml's workspace_roots."
    n=$((n + 1))
  fi
  say "  $n. Enroll the new host and start it."
  n=$((n + 1))
  say "  $n. Summary, and what to do by hand afterwards (revoke the old host, delete leftovers)."

  TOTAL_STEPS=$((n))
}

# ------------------------------------------------------- step runner ---

STEP_N=0
CURRENT_STEP_DESC=""

step_begin() {
  STEP_N=$((STEP_N + 1))
  CURRENT_STEP_DESC=$1
  printf '\n%s[%s/%s]%s %s\n' "$C_BOLD" "$STEP_N" "$TOTAL_STEPS" "$C_RESET" "$1"
  STEP_START=$(now_s)
}

step_ok() {
  elapsed=$(($(now_s) - STEP_START))
  ok "$1 ($(fmt_duration "$elapsed"))"
}

step_skip() {
  note "  skipped: $1"
}

step_fail() {
  elapsed=$(($(now_s) - STEP_START))
  fail_line "$1 ($(fmt_duration "$elapsed"))"
}

# --------------------------------------------------------------- steps ---

STOP_WAIT_TRIES=${STORM_MIGRATE_STOP_WAIT_TRIES:-30}
STOP_WAIT_SLEEP=${STORM_MIGRATE_STOP_WAIT_SLEEP:-1}

# Polls (up to STOP_WAIT_TRIES x STOP_WAIT_SLEEP, default 30s) until the old
# Linux service is no longer active or deactivating. Nothing is moved while
# it might still be using its files.
wait_old_stopped_linux() {
  i=0
  while [ "$i" -lt "$STOP_WAIT_TRIES" ]; do
    state=$(systemctl is-active storm-runtime 2>/dev/null) || state=""
    case "$state" in
      active | deactivating) ;;
      *) return 0 ;;
    esac
    i=$((i + 1))
    [ "$IS_TTY" = 1 ] && printf '\r  waiting for the old service to stop (%s)...' "$state" >&2
    sleep "$STOP_WAIT_SLEEP"
  done
  [ "$IS_TTY" = 1 ] && printf '\r%*s\r' 60 "" >&2
  return 1
}

# Polls until launchctl says the old LaunchDaemon is no longer loaded.
wait_old_stopped_macos() {
  i=0
  while [ "$i" -lt "$STOP_WAIT_TRIES" ]; do
    sudo launchctl print system/dev.storm.runtime >/dev/null 2>&1 || return 0
    i=$((i + 1))
    [ "$IS_TTY" = 1 ] && printf '\r  waiting for the old LaunchDaemon to stop...' >&2
    sleep "$STOP_WAIT_SLEEP"
  done
  [ "$IS_TTY" = 1 ] && printf '\r%*s\r' 60 "" >&2
  return 1
}

do_stop_old_service() {
  step_begin "Stop and disable the old $OLD_KIND host"
  if [ "$OLD_KIND" = none ]; then
    step_skip "no old service found"
    return 0
  fi
  if step_done "stop_old"; then
    step_skip "already stopped"
    return 0
  fi
  if [ "$OS" = linux ]; then
    sudo systemctl disable --now storm-runtime 2>&1 | sed 's/^/    /' || true
    if ! wait_old_stopped_linux; then
      step_fail "the old service would not stop within ${STOP_WAIT_TRIES}x${STOP_WAIT_SLEEP}s; nothing was moved"
      return 1
    fi
  else
    uninstall_out=$(mktemp)
    # shellcheck disable=SC2024  # uninstall_out is our own file, not root's
    if sudo "$OLD_BIN" uninstall >"$uninstall_out" 2>&1; then
      rc=0
    else
      rc=$?
    fi
    sed 's/^/    /' "$uninstall_out"
    rm -f "$uninstall_out"
    if [ "$rc" -ne 0 ]; then
      sudo launchctl bootout system/dev.storm.runtime 2>&1 | sed 's/^/    /' || true
    fi
    if ! wait_old_stopped_macos; then
      step_fail "the old LaunchDaemon would not stop; nothing was moved"
      return 1
    fi
  fi
  mark_done "stop_old"
  step_ok "stopped"
}

move_one_workspace() {
  name=$1 src_root=$2
  src="$src_root/$name"
  dst="$OPT_DEST/$name"
  if [ -e "$dst" ] && [ ! -e "$src" ]; then
    return 0
  fi
  if [ -e "$dst" ]; then
    die "refusing to overwrite existing destination: $dst"
  fi
  mkdir -p "$OPT_DEST"
  if [ "$SAME_FS" = 1 ]; then
    sudo mv "$src" "$dst"
  elif [ "$OPT_ALLOW_COPY" = 1 ]; then
    sudo rsync -a --info=progress2 "$src"/ "$dst"/
    sudo rm -rf "$src"
  else
    die "$src is not on the same filesystem as $OPT_DEST; pass --allow-copy or choose a --dest on the same disk"
  fi
}

do_move_workspaces() {
  step_begin "Move workspaces into $OPT_DEST"
  if [ -z "$WS_NAMES" ]; then
    step_skip "no workspaces found"
    return 0
  fi
  if step_done "move_workspaces"; then
    step_skip "already moved"
    return 0
  fi
  old_ifs=$IFS; IFS='
'
  n=1
  for name in $WS_NAMES; do
    root=$(printf '%s\n' "$WS_ROOT_FOR" | sed -n "${n}p")
    if [ "$SAME_FS" = 1 ]; then
      say "    $name: same filesystem, instant rename"
    fi
    move_one_workspace "$name" "$root"
    n=$((n + 1))
  done
  IFS=$old_ifs
  mark_done "move_workspaces"
  step_ok "moved $(printf '%s\n' "$WS_NAMES" | sed '/^$/d' | wc -l | tr -d ' ') workspace(s)"
}

do_move_toolchains() {
  step_begin "Move toolchains into your home"
  want=$(wants_toolchains)
  if [ "$want" != 1 ]; then
    step_skip "none to move, or disabled with --no-toolchains"
    return 0
  fi
  if step_done "move_toolchains"; then
    step_skip "already moved"
    return 0
  fi
  old_ifs=$IFS; IFS='
'
  for d in $FOUND_TOOLCHAINS; do
    IFS=$old_ifs
    if [ -e "$HOME/$d" ]; then
      note "    keeping your own $d"
    else
      mkdir -p "$(dirname "$HOME/$d")"
      sudo mv "$OLD_HOME/$d" "$HOME/$d"
      say "    moved $d"
    fi
    IFS='
'
  done
  IFS=$old_ifs
  mkdir -p "$HOME/.local/bin"
  for b in node npm npx; do
    if [ -x "$HOME/.local/node/bin/$b" ]; then
      ln -sf "$HOME/.local/node/bin/$b" "$HOME/.local/bin/$b"
    fi
  done
  mark_done "move_toolchains"
  step_ok "toolchains moved"
}

do_copy_claude_memory() {
  step_begin "Copy Claude Code memory for moved workspaces"
  want=$(wants_claude_memory)
  if [ "$want" != 1 ]; then
    step_skip "none found, or disabled with --no-claude-memory"
    return 0
  fi
  if step_done "copy_memory"; then
    step_skip "already copied"
    return 0
  fi
  old_ifs=$IFS; IFS='
'
  n=1
  for name in $WS_NAMES; do
    root=$(printf '%s\n' "$WS_ROOT_FOR" | sed -n "${n}p")
    old_key=$(memory_key "$root/$name")
    new_key=$(memory_key "$OPT_DEST/$name")
    src="$OLD_HOME/.claude/projects/$old_key/memory"
    dst="$HOME/.claude/projects/$new_key/memory"
    if sudo test -d "$src" 2>/dev/null; then
      mkdir -p "$dst"
      if [ -z "$(ls -A "$dst" 2>/dev/null)" ]; then
        sudo cp -a "$src"/. "$dst"/
        sudo chown -R "$ME" "$dst"
        say "    $name: copied memory to $dst"
      else
        tmp=$(mktemp -d)
        sudo cp -a "$src"/. "$tmp"/
        cp -Rn "$tmp"/. "$dst"/ 2>/dev/null || true
        rm -rf "$tmp"
        sudo chown -R "$ME" "$dst"
        warn "    $name: memory already existed at $dst; merged only missing files, kept the rest"
      fi
    fi
    n=$((n + 1))
  done
  IFS=$old_ifs
  mark_done "copy_memory"
  step_ok "memory copied"
}

chown_with_progress() {
  path=$1
  [ -e "$path" ] || return 0
  # sudo: the moved trees still carry the service account's 0700 modes, so
  # an unprivileged find can't descend into them and would skip most files.
  total=$(spin_run "counting files under $(basename "$path")" sudo sh -c "find '$path' | wc -l" | tr -d ' ')
  [ -n "$total" ] && [ "$total" -gt 0 ] || { sudo chown -R "$ME" "$path"; return 0; }

  count_file=$(mktemp)
  group=$(id -gn)
  owner="$ME:$group"
  start=$(now_s)

  export COUNT_FILE="$count_file"
  # shellcheck disable=SC2016  # expanded by the inner sh -c, not here
  ( sudo find "$path" -print0 \
    | xargs -0 -n 2000 sh -c '
        owner=$1; shift
        sudo chown "$owner" "$@" 2>/dev/null
        echo "$#" >> "$COUNT_FILE"
      ' _ "$owner" ) &
  cpid=$!

  while kill -0 "$cpid" 2>/dev/null; do
    done_n=$(awk '{s+=$1} END{print s+0}' "$count_file" 2>/dev/null)
    [ -n "$done_n" ] || done_n=0
    elapsed=$(( $(now_s) - start ))
    if [ "$total" -gt 0 ]; then pct=$(( done_n * 100 / total )); else pct=0; fi
    if [ "$done_n" -gt 0 ] && [ "$elapsed" -gt 0 ]; then
      rate_x1000=$(( done_n * 1000 / elapsed ))
      if [ "$rate_x1000" -gt 0 ]; then
        remaining=$(( (total - done_n) * 1000 / rate_x1000 ))
      else
        remaining=0
      fi
      eta=" · ~$(fmt_duration "$remaining") left"
    else
      eta=""
    fi
    line="chown: $(commify "$done_n") / $(commify "$total") files ($pct%) · $(fmt_duration "$elapsed")$eta"
    if [ "$IS_TTY" = 1 ]; then
      printf '\r  %s' "$line"
    else
      printf '  %s\n' "$line"
    fi
    sleep 2
  done
  wait "$cpid" 2>/dev/null || true
  if [ "$IS_TTY" = 1 ]; then printf '\r%*s\r' 80 ""; fi
  rm -f "$count_file"
}

do_chown() {
  step_begin "chown moved files to $ME"
  if [ -z "$WS_NAMES" ]; then
    step_skip "nothing was moved"
    return 0
  fi
  if step_done "chown"; then
    step_skip "already done"
    return 0
  fi
  chown_with_progress "$OPT_DEST"
  if [ "$OPT_TOOLCHAINS" != 0 ]; then
    for d in $TOOLCHAIN_DIRS; do
      if [ -e "$HOME/$d" ]; then chown_with_progress "$HOME/$d"; fi
    done
  fi
  mark_done "chown"
  step_ok "ownership updated"
}

do_worktree_repair() {
  step_begin "Repair git worktrees under $OPT_DEST"
  if [ -z "$WS_NAMES" ]; then
    step_skip "nothing was moved"
    return 0
  fi
  if step_done "worktree_repair"; then
    step_skip "already done"
    return 0
  fi
  gitdirs_file=$(mktemp)
  find "$OPT_DEST" -maxdepth 6 -type d -name .git >"$gitdirs_file" 2>/dev/null || true
  n=0
  while IFS= read -r gitdir; do
    [ -n "$gitdir" ] || continue
    repo=$(dirname "$gitdir")
    wtcount=$(git -C "$repo" worktree list 2>/dev/null | wc -l | tr -d ' ')
    if [ -n "$wtcount" ] && [ "$wtcount" -gt 1 ]; then
      git -C "$repo" worktree repair >/dev/null 2>&1 || true
      git -C "$repo" worktree prune >/dev/null 2>&1 || true
      n=$((n + 1))
      say "    repaired $repo ($((wtcount - 1)) worktree(s))"
    fi
  done <"$gitdirs_file"
  rm -f "$gitdirs_file"
  mark_done "worktree_repair"
  step_ok "$n repo(s) repaired"
}

do_install_user_runtime() {
  step_begin "Install the latest Runtime Host as $ME"
  if step_done "install_user"; then
    step_skip "already installed"
    return 0
  fi
  # A pipeline's status is its last command's, so piping through sh and sed
  # would hide curl's and install.sh's real exit codes: download to a file
  # first, then run it, each with its own captured rc.
  install_sh=$(mktemp)
  if curl -fsSL -o "$install_sh" "$INSTALL_SH_URL"; then
    rc=0
  else
    rc=$?
  fi
  if [ "$rc" -ne 0 ]; then
    rm -f "$install_sh"
    step_fail "could not download $INSTALL_SH_URL"
    return 1
  fi
  install_out=$(mktemp)
  if sh "$install_sh" --yes --runtime --channel latest --user >"$install_out" 2>&1; then
    rc=0
  else
    rc=$?
  fi
  sed 's/^/    /' "$install_out"
  rm -f "$install_sh" "$install_out"
  if [ "$rc" -ne 0 ]; then
    step_fail "install.sh failed"
    return 1
  fi
  mark_done "install_user"
  step_ok "installed"
}

toml_add_workspace_root() {
  cfg=$1 newpath=$2
  if [ ! -f "$cfg" ]; then
    mkdir -p "$(dirname "$cfg")"
    printf 'workspace_roots = ["%s"]\n' "$newpath" >"$cfg"
    return 0
  fi
  if grep -Eq '^[[:space:]]*workspace_roots[[:space:]]*=[[:space:]]*\[[^]]*$' "$cfg" \
     && ! grep -Eq '^[[:space:]]*workspace_roots[[:space:]]*=.*\]' "$cfg"; then
    die "workspace_roots in $cfg spans multiple lines; add \"$newpath\" to it by hand, then re-run"
  fi
  awk -v newpath="$newpath" '
    /^[[:space:]]*workspace_roots[[:space:]]*=/ {
      if (index($0, "\"" newpath "\"") > 0) { print; done = 1; next }
      line = $0
      sub(/\][[:space:]]*$/, "", line)
      rest = line
      sub(/^[^\[]*\[/, "", rest)
      if (rest ~ /^[[:space:]]*$/) {
        printf "%s\"%s\"]\n", line, newpath
      } else {
        printf "%s, \"%s\"]\n", line, newpath
      }
      done = 1
      next
    }
    { print }
    END { if (!done) printf "workspace_roots = [\"%s\"]\n", newpath }
  ' "$cfg" >"$cfg.tmp.$$"
  mv "$cfg.tmp.$$" "$cfg"
}

do_add_workspace_root() {
  step_begin "Add $OPT_DEST to the new runtime.toml"
  if [ -z "$WS_NAMES" ]; then
    step_skip "no workspaces were moved"
    return 0
  fi
  if step_done "add_root"; then
    step_skip "already added"
    return 0
  fi
  toml_add_workspace_root "$NEW_CONFIG" "$OPT_DEST"
  mark_done "add_root"
  step_ok "workspace_roots updated"
  note "    $(grep workspace_roots "$NEW_CONFIG" 2>/dev/null | head -1)"
}

do_enroll() {
  step_begin "Enroll and start the new host"
  resolve_new_bin
  if step_done "enroll"; then
    step_skip "already enrolled"
  else
    enroll_str=$OPT_ENROLL
    if [ -z "$enroll_str" ]; then
      say "  In the Storm app: Settings › Agents & hosts › Enroll a host — paste the string here:"
      read_tty "> "
      enroll_str=$REPLY_TTY
    fi
    [ -n "$enroll_str" ] || die "no enrollment string given"
    # Capture enroll's own exit code (see do_install_user_runtime): piping
    # through sed would otherwise report sed's status, not enroll's.
    enroll_out=$(mktemp)
    if printf '%s\n' "$enroll_str" | "$NEW_BIN" enroll >"$enroll_out" 2>&1; then
      rc=0
    else
      rc=$?
    fi
    sed 's/^/    /' "$enroll_out"
    rm -f "$enroll_out"
    if [ "$rc" -ne 0 ]; then
      step_fail "enroll failed"
      return 1
    fi
    mark_done "enroll"
  fi
  if [ "$OS" = linux ]; then
    systemctl --user restart storm-runtime 2>&1 | sed 's/^/    /' || true
  else
    launchctl kickstart -k "gui/$(id -u)/dev.storm.runtime" 2>&1 | sed 's/^/    /' || true
  fi
  check_out=$(mktemp)
  if "$NEW_BIN" check >"$check_out" 2>&1; then
    rc=0
  else
    rc=$?
  fi
  sed 's/^/    /' "$check_out"
  rm -f "$check_out"
  if [ "$rc" -eq 0 ]; then
    step_ok "host is up"
  else
    step_fail "storm-runtime check failed; see the log"
    return 1
  fi
}

print_summary() {
  step_begin "Summary"
  if [ -n "$WS_NAMES" ]; then
    say "  Moved to $OPT_DEST:"
    old_ifs=$IFS; IFS='
'
    n=1
    for name in $WS_NAMES; do
      size=$(printf '%s\n' "$WS_SIZES" | sed -n "${n}p")
      say "    - $name ($size)"
      n=$((n + 1))
    done
    IFS=$old_ifs
  fi
  say "  New host: runs as $ME, config at $NEW_CONFIG"
  head_line "Do by hand"
  say "  1. Revoke the OLD host in the app: Settings › Agents & hosts, find the host named"
  say "     for this machine running as '$OLD_SERVICE_ACCOUNT', Revoke, then Remove."
  if [ "$OLD_KIND" != none ]; then
    say "  2. Once you're happy, delete the leftovers (never done automatically):"
    say "       sudo rm -rf $OLD_STATE_DIR"
    if [ "$OLD_KIND" = apt ]; then
      say "       sudo apt remove storm-runtime"
    elif [ -n "$OLD_BIN" ]; then
      say "       sudo rm -f $OLD_BIN"
    fi
    if [ "$OS" = linux ]; then
      say "       sudo userdel -r $OLD_SERVICE_ACCOUNT   # after confirming nothing else needs it"
    else
      say "       sudo /Library/StormRuntime/bin/storm-runtime uninstall --purge   # before removing the binary above"
    fi
  fi
  step_ok "done"
}

# ------------------------------------------------------------ failure ---

# shellcheck disable=SC2329  # invoked from the EXIT trap string, not by name
on_exit() {
  rc=$1
  sudo_down
  if [ "$rc" -ne 0 ] && [ -n "${CURRENT_STEP_DESC:-}" ]; then
    printf '\n%s%s%s\n' "$C_RED" "Migration stopped: $CURRENT_STEP_DESC" "$C_RESET" >&2
    say "Steps already completed are recorded in $STATE_FILE and will be skipped on rerun." >&2
    say "Re-run the same command (with --resume, or plain) to continue from here." >&2
  fi
  if [ "$rc" -eq 0 ]; then
    say "
Log: $LOG_FILE"
  else
    say "Log: $LOG_FILE" >&2
  fi
}

# --------------------------------------------------------------- main ---

main() {
  if [ "$OPT_RESUME" = 1 ] && [ ! -f "$STATE_FILE" ]; then
    note "--resume passed, but there is nothing to resume yet (first run)."
  fi

  if [ "$OS" = linux ]; then detect_linux_old; else detect_macos_old; fi
  detect_user_install
  # --dry-run must never prompt for sudo or change anything: sudo_up (and
  # its background keepalive) runs only for a real migration.
  if [ "$OPT_DRY_RUN" != 1 ]; then
    sudo_up
  fi
  collect_workspaces
  collect_toolchains
  collect_claude_memory
  check_filesystem_and_space

  print_plan

  if [ "$OPT_DRY_RUN" = 1 ]; then
    say ""
    note "Dry run: nothing was changed."
    return 0
  fi

  if [ "$SAME_FS" != 1 ] && [ "$OPT_ALLOW_COPY" != 1 ] && [ -n "$WS_NAMES" ]; then
    die "workspaces are not on the same filesystem as $OPT_DEST; re-run with --allow-copy, or pick a --dest on the same disk"
  fi

  say ""
  warn "Stopping the old host ends every Storm session on this host, including any agent running this script."
  confirm "Continue?" || { note "Cancelled."; return 1; }

  do_stop_old_service
  do_move_workspaces
  do_move_toolchains
  do_copy_claude_memory
  do_chown
  do_worktree_repair
  do_install_user_runtime
  do_add_workspace_root
  do_enroll
  print_summary
}

# STORM_MIGRATE_TEST=1 defines every function above (including the arg
# parsing and safety checks that already ran) and runs nothing else, so
# tools/installer-tests can source this file and call its functions
# directly -- the same convention deploy/install.sh uses for its own tests.
if [ "${STORM_MIGRATE_TEST:-}" != 1 ]; then
  status_file=$(mktemp)
  export STATUS_FILE="$status_file"
  ( trap 'rc=$?; on_exit "$rc"; echo "$rc" >"$STATUS_FILE"' EXIT; main "$@" ) 2>&1 | tee -a "$LOG_FILE"
  status=$(cat "$status_file")
  rm -f "$status_file"
  exit "$status"
fi
