#!/bin/sh
# Storm installer: the server, the Runtime Host, the desktop app and phone
# pairing, on macOS or Debian/Ubuntu Linux (decision 84).
#
#   curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh
#   sh install.sh --help
#
# Re-run it any time: it shows what is installed and offers upgrades,
# pairing, enrollment, status and uninstall.
#
# How this file is built (read before editing):
#   * The installer proper is bash (3.2 or newer, what macOS ships). It lives
#     in the quoted heredoc below, so a POSIX sh (dash on Debian) can read the
#     whole file without parsing any bash, then hand it to bash.
#   * Nothing runs until the last line. A truncated download leaves the
#     heredoc and the function unterminated, and the shell refuses to run it.
#   * STORM_INSTALL_TEST=1 defines every function and runs nothing; that is
#     how tools/installer-tests sources it. shellcheck runs on the extracted
#     body (tools/installer-tests/run.sh).
#
# Env:
#   STORM_APT_ROOT  apt/Pages root (default: https://dewanshdt.github.io/Storm)
#   STORM_CHANNEL   apt suite (default: stable)
#   NO_COLOR        plain output

storm_installer_body() {
  cat <<'__STORM_INSTALLER_BODY__'
# shellcheck shell=bash
# The Storm installer body. bash >= 3.2: no associative arrays, no ${v,,},
# no mapfile/readarray, integer `read -t` only, no namerefs, no {fd}>.

if [ -n "${STORM_INSTALL_SELF_TMP:-}" ]; then
  rm -f "$STORM_INSTALL_SELF_TMP"
  unset STORM_INSTALL_SELF_TMP
fi

# ------------------------------------------------------------ constants

STORM_REPO_URL="https://github.com/dewanshDT/Storm"
STORM_LATEST_API="https://api.github.com/repos/dewanshDT/Storm/releases/latest"
STORM_INSTALL_URL="https://dewanshdt.github.io/Storm/install.sh"
STORM_DEFAULT_PORT=8484
# The registry. Its order is the checklist's and the plan's.
STORM_COMPONENTS="server runtime agents app mobile relay"
# The first release with a macOS server tarball.
STORM_MACOS_SERVER_SINCE="0.6.0"
# A step returns this when the person chose to skip it.
STEP_SKIPPED=10
# Seconds between polls (health, account). Tests set it to 0.
POLL_SLEEP=2
HEALTH_TRIES=15

# Defaults until parse_args and ui_init set them properly.
OPT_YES=0 OPT_DRY_RUN=0 INTERACTIVE=0 RAW_KEYS=0 UI_IN=0 UI_FD=2
WORK_DIR="" LOG_FILE=/dev/null STTY_SAVED="" SPIN_PID=""

# ------------------------------------------------------------ arguments

usage() {
  cat <<EOF
Storm installer

Usage:
  curl -fsSL $STORM_INSTALL_URL | sh
  curl -fsSL $STORM_INSTALL_URL | sh -s -- [options] [command]
  sh install.sh [options] [command]

With no options it asks what to set up. Without a terminal it needs --yes
and the components to install.

Components:
  --everything         server, runtime, agent CLIs, desktop app, phone pairing
  --server             the Storm server (notes, sync, the web client)
  --runtime            the Runtime Host (runs coding agents for a Storm server)
  --agents             Claude Code / OpenCode for the Runtime Host
  --app                the desktop app (macOS; on Linux, the web client's address)
  --mobile             phone or tablet: pairing QR, Android APK QR, web address

Options:
  --yes                don't ask: use the flags and the defaults
  --dry-run            print the plan and change nothing
  --version vX.Y.Z     install this release (default: the latest)
  --data DIR           server data folder (default: ~/Storm on macOS, /srv/storm on Linux)
  --port N             server port (default: $STORM_DEFAULT_PORT)
  --addr HOST:PORT     the address phones and other machines use to reach the server
  --enroll-from-stdin  read the runtime's enrollment string from stdin
  --password-stdin     set the new server account's password from stdin
  --from-dir DIR       take release files from DIR (with its checksums.txt)
                       instead of downloading them
  -h, --help           this help

Commands:
  status               what is installed, running and enrolled
  upgrade              upgrade what is installed
  uninstall            remove components (never vaults or workspaces)

Examples:
  sh install.sh --yes --server --app --data ~/Storm --port 8484
  sh install.sh --yes --runtime --enroll-from-stdin < enrollment.txt
  sh install.sh --dry-run --everything
  sh install.sh status
EOF
}

arg_error() {
  printf 'install.sh: %s\n' "$1" >&2
  printf 'Run it with --help to see the options.\n' >&2
}

args_reset() {
  OPT_YES=0 OPT_DRY_RUN=0 OPT_VERSION="" OPT_DATA="" OPT_PORT="" OPT_ADDR=""
  OPT_ENROLL_STDIN=0 OPT_PASSWORD_STDIN=0 OPT_FROM_DIR="" OPT_COMPONENTS=""
  OPT_HELP=0 SUBCOMMAND=install
}

preset_components() {
  case "$1" in
    everything) printf '%s' "server runtime agents app mobile" ;;
    server-app) printf '%s' "server app mobile" ;;
    runtime) printf '%s' "runtime agents" ;;
    *) return 1 ;;
  esac
}

add_component() {
  case " $OPT_COMPONENTS " in
    *" $1 "*) ;;
    *) OPT_COMPONENTS="${OPT_COMPONENTS:+$OPT_COMPONENTS }$1" ;;
  esac
}

valid_port() {
  case "$1" in
    '' | *[!0-9]*) return 1 ;;
  esac
  [ "${#1}" -le 5 ] && [ "$((10#$1))" -ge 1 ] && [ "$((10#$1))" -le 65535 ]
}

# set_opt NAME VALUE: validate one valued option.
set_opt() {
  local v
  case "$1" in
    --version)
      if ! v=$(normalize_version "$2"); then
        arg_error "--version wants a release like v0.6.0, not '$2'"
        return 2
      fi
      OPT_VERSION=$v
      ;;
    --data)
      if [ -z "$2" ]; then
        arg_error "--data needs a directory"
        return 2
      fi
      v=$2
      case "$v" in
        \~) v=$HOME ;;
        \~/*) v="$HOME/${v#\~/}" ;;
      esac
      case "$v" in
        /*) ;;
        *) v="$PWD/$v" ;;
      esac
      OPT_DATA=${v%/}
      [ -n "$OPT_DATA" ] || OPT_DATA=/
      ;;
    --port)
      if ! valid_port "$2"; then
        arg_error "--port wants a number from 1 to 65535, not '$2'"
        return 2
      fi
      OPT_PORT=$((10#$2))
      ;;
    --addr)
      case "$2" in
        '' | *[[:space:]/]*)
          arg_error "--addr wants HOST or HOST:PORT, not '$2'"
          return 2
          ;;
      esac
      OPT_ADDR=$2
      ;;
    --from-dir)
      if [ ! -d "$2" ]; then
        arg_error "--from-dir: there's no directory '$2'"
        return 2
      fi
      OPT_FROM_DIR=$(cd "$2" && pwd)
      ;;
  esac
  return 0
}

parse_args() {
  args_reset
  local c
  while [ $# -gt 0 ]; do
    case "$1" in
      -y | --yes) OPT_YES=1 ;;
      --dry-run) OPT_DRY_RUN=1 ;;
      --version | --data | --port | --addr | --from-dir)
        if [ $# -lt 2 ]; then
          arg_error "$1 needs a value"
          return 2
        fi
        set_opt "$1" "$2" || return 2
        shift
        ;;
      --version=* | --data=* | --port=* | --addr=* | --from-dir=*)
        set_opt "${1%%=*}" "${1#*=}" || return 2
        ;;
      --server | --runtime | --app | --mobile | --agents) add_component "${1#--}" ;;
      --everything)
        for c in $(preset_components everything); do add_component "$c"; done
        ;;
      --relay)
        arg_error "remote access (the relay) is coming soon; it isn't installable yet"
        return 2
        ;;
      --enroll-from-stdin) OPT_ENROLL_STDIN=1 ;;
      --password-stdin) OPT_PASSWORD_STDIN=1 ;;
      -h | --help) OPT_HELP=1 ;;
      status | upgrade | uninstall | install) SUBCOMMAND=$1 ;;
      *)
        arg_error "unknown option or command: $1"
        return 2
        ;;
    esac
    shift
  done
  if [ "$OPT_ENROLL_STDIN" = 1 ] && [ "$OPT_PASSWORD_STDIN" = 1 ]; then
    arg_error "--enroll-from-stdin and --password-stdin both read stdin; use one per run"
    return 2
  fi
  return 0
}

# ------------------------------------------------------------ versions

# normalize_version v0.6.0 -> 0.6.0; fails on anything that isn't one.
normalize_version() {
  local v=${1#v}
  case "$v" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) return 1 ;;
  esac
  case "$v" in
    *[!0-9A-Za-z.+-]*) return 1 ;;
  esac
  printf '%s' "$v"
}

# ver_key 1.2.3-rc1 -> a number that sorts like the version (suffix ignored).
ver_key() {
  local v=${1#v} a b c
  v=${v%%[-+]*}
  a=${v%%.*}
  v=${v#*.}
  b=${v%%.*}
  c=${v#*.}
  c=${c%%.*}
  case "$a$b$c" in
    '' | *[!0-9]*)
      printf '0'
      return 0
      ;;
  esac
  printf '%d%05d%05d' "$((10#$a))" "$((10#$b))" "$((10#$c))"
}

ver_lt() {
  local a b
  a=$(ver_key "$1")
  b=$(ver_key "$2")
  [ "$a" -lt "$b" ]
}

# The first thing that looks like a version in `tool --version` output.
parse_version_out() {
  sed -n 's/^[^0-9]*\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9A-Za-z.+-]*\).*/\1/p' | head -n 1
}

# ------------------------------------------------------------ platform

detect_platform() {
  local s m f=/etc/os-release like="" ver=""
  s=$(uname -s 2>/dev/null) || s=unknown
  m=$(uname -m 2>/dev/null) || m=unknown
  case "$s" in
    Darwin) OS=macos ;;
    Linux) OS=linux ;;
    *) OS=other ;;
  esac
  case "$m" in
    arm64 | aarch64) ARCH=arm64 ;;
    x86_64 | amd64) ARCH=x86_64 ;;
    *) ARCH=$m ;;
  esac
  DEBIAN_LIKE=0 OS_ID="" OS_LABEL="$s"
  case "$OS" in
    linux)
      if [ "${STORM_INSTALL_TEST:-}" = 1 ] && [ -n "${STORM_TEST_OS_RELEASE:-}" ]; then
        f=$STORM_TEST_OS_RELEASE
      fi
      if [ -r "$f" ]; then
        # shellcheck disable=SC1090
        OS_ID=$(. "$f" && printf '%s' "${ID:-}")
        # shellcheck disable=SC1090
        like=$(. "$f" && printf '%s' "${ID_LIKE:-}")
        # shellcheck disable=SC1090
        OS_LABEL=$(. "$f" && printf '%s' "${PRETTY_NAME:-Linux}")
      fi
      case "$OS_ID:$like" in
        debian:* | ubuntu:* | *:*debian* | *:*ubuntu*) DEBIAN_LIKE=1 ;;
      esac
      ;;
    macos)
      ver=$(sw_vers -productVersion 2>/dev/null) || ver=""
      OS_LABEL="macOS${ver:+ $ver}"
      ;;
  esac
}

# Every path the installer touches, per OS. STORM_TEST_SYSROOT (tests only)
# moves the system paths under a scratch directory.
set_paths() {
  local root=""
  if [ "${STORM_INSTALL_TEST:-}" = 1 ]; then root=${STORM_TEST_SYSROOT:-}; fi
  ME=$(id -un)
  MY_UID=$(id -u)
  APT_ROOT=${STORM_APT_ROOT:-https://dewanshdt.github.io/Storm}
  APT_ROOT=${APT_ROOT%/}
  CHANNEL=${STORM_CHANNEL:-stable}
  KEYRING_URL="$APT_ROOT/storm-archive-keyring.gpg"
  case "$OS" in
    macos)
      MACHINE=Mac
      RT_ACCOUNT=_stormruntime
      RT_ROOT="$root/Library/StormRuntime"
      RT_BIN="$RT_ROOT/bin/storm-runtime"
      RT_STATE="$RT_ROOT/state"
      RT_CONFIG="$RT_ROOT/runtime.toml"
      RT_WORKSPACES="$RT_ROOT/workspaces"
      SRV_SUPPORT="$HOME/Library/Application Support/Storm"
      SRV_INSTALLED_BIN="$SRV_SUPPORT/bin/storm-server"
      SRV_PLIST="$HOME/Library/LaunchAgents/dev.storm.server.plist"
      DEFAULT_DATA="$HOME/Storm"
      APP_DIR="$root/Applications"
      AGENT_BIN_DIRS="$root/opt/homebrew/bin $root/usr/local/bin"
      LOG_FILE="$HOME/Library/Logs/Storm/install.log"
      ;;
    *)
      MACHINE=machine
      RT_ACCOUNT=storm-runtime
      RT_ROOT="$root/var/lib/storm-runtime"
      RT_BIN=storm-runtime
      RT_STATE="$RT_ROOT"
      RT_CONFIG="$root/etc/storm-runtime/runtime.toml"
      RT_WORKSPACES="$RT_ROOT/workspaces"
      SRV_INSTALLED_BIN=storm-server
      SRV_DROPIN="$root/etc/systemd/system/storm-server.service.d/data-root.conf"
      DEFAULT_DATA="$root/srv/storm"
      APP_DIR=""
      AGENT_BIN_DIRS="$root/usr/local/bin $root/usr/bin"
      KEYRING_DIR="$root/usr/share/keyrings"
      KEYRING_PATH="$KEYRING_DIR/storm.gpg"
      LIST_PATH="$root/etc/apt/sources.list.d/storm.list"
      LOG_FILE="${XDG_STATE_HOME:-$HOME/.local/state}/storm/install.log"
      ;;
  esac
  LOG_PATH=$LOG_FILE
}

# ------------------------------------------------------------ logging

# Anything that looks like a secret never reaches the log.
log_redact() {
  sed -e 's/sk-ant-[^[:space:]]*/sk-ant-[redacted]/g' \
    -e 's/storm-enroll:v1:[^[:space:]]*/storm-enroll:v1:[redacted]/g' \
    -e 's/CLAUDE_CODE_OAUTH_TOKEN=[^[:space:]]*/CLAUDE_CODE_OAUTH_TOKEN=[redacted]/g'
}

log_init() {
  local dir
  if [ "$OPT_DRY_RUN" = 1 ]; then
    LOG_FILE=/dev/null
    return 0
  fi
  dir=$(dirname "$LOG_FILE")
  if ! (umask 077 && mkdir -p "$dir" && : >>"$LOG_FILE") 2>/dev/null; then
    LOG_FILE=/dev/null
    return 0
  fi
  log "---- storm installer: $* ----"
}

log() {
  [ "$LOG_FILE" = /dev/null ] && return 0
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" | log_redact 2>/dev/null >>"$LOG_FILE" || :
}

# logged CMD...: run CMD with its output (redacted) in the log only.
logged() {
  local st
  {
    { "$@"; } 2>&1 | log_redact 2>/dev/null >>"$LOG_FILE"
    st=${PIPESTATUS[0]}
  } || :
  return "$st"
}

# probe CMD...: a read-only query. A dry run executes nothing, so there it
# answers 125 ("can't tell") without running anything.
probe() {
  [ "$OPT_DRY_RUN" = 1 ] && return 125
  "$@" 2>/dev/null
}

# ------------------------------------------------------------ privilege

# as_root [-n] CMD...: as root, through sudo unless we are root already.
# -n (or SUDO_NONINTERACTIVE=1, set inside spinner steps) never prompts.
as_root() {
  local n=""
  if [ "${1:-}" = -n ]; then
    n=-n
    shift
  fi
  [ "${SUDO_NONINTERACTIVE:-}" = 1 ] && n=-n
  if [ "$MY_UID" = 0 ]; then
    "$@"
  else
    sudo ${n:+"$n"} "$@"
  fi
}

# run_as [-n] [-H] USER CMD...: as USER; sudo only when that's someone else.
run_as() {
  local n="" h="" u
  while :; do
    case "${1:-}" in
      -n) n=-n ;;
      -H) h=-H ;;
      *) break ;;
    esac
    shift
  done
  [ "${SUDO_NONINTERACTIVE:-}" = 1 ] && n=-n
  u=$1
  shift
  if [ "$u" = "$ME" ]; then
    "$@"
  elif [ "$u" = root ]; then
    as_root ${n:+"$n"} "$@"
  elif [ "$MY_UID" = 0 ] && ! command -v sudo >/dev/null 2>&1; then
    runuser -u "$u" -- "$@"
  else
    sudo ${n:+"$n"} ${h:+"$h"} -u "$u" "$@"
  fi
}

can_sudo_quietly() {
  [ "$MY_UID" = 0 ] && return 0
  command -v sudo >/dev/null 2>&1 || return 1
  probe sudo -n true
}

sudo_prime() {
  plan_has_sudo || return 0
  [ "$MY_UID" = 0 ] && return 0
  if ! command -v sudo >/dev/null 2>&1; then
    ui_fail "Some steps need administrator rights, and sudo isn't installed. Run it as root, or install sudo."
    return 1
  fi
  ui_line ""
  ui_info "Some steps need administrator rights; sudo may ask for your password."
  if [ "$INTERACTIVE" = 1 ]; then
    sudo -v <&"$UI_IN" || return 1
  else
    sudo -v || return 1
  fi
}

# Before each sudo step: refresh the timestamp quietly, prompt only if it lapsed.
sudo_keepalive() {
  [ "$MY_UID" = 0 ] && return 0
  sudo -n -v 2>/dev/null && return 0
  if [ "$INTERACTIVE" = 1 ]; then sudo -v <&"$UI_IN"; else sudo -v; fi
}

file_owner() {
  [ -e "$1" ] || return 1
  stat -c %U "$1" 2>/dev/null || stat -f %Su "$1" 2>/dev/null
}

# Quote a word for display, the way a person would type it.
shq() {
  case "$1" in
    '' | *[!A-Za-z0-9_./:=@%+,-]*)
      printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
      ;;
    *) printf '%s' "$1" ;;
  esac
}

# ------------------------------------------------------------ the TUI

ui_init() {
  local tty=${STORM_TTY:-/dev/tty}
  local out=${STORM_TTY_OUT:-${STORM_TTY:-/dev/tty}}
  INTERACTIVE=0 RAW_KEYS=0 UI_IN=0 UI_FD=2 STTY_SAVED=""
  if [ "$OPT_YES" != 1 ] && (: <"$tty") 2>/dev/null && (: >>"$out") 2>/dev/null; then
    # 8 and 9: well clear of the descriptors callers (and bats) use.
    exec 8<"$tty" 9>>"$out"
    INTERACTIVE=1 UI_IN=8 UI_FD=9
    if [ -z "${STORM_LINE_MODE:-}" ] && [ "${TERM:-dumb}" != dumb ] && [ -t 8 ]; then
      STTY_SAVED=$(stty -g <&8 2>/dev/null) || STTY_SAVED=""
      [ -n "$STTY_SAVED" ] && RAW_KEYS=1
    fi
  fi
  ui_style
}

ui_style() {
  local colour=0 n
  C_RESET="" C_BOLD="" C_DIM="" C_RED="" C_GREEN="" C_YELLOW="" C_CYAN=""
  if [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    if [ "$RAW_KEYS" = 1 ] || { [ "$INTERACTIVE" != 1 ] && [ -t 2 ]; }; then colour=1; fi
  fi
  if [ "$colour" = 1 ] && command -v tput >/dev/null 2>&1; then
    n=$(tput colors 2>/dev/null) || n=8
    case "$n" in '' | *[!0-9]*) n=8 ;; esac
    [ "$n" -ge 8 ] || colour=0
  fi
  if [ "$colour" = 1 ]; then
    C_RESET=$'\033[0m' C_BOLD=$'\033[1m' C_DIM=$'\033[2m'
    C_RED=$'\033[31m' C_GREEN=$'\033[32m' C_YELLOW=$'\033[33m' C_CYAN=$'\033[36m'
  fi
  case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
    *UTF-8* | *utf-8* | *UTF8* | *utf8*)
      G_OK="✓" G_FAIL="✗" G_PTR="›" G_DOT="·" G_RULE="─────────────"
      SPIN_FRAMES=("⠋" "⠙" "⠹" "⠸" "⠼" "⠴" "⠦" "⠧" "⠇" "⠏")
      ;;
    *)
      G_OK="ok" G_FAIL="x" G_PTR=">" G_DOT="-" G_RULE="-------------"
      SPIN_FRAMES=("|" "/" "-" "\\")
      ;;
  esac
  G_ON="[x]" G_OFF="[ ]" G_WARN="!"
}

ui_line() { printf '%s\n' "$*" >&"$UI_FD"; }
ui_head() {
  ui_line ""
  ui_line "${C_BOLD}$*${C_RESET}"
}
ui_info() {
  ui_line "  $*"
  log "info: $*"
}
ui_note() {
  ui_line "  ${C_DIM}$*${C_RESET}"
  log "note: $*"
}
ui_ok() {
  ui_line "  ${C_GREEN}${G_OK}${C_RESET} $*"
  log "ok: $*"
}
ui_warn() {
  ui_line "  ${C_YELLOW}${G_WARN}${C_RESET} $*"
  log "warn: $*"
}
ui_fail() {
  ui_line "  ${C_RED}${G_FAIL}${C_RESET} $*"
  log "fail: $*"
}

# ui_pad TEXT WIDTH: TEXT padded to WIDTH characters (printf pads bytes,
# which misaligns "…" and friends).
ui_pad() {
  local n=$(($2 - ${#1}))
  [ "$n" -gt 0 ] || n=0
  printf '%s%*s' "$1" "$n" ''
}

ui_cursor() {
  [ "$RAW_KEYS" = 1 ] || return 0
  if [ "$1" = hide ]; then printf '\033[?25l' >&"$UI_FD"; else printf '\033[?25h' >&"$UI_FD"; fi
}

# ui_read_key: one key into KEY (up, down, enter, space, q, esc, or the character).
ui_read_key() {
  local k="" rest=""
  IFS= read -rsn1 k <&"$UI_IN" || return 1
  case "$k" in
    $'\033')
      IFS= read -rsn2 -t 1 rest <&"$UI_IN" || :
      case "$rest" in
        '[A' | 'OA') KEY=up ;;
        '[B' | 'OB') KEY=down ;;
        *) KEY=esc ;;
      esac
      ;;
    '' | $'\n' | $'\r') KEY=enter ;;
    ' ') KEY=space ;;
    k | K) KEY=up ;;
    j | J) KEY=down ;;
    q | Q) KEY=q ;;
    *) KEY=$k ;;
  esac
  return 0
}

# ui_menu TITLE ITEM...: ITEM is "label|hint", or "-" for a separator.
# Sets UI_INDEX to the chosen item's position (1-based); returns 1 on q.
ui_menu() {
  local title=$1 n sel=1 i
  shift
  local -a items
  items=("$@")
  n=${#items[@]}
  while [ "$sel" -le "$n" ] && [ "${items[$((sel - 1))]}" = "-" ]; do sel=$((sel + 1)); done
  UI_INDEX=0
  if [ "$INTERACTIVE" != 1 ]; then
    UI_INDEX=$sel
    return 0
  fi
  if [ "$RAW_KEYS" != 1 ]; then
    ui_menu_lines "$title" "$@"
    return
  fi
  ui_line ""
  ui_line "  ${C_BOLD}${title}${C_RESET}"
  ui_line ""
  ui_cursor hide
  ui_menu_draw "$sel" "$@"
  while :; do
    ui_read_key || KEY=q
    case "$KEY" in
      up)
        i=$sel
        while :; do
          i=$((i - 1))
          [ "$i" -lt 1 ] && i=$n
          [ "${items[$((i - 1))]}" != "-" ] && break
        done
        sel=$i
        ;;
      down)
        i=$sel
        while :; do
          i=$((i + 1))
          [ "$i" -gt "$n" ] && i=1
          [ "${items[$((i - 1))]}" != "-" ] && break
        done
        sel=$i
        ;;
      enter)
        UI_INDEX=$sel
        break
        ;;
      q | esc)
        UI_INDEX=0
        break
        ;;
      [1-9])
        if [ "$KEY" -le "$n" ] && [ "${items[$((KEY - 1))]}" != "-" ]; then sel=$KEY; fi
        ;;
    esac
    printf '\033[%dA' "$((n + 1))" >&"$UI_FD"
    ui_menu_draw "$sel" "$@"
  done
  ui_cursor show
  [ "$UI_INDEX" -gt 0 ]
}

ui_menu_draw() {
  local sel=$1 i=0 item label hint
  shift
  for item in "$@"; do
    i=$((i + 1))
    if [ "$item" = "-" ]; then
      printf '\r\033[2K    %s%s%s\n' "$C_DIM" "$G_RULE" "$C_RESET" >&"$UI_FD"
      continue
    fi
    label=${item%%|*}
    hint=""
    case "$item" in *"|"*) hint=${item#*|} ;; esac
    if [ "$i" = "$sel" ]; then
      printf '\r\033[2K  %s%s %s%s %s%s%s\n' "$C_CYAN$C_BOLD" "$G_PTR" "$(ui_pad "$label" 22)" "$C_RESET" "$C_DIM" "$hint" "$C_RESET" >&"$UI_FD"
    else
      printf '\r\033[2K    %s %s%s%s\n' "$(ui_pad "$label" 22)" "$C_DIM" "$hint" "$C_RESET" >&"$UI_FD"
    fi
  done
  printf '\r\033[2K  %s%s%s\n' "$C_DIM" "up/down move ${G_DOT} enter select ${G_DOT} q quit" "$C_RESET" >&"$UI_FD"
}

# The fallback when raw keys can't be read: a numbered list.
ui_menu_lines() {
  local title=$1 i=0 j=0 item label hint ans
  shift
  local -a map
  map=()
  ui_line ""
  ui_line "  $title"
  for item in "$@"; do
    i=$((i + 1))
    if [ "$item" = "-" ]; then
      ui_line "     $G_RULE"
      continue
    fi
    j=$((j + 1))
    map[j]=$i
    label=${item%%|*}
    hint=""
    case "$item" in *"|"*) hint=${item#*|} ;; esac
    ui_line "  $(printf '%2d' "$j")) $label${hint:+  - $hint}"
  done
  while :; do
    printf '  Choose 1-%d (q to quit): ' "$j" >&"$UI_FD"
    if ! IFS= read -r ans <&"$UI_IN"; then
      UI_INDEX=0
      return 1
    fi
    case "$ans" in
      q | Q)
        UI_INDEX=0
        return 1
        ;;
      '' | *[!0-9]*) ;;
      *)
        if [ "$ans" -ge 1 ] && [ "$ans" -le "$j" ]; then
          UI_INDEX=${map[$ans]}
          return 0
        fi
        ;;
    esac
    ui_line "  Type a number from the list."
  done
}

# ui_checklist TITLE ITEM...: ITEM is "key|label|state|note"; state is on,
# off or disabled (drawn dim, the note saying why). Sets UI_RESULT to the keys
# that are on, space separated; returns 1 on q.
ui_checklist() {
  local title=$1 item rest n=0 i sel=0 ans
  shift
  local -a keys labels states notes
  keys=() labels=() states=() notes=()
  for item in "$@"; do
    keys[n]=${item%%|*}
    rest=${item#*|}
    labels[n]=${rest%%|*}
    rest=${rest#*|}
    states[n]=${rest%%|*}
    notes[n]=${rest#*|}
    n=$((n + 1))
  done
  if [ "$INTERACTIVE" = 1 ] && [ "$RAW_KEYS" = 1 ]; then
    ui_line ""
    ui_line "  ${C_BOLD}${title}${C_RESET}"
    ui_line ""
    ui_cursor hide
    while [ "$sel" -lt "$n" ] && [ "${states[$sel]}" = disabled ]; do sel=$((sel + 1)); done
    ui_checklist_draw
    while :; do
      ui_read_key || KEY=q
      case "$KEY" in
        up) [ "$sel" -gt 0 ] && sel=$((sel - 1)) ;;
        down) [ "$sel" -lt $((n - 1)) ] && sel=$((sel + 1)) ;;
        space)
          case "${states[$sel]}" in
            on) states[sel]=off ;;
            off) states[sel]=on ;;
          esac
          ;;
        enter) break ;;
        q | esc)
          ui_cursor show
          return 1
          ;;
      esac
      printf '\033[%dA' "$((n + 1))" >&"$UI_FD"
      ui_checklist_draw
    done
    ui_cursor show
  elif [ "$INTERACTIVE" = 1 ]; then
    while :; do
      ui_line ""
      ui_line "  $title"
      i=0
      while [ "$i" -lt "$n" ]; do
        case "${states[$i]}" in
          disabled) ui_line "   $((i + 1))) ${G_OFF} ${labels[$i]}  (unavailable: ${notes[$i]})" ;;
          on) ui_line "   $((i + 1))) ${G_ON} ${labels[$i]}  ${notes[$i]}" ;;
          *) ui_line "   $((i + 1))) ${G_OFF} ${labels[$i]}  ${notes[$i]}" ;;
        esac
        i=$((i + 1))
      done
      printf '  Numbers to toggle (e.g. 1 3), Enter when done, q to go back: ' >&"$UI_FD"
      IFS= read -r ans <&"$UI_IN" || return 1
      case "$ans" in
        '') break ;;
        q | Q) return 1 ;;
      esac
      for i in $ans; do
        case "$i" in '' | *[!0-9]*) continue ;; esac
        if [ "$i" -lt 1 ] || [ "$i" -gt "$n" ]; then continue; fi
        i=$((i - 1))
        case "${states[$i]}" in
          on) states[i]=off ;;
          off) states[i]=on ;;
        esac
      done
    done
  fi
  UI_RESULT=""
  i=0
  while [ "$i" -lt "$n" ]; do
    [ "${states[$i]}" = on ] && UI_RESULT="${UI_RESULT:+$UI_RESULT }${keys[$i]}"
    i=$((i + 1))
  done
  return 0
}

# Draws ui_checklist's arrays (its locals, by bash's dynamic scope).
ui_checklist_draw() {
  local i=0 box
  while [ "$i" -lt "$n" ]; do
    if [ "${states[$i]}" = disabled ]; then
      printf '\r\033[2K    %s%s %s %s%s\n' "$C_DIM" "$G_OFF" "$(ui_pad "${labels[$i]}" 16)" "${notes[$i]}" "$C_RESET" >&"$UI_FD"
    else
      box=$G_OFF
      [ "${states[$i]}" = on ] && box=$G_ON
      if [ "$i" = "$sel" ]; then
        printf '\r\033[2K  %s%s %s %s%s %s%s%s\n' "$C_CYAN$C_BOLD" "$G_PTR" "$box" "$(ui_pad "${labels[$i]}" 16)" "$C_RESET" "$C_DIM" "${notes[$i]}" "$C_RESET" >&"$UI_FD"
      else
        printf '\r\033[2K    %s %s %s%s%s\n' "$box" "$(ui_pad "${labels[$i]}" 16)" "$C_DIM" "${notes[$i]}" "$C_RESET" >&"$UI_FD"
      fi
    fi
    i=$((i + 1))
  done
  printf '\r\033[2K  %s%s%s\n' "$C_DIM" "space toggles ${G_DOT} enter continues ${G_DOT} q back" "$C_RESET" >&"$UI_FD"
}

# ui_input PROMPT DEFAULT: one line, with a default; sets UI_RESULT.
ui_input() {
  local ans=""
  UI_RESULT=$2
  [ "$INTERACTIVE" = 1 ] || return 0
  printf '  %s%s%s [%s]: ' "$C_BOLD" "$1" "$C_RESET" "$2" >&"$UI_FD"
  IFS= read -r ans <&"$UI_IN" || ans=""
  [ -n "$ans" ] && UI_RESULT=$ans
  return 0
}

# ui_confirm PROMPT DEFAULT(y|n): without a terminal the answer is DEFAULT.
ui_confirm() {
  local ans="" def=${2:-n} hint="y/N"
  [ "$def" = y ] && hint="Y/n"
  if [ "$INTERACTIVE" != 1 ]; then
    [ "$def" = y ]
    return
  fi
  while :; do
    printf '  %s [%s]: ' "$1" "$hint" >&"$UI_FD"
    IFS= read -r ans <&"$UI_IN" || ans=""
    case "$ans" in
      '')
        [ "$def" = y ]
        return
        ;;
      y | Y | yes | YES | Yes) return 0 ;;
      n | N | no | NO | No) return 1 ;;
    esac
  done
}

# ui_secret_pipe PROMPT VALIDATOR BADMSG CMD...: read a secret with echo off
# and hand it to CMD on stdin. It lives in one local variable, and is never
# echoed, logged or put in an argument; it's cleared once CMD has it.
# Returns STEP_SKIPPED on an empty answer, else CMD's status.
ui_secret_pipe() {
  local prompt=$1 validator=$2 bad=$3 secret="" rc
  local paste_on=$'\033[200~' paste_off=$'\033[201~'
  shift 3
  while :; do
    printf '  %s: ' "$prompt" >&"$UI_FD"
    stty -echo <&"$UI_IN" 2>/dev/null || :
    IFS= read -r secret <&"$UI_IN" || secret=""
    stty echo <&"$UI_IN" 2>/dev/null || :
    printf '\n' >&"$UI_FD"
    # Bracketed-paste markers and stray whitespace from the clipboard.
    secret=${secret#"$paste_on"}
    secret=${secret%"$paste_off"}
    secret=${secret//$'\r'/}
    secret="${secret#"${secret%%[![:space:]]*}"}"
    secret="${secret%"${secret##*[![:space:]]}"}"
    if [ -z "$secret" ]; then
      return "$STEP_SKIPPED"
    fi
    if "$validator" "$secret"; then break; fi
    secret=""
    ui_warn "$bad"
  done
  # Without pipefail a pipeline's status is its last command's: CMD's.
  if printf '%s\n' "$secret" | "$@"; then rc=0; else rc=$?; fi
  secret=""
  unset secret
  return "$rc"
}

# ui_spinner DESC CMD...: run CMD in the background with its output in the
# log, a spinner meanwhile, then a ✓ or ✗ line. Returns CMD's status.
ui_spinner() {
  local desc=$1 rcf rc i=0
  shift
  rcf="$WORK_DIR/rc.$$.$RANDOM"
  log "== $desc"
  {
    (SUDO_NONINTERACTIVE=1 && "$@") </dev/null 2>&1
    echo "$?" >"$rcf"
  } | log_redact 2>/dev/null >>"$LOG_FILE" &
  SPIN_PID=$!
  if [ "$RAW_KEYS" = 1 ]; then
    ui_cursor hide
    # Keys typed meanwhile (a paste meant for the next prompt) stay unseen.
    stty -echo <&"$UI_IN" 2>/dev/null || :
    while kill -0 "$SPIN_PID" 2>/dev/null; do
      printf '\r\033[2K  %s%s%s %s' "$C_CYAN" "${SPIN_FRAMES[$((i % ${#SPIN_FRAMES[@]}))]}" "$C_RESET" "$desc" >&"$UI_FD"
      i=$((i + 1))
      sleep 0.1 2>/dev/null || sleep 1
    done
    printf '\r\033[2K' >&"$UI_FD"
    stty echo <&"$UI_IN" 2>/dev/null || :
    ui_cursor show
  else
    ui_line "  ... $desc"
  fi
  wait "$SPIN_PID" 2>/dev/null
  SPIN_PID=""
  rc=1
  [ -s "$rcf" ] && rc=$(cat "$rcf")
  rm -f "$rcf"
  case "$rc" in
    0) ui_ok "$desc" ;;
    "$STEP_SKIPPED") ui_note "skipped: $desc" ;;
    *) ui_fail "$desc" ;;
  esac
  return "$rc"
}

# ui_wait_until DESC CMD...: try CMD every POLL_SLEEP seconds until it
# succeeds; q skips (returns 1). Without a terminal it tries once.
ui_wait_until() {
  local desc=$1 key="" start=$SECONDS i=0
  shift
  while :; do
    if "$@" >/dev/null 2>&1; then
      [ "$RAW_KEYS" = 1 ] && printf '\r\033[2K' >&"$UI_FD"
      ui_ok "$desc"
      return 0
    fi
    if [ "$RAW_KEYS" = 1 ]; then
      printf '\r\033[2K  %s%s%s %s %s(%ss, q to skip)%s' "$C_CYAN" "${SPIN_FRAMES[$((i % ${#SPIN_FRAMES[@]}))]}" "$C_RESET" \
        "$desc" "$C_DIM" "$((SECONDS - start))" "$C_RESET" >&"$UI_FD"
      i=$((i + 1))
      key=""
      IFS= read -rsn1 -t "$POLL_SLEEP" key <&"$UI_IN" || key=""
      case "$key" in
        q | Q)
          printf '\r\033[2K' >&"$UI_FD"
          ui_note "skipped: $desc"
          return 1
          ;;
      esac
    elif [ "$INTERACTIVE" = 1 ]; then
      [ "$i" = 0 ] && ui_line "  $desc (type q and Enter to skip)"
      i=$((i + 1))
      key=""
      IFS= read -r -t "$POLL_SLEEP" key <&"$UI_IN" || key=""
      case "$key" in
        q | Q)
          ui_note "skipped: $desc"
          return 1
          ;;
      esac
    else
      return 1
    fi
  done
}

ui_header() {
  ui_line ""
  ui_line "  ${C_BOLD}Storm installer${C_RESET}   ${C_DIM}${REL_VERSION:+v$REL_VERSION $G_DOT }$OS_LABEL ($ARCH)${C_RESET}"
}

# ------------------------------------------------------------ releases

# Every release asset name the installer knows, in one place.
asset_name() {
  case "$1" in
    server-macos) printf 'storm-server-%s-macos-universal.tar.gz' "$2" ;;
    runtime-macos) printf 'storm-runtime-%s-macos-universal.tar.gz' "$2" ;;
    app-macos) printf 'Storm-%s-macos-arm64.zip' "$2" ;;
    apk) printf 'storm-%s.apk' "$2" ;;
    *) return 1 ;;
  esac
}

release_url() {
  printf '%s/releases/download/v%s/%s' "$STORM_REPO_URL" "$REL_VERSION" "$1"
}

fetch_url() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O- "$1"
  else
    echo "need curl or wget to download $1" >&2
    return 1
  fi
}

download_to() {
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 2 -sS -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    echo "need curl or wget to download $1" >&2
    return 1
  fi
}

parse_tag_name() {
  sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

release_latest() {
  local tag=""
  tag=$(fetch_url "$STORM_LATEST_API" 2>/dev/null | parse_tag_name)
  if [ -z "$tag" ] && command -v curl >/dev/null 2>&1; then
    # The API is rate limited; the web redirect isn't.
    tag=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$STORM_REPO_URL/releases/latest" 2>/dev/null |
      sed -n 's#.*/tag/##p')
  fi
  normalize_version "$tag"
}

# The version the files in DIR are for, from their names.
version_from_dir() {
  local f v
  # shellcheck disable=SC2046
  for f in "$1"/* $(awk '{print $2}' "$1/checksums.txt" 2>/dev/null); do
    f=${f##*/}
    f=${f#\*}
    v=""
    case "$f" in
      storm-server-*-macos-universal.tar.gz) v=${f#storm-server-} v=${v%-macos-universal.tar.gz} ;;
      storm-runtime-*-macos-universal.tar.gz) v=${f#storm-runtime-} v=${v%-macos-universal.tar.gz} ;;
      Storm-*-macos-arm64.zip) v=${f#Storm-} v=${v%-macos-arm64.zip} ;;
      storm-server_*.deb | storm-runtime_*.deb) v=${f#*_} v=${v%%_*} v=${v%-[0-9]*} ;;
      storm-*.apk) v=${f#storm-} v=${v%.apk} ;;
    esac
    if [ -n "$v" ] && normalize_version "$v" >/dev/null; then
      normalize_version "$v"
      return 0
    fi
  done
  return 1
}

# checksum_for NAME [FILE]: NAME's sha256 in checksums.txt ("<sha>  <name>").
checksum_for() {
  local f=${2:-$CHECKSUMS_FILE}
  if [ -z "$f" ] || [ ! -r "$f" ]; then return 1; fi
  awk -v n="$1" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print $1; exit } }' "$f"
}

checksum_has() {
  [ -n "$(checksum_for "$1" "${2:-}")" ]
}

sha256_of() {
  if [ "$OS" = macos ] && command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "need sha256sum or shasum to verify downloads" >&2
    return 1
  fi
}

sha_tool_display() {
  if [ "$OS" = macos ]; then printf 'shasum -a 256'; else printf 'sha256sum'; fi
}

# verify_file FILE NAME: FILE's sha256 must be NAME's line in checksums.txt.
verify_file() {
  local want got
  want=$(checksum_for "$2") || want=""
  if [ -z "$want" ]; then
    echo "$2 isn't listed in checksums.txt; refusing to use it"
    return 1
  fi
  got=$(sha256_of "$1") || return 1
  if [ "$got" != "$want" ]; then
    echo "checksum mismatch for $2: checksums.txt says $want, the file is $got"
    return 1
  fi
  echo "sha256 ok: $2"
}

# Which release, and its checksums.txt. A failure becomes REL_ERROR, which
# every component that needs a download then shows as its reason.
release_init() {
  REL_VERSION="" REL_ERROR="" CHECKSUMS_FILE=""
  if [ -n "$OPT_VERSION" ]; then
    REL_VERSION=$OPT_VERSION
  elif [ -n "$OPT_FROM_DIR" ]; then
    REL_VERSION=$(version_from_dir "$OPT_FROM_DIR") || REL_VERSION=""
    [ -n "$REL_VERSION" ] || REL_ERROR="can't tell which version the files in $OPT_FROM_DIR are; pass --version"
  else
    REL_VERSION=$(release_latest) || REL_VERSION=""
    [ -n "$REL_VERSION" ] || REL_ERROR="couldn't reach GitHub to find the latest release; check the network or pass --version vX.Y.Z"
  fi
  if [ -n "$OPT_FROM_DIR" ]; then
    if [ -f "$OPT_FROM_DIR/checksums.txt" ]; then
      CHECKSUMS_FILE="$OPT_FROM_DIR/checksums.txt"
    else
      REL_ERROR="$OPT_FROM_DIR has no checksums.txt to verify its files against"
    fi
  elif [ -n "$REL_VERSION" ]; then
    if download_to "$(release_url checksums.txt)" "$WORK_DIR/checksums.txt" >/dev/null 2>&1; then
      CHECKSUMS_FILE="$WORK_DIR/checksums.txt"
    else
      REL_ERROR="couldn't download checksums.txt for v$REL_VERSION"
    fi
  fi
  log "release: ${REL_VERSION:-unknown}${REL_ERROR:+ ($REL_ERROR)}"
}

release_ok() {
  if [ -n "$REL_ERROR" ]; then
    REASON=$REL_ERROR
    return 1
  fi
  return 0
}

# A .deb for PKG in --from-dir (the last in sort order if several).
deb_in_dir() {
  local f last=""
  for f in "$OPT_FROM_DIR/$1"_*.deb; do
    [ -f "$f" ] && last=$f
  done
  [ -n "$last" ] || return 1
  printf '%s' "$last"
}

# ------------------------------------------------------------ network

lan_addr() {
  local a=""
  case "$OS" in
    macos)
      a=$(ipconfig getifaddr en0 2>/dev/null) || a=""
      [ -n "$a" ] || a=$(ipconfig getifaddr en1 2>/dev/null) || a=""
      ;;
    linux)
      a=$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1)
      ;;
  esac
  printf '%s' "${a:-127.0.0.1}"
}

health_ok() {
  local out
  out=$(curl -fsS --max-time 5 "http://127.0.0.1:$1/v1/health" 2>/dev/null) || return 1
  case "$out" in *ok*) return 0 ;; esac
  return 1
}

web_url() { printf 'http://%s' "$ADVERTISE"; }

# ------------------------------------------------------------ the registry
#
# Each component provides <c>_available (sets REASON when not), <c>_detect,
# <c>_status_line, <c>_plan (adds steps for its MODE: install, upgrade,
# repair, keep or uninstall), and the step functions <c>_install,
# <c>_configure, <c>_verify and <c>_uninstall that the plan runs. Everything
# else here is generic over STORM_COMPONENTS.

comp_label() {
  case "$1" in
    server) printf 'Storm server' ;;
    runtime) printf 'Runtime Host' ;;
    agents) printf 'Agent CLIs' ;;
    app) printf 'Desktop app' ;;
    mobile) printf 'Phone / tablet' ;;
    relay) printf 'Remote access' ;;
  esac
}

comp_blurb() {
  case "$1" in
    server) printf 'your notes, sync and the web client' ;;
    runtime) printf 'runs coding agents for a Storm server' ;;
    agents) printf 'Claude Code and OpenCode for the runtime' ;;
    app)
      if [ "$OS" = macos ]; then printf 'Storm.app in /Applications'; else printf 'the web client (no Linux desktop app yet)'; fi
      ;;
    mobile) printf 'pairing QR, Android APK, web address' ;;
    relay) printf 'reach this machine from anywhere' ;;
  esac
}

comp_call() {
  local c=$1 fn=$2
  shift 2
  "${c}_${fn}" "$@"
}

in_sel() {
  case " $SEL " in *" $1 "*) return 0 ;; esac
  return 1
}

comp_installed() {
  case "$1" in
    server) [ "$SRV_INSTALLED" = 1 ] ;;
    runtime) [ "$RT_INSTALLED" = 1 ] ;;
    app) [ "$APP_INSTALLED" = 1 ] ;;
    *) return 1 ;;
  esac
}

comp_version() {
  case "$1" in
    server) printf '%s' "$SRV_VERSION" ;;
    runtime) printf '%s' "$RT_VERSION" ;;
    app) printf '%s' "$APP_VERSION" ;;
  esac
}

detect_all() {
  local c
  for c in $STORM_COMPONENTS; do comp_call "$c" detect; done
}

# AVAIL_<c> is 1 or 0; REASON_<c> says why not.
availability_all() {
  local c
  for c in $STORM_COMPONENTS; do
    REASON=""
    if comp_call "$c" available; then
      printf -v "AVAIL_$c" '%s' 1
    else
      printf -v "AVAIL_$c" '%s' 0
    fi
    printf -v "REASON_$c" '%s' "$REASON"
  done
}

comp_available() {
  local v="AVAIL_$1"
  [ "${!v:-0}" = 1 ]
}

comp_reason() {
  local v="REASON_$1"
  printf '%s' "${!v:-}"
}

comp_mode() {
  local v="MODE_$1"
  printf '%s' "${!v:-install}"
}

set_mode() { printf -v "MODE_$1" '%s' "$2"; }

# The server binary to drive: installed, else just extracted, else PATH.
srv_bin() {
  if [ "$OS" = macos ]; then
    if [ -x "$SRV_INSTALLED_BIN" ]; then
      printf '%s' "$SRV_INSTALLED_BIN"
    elif [ -n "$WORK_DIR" ] && [ -x "$WORK_DIR/server/storm-server" ]; then
      printf '%s' "$WORK_DIR/server/storm-server"
    else
      printf 'storm-server'
    fi
  else
    printf 'storm-server'
  fi
}

srv_bin_display() {
  if [ "$OS" = macos ]; then shq "$SRV_INSTALLED_BIN"; else printf 'storm-server'; fi
}

srv_default_owner() {
  if [ "$OS" = macos ]; then printf '%s' "$ME"; else printf 'storm'; fi
}

# State commands run as whoever owns the state directory.
srv_owner() {
  local o
  o=$(file_owner "$1") || o=""
  printf '%s' "${o:-$(srv_default_owner)}"
}

srv_as_owner() {
  local state=$1
  shift
  run_as "$(srv_owner "$state")" "$@"
}

# How a state command is prefixed on the plan, and whether that's sudo.
srv_owner_prefix() {
  local o
  o=$(srv_owner "$1")
  if [ "$o" = "$ME" ]; then
    printf ''
  elif [ "$o" = root ]; then
    printf 'sudo '
  else
    printf 'sudo -u %s ' "$o"
  fi
}

srv_owner_sudo() {
  if [ "$(srv_owner "$1")" = "$ME" ]; then printf 0; else printf 1; fi
}

srv_has_account() {
  local state=$1
  # On Linux the state directory is usually unreadable to us; ask anyway.
  if [ "$OS" = macos ] && [ ! -d "$state" ]; then return 1; fi
  srv_as_owner "$state" "$(srv_bin)" has-account --state "$state" >/dev/null 2>&1
}

show_qr() {
  local bin
  bin=$(srv_bin)
  if command -v "$bin" >/dev/null 2>&1; then
    "$bin" qr "$1" >&"$UI_FD" 2>&1 || ui_line "    $1"
  else
    ui_line "    $1"
  fi
}

# ------------------------------------------------------------ component: server

server_available() {
  local name
  REASON=""
  case "$OS" in
    macos)
      release_ok || return 1
      name=$(asset_name server-macos "$REL_VERSION")
      checksum_has "$name" && return 0
      if ver_lt "$REL_VERSION" "$STORM_MACOS_SERVER_SINCE"; then
        REASON="the macOS server ships from v$STORM_MACOS_SERVER_SINCE; this release is v$REL_VERSION"
      else
        REASON="$name isn't in v$REL_VERSION's checksums.txt"
      fi
      return 1
      ;;
    linux) linux_pkg_available server ;;
    *)
      REASON="the installer supports macOS and Debian/Ubuntu Linux"
      return 1
      ;;
  esac
}

linux_pkg_available() {
  local deb
  if [ "$DEBIAN_LIKE" != 1 ] || ! command -v apt-get >/dev/null 2>&1; then
    REASON="Storm's Linux packages are for Debian and Ubuntu (this is ${OS_ID:-an unknown Linux}); use the release binaries: $STORM_REPO_URL/releases"
    return 1
  fi
  if [ "$ARCH" != x86_64 ]; then
    REASON="Storm's Linux packages are built for x86_64 (this machine is $ARCH); see $STORM_REPO_URL/releases"
    return 1
  fi
  if [ -n "$OPT_FROM_DIR" ]; then
    if ! deb=$(deb_in_dir "storm-$1"); then
      REASON="there's no storm-$1_*.deb in $OPT_FROM_DIR"
      return 1
    fi
    if ! checksum_has "${deb##*/}"; then
      REASON="${deb##*/} isn't in $OPT_FROM_DIR/checksums.txt"
      return 1
    fi
  fi
  return 0
}

macos_server_state_from_plist() {
  [ -f "$SRV_PLIST" ] || return 1
  awk '/<key>STORM_STATE<\/key>/ { getline; sub(/.*<string>/, ""); sub(/<\/string>.*/, ""); print; exit }' "$SRV_PLIST"
}

server_detect() {
  local out port="" state="" rc
  SRV_INSTALLED=0 SRV_VERSION="" SRV_RUNNING=no SRV_ACCOUNT=no SRV_DATA="" SRV_PORT=""
  case "$OS" in
    macos)
      [ -x "$SRV_INSTALLED_BIN" ] && SRV_INSTALLED=1
      state=$(macos_server_state_from_plist 2>/dev/null) || state=""
      [ -n "$state" ] && SRV_DATA=${state%/state}
      ;;
    linux)
      command -v storm-server >/dev/null 2>&1 && SRV_INSTALLED=1
      if [ -r "$SRV_DROPIN" ]; then
        SRV_DATA=$(sed -n 's/^ReadWritePaths=\([^ ]*\).*/\1/p' "$SRV_DROPIN" | head -n 1)
      fi
      ;;
    *) return 0 ;;
  esac
  [ -n "$SRV_DATA" ] || SRV_DATA=$DEFAULT_DATA
  [ "$SRV_INSTALLED" = 1 ] || return 0
  out=$(probe "$(srv_bin)" --version) && SRV_VERSION=$(printf '%s\n' "$out" | parse_version_out)
  out=$(probe "$(srv_bin)" status) &&
    port=$(printf '%s\n' "$out" | sed -n 's/^listen[[:space:]]*:.*:\([0-9][0-9]*\)[[:space:]]*$/\1/p' | head -n 1)
  [ -n "$port" ] && SRV_PORT=$port
  if [ "$OPT_DRY_RUN" = 1 ]; then
    SRV_RUNNING=unknown SRV_ACCOUNT=unknown
    return 0
  fi
  health_ok "${SRV_PORT:-$STORM_DEFAULT_PORT}" && SRV_RUNNING=yes
  state="$SRV_DATA/state"
  if [ "$(srv_owner "$state")" != "$ME" ] && ! can_sudo_quietly; then
    SRV_ACCOUNT=unknown
    return 0
  fi
  rc=0
  probe run_as -n "$(srv_owner "$state")" "$(srv_bin)" has-account --state "$state" >/dev/null || rc=$?
  case "$rc" in
    0) SRV_ACCOUNT=yes ;;
    1) SRV_ACCOUNT=no ;;
    *) SRV_ACCOUNT=unknown ;;
  esac
  return 0
}

server_status_line() {
  local running=running
  if [ "$SRV_INSTALLED" != 1 ]; then
    printf 'not installed'
    return 0
  fi
  [ "$SRV_RUNNING" = yes ] || running="running: $SRV_RUNNING"
  printf 'installed%s %s %s %s data %s %s account: %s' "${SRV_VERSION:+ v$SRV_VERSION}" "$G_DOT" \
    "$running" "$G_DOT" "$SRV_DATA" "$G_DOT" "$SRV_ACCOUNT"
}

server_plan() {
  local mode state disp pfx bin
  mode=$(comp_mode server)
  state="$DATA_ROOT/state"
  bin=$(srv_bin_display)
  if [ "$mode" = uninstall ]; then
    case "$OS" in
      linux)
        plan_add spin 1 "Stop the server and remove the storm-server package (data in $DATA_ROOT stays)" \
          "sudo storm-server down\nsudo apt-get remove -y storm-server" server_uninstall
        ;;
      macos)
        plan_add spin 0 "Remove the server's LaunchAgent and binary (data in $DATA_ROOT stays)" \
          "$bin uninstall" server_uninstall
        ;;
    esac
    return 0
  fi
  if [ "$mode" != keep ]; then
    case "$OS" in
      linux)
        if [ -z "$OPT_FROM_DIR" ]; then apt_plan_repo; fi
        plan_add spin 1 "Install the storm-server package" "$(apt_install_display server "$mode")" server_install package "$mode"
        if [ "$mode" = install ]; then
          plan_add spin 1 "Set up and start the server (systemd; data in $DATA_ROOT, port $PORT)" \
            "sudo storm-server up --data-root $(shq "$DATA_ROOT") --port $PORT --host 0.0.0.0" server_install up
        else
          plan_add spin 1 "Restart the server on the new files" "sudo systemctl restart storm-server" server_install restart
        fi
        ;;
      macos)
        plan_fetch server
        plan_add spin 0 "Install the server as a LaunchAgent running as $ME (data in $DATA_ROOT, port $PORT)" \
          "./storm-server up --data-root $(shq "$DATA_ROOT") --port $PORT --host 0.0.0.0" server_install up
        ;;
    esac
  fi
  plan_add spin 0 "Check the server answers on port $PORT" "curl -fsS http://127.0.0.1:$PORT/v1/health" server_verify
  if [ "$SRV_ACCOUNT" != yes ]; then
    pfx=$(srv_owner_prefix "$state")
    if [ "$OPT_PASSWORD_STDIN" = 1 ]; then
      disp="$pfx$bin passwd --state $(shq "$state") --password-stdin   # the password from stdin"
    else
      disp="$pfx$bin pair --state $(shq "$state") --addr $ADVERTISE --qr"
      disp="$disp\n$pfx$bin has-account --state $(shq "$state")   # every ${POLL_SLEEP}s until it exists"
      if [ "$INTERACTIVE" = 1 ]; then
        disp="$disp\n(or) $pfx$bin passwd --state $(shq "$state")"
      fi
    fi
    plan_add fg "$(srv_owner_sudo "$state")" "Set up your account: pair a device (QR code) or set a password" "$disp" server_configure account
  fi
  return 0
}

server_install() {
  case "$1" in
    package) apt_install_pkg storm-server "${2:-install}" ;;
    up)
      case "$OS" in
        linux) as_root storm-server up --data-root "$DATA_ROOT" --port "$PORT" --host 0.0.0.0 ;;
        macos)
          if [ ! -x "$WORK_DIR/server/storm-server" ]; then
            echo "the server tarball has no storm-server"
            return 1
          fi
          (cd "$WORK_DIR/server" && ./storm-server up --data-root "$DATA_ROOT" --port "$PORT" --host 0.0.0.0)
          ;;
      esac
      ;;
    restart) as_root systemctl restart storm-server ;;
  esac
}

server_verify() {
  local i=0
  while [ "$i" -lt "$HEALTH_TRIES" ]; do
    if health_ok "$PORT"; then
      echo "health ok on port $PORT"
      return 0
    fi
    sleep "$POLL_SLEEP"
    i=$((i + 1))
  done
  echo "nothing answered at http://127.0.0.1:$PORT/v1/health"
  return 1
}

server_configure() {
  case "$1" in
    account) server_account ;;
  esac
}

server_account() {
  local state="$DATA_ROOT/state" bin rc pw=""
  bin=$(srv_bin)
  if srv_has_account "$state"; then
    SRV_ACCOUNT=yes
    ui_ok "The server already has an account"
    return 0
  fi
  if [ "$OPT_PASSWORD_STDIN" = 1 ]; then
    IFS= read -r pw || :
    if [ -z "$pw" ]; then
      ui_fail "--password-stdin: there was no password on stdin"
      return 1
    fi
    if printf '%s\n' "$pw" | logged srv_as_owner "$state" "$bin" passwd --state "$state" --password-stdin; then rc=0; else rc=$?; fi
    pw=""
    [ "$rc" = 0 ] || return "$rc"
    SRV_ACCOUNT=yes
    ui_ok "Account created with the password from stdin"
    return 0
  fi
  if [ "$INTERACTIVE" != 1 ]; then
    ui_info "No account yet. The first device that scans this creates it:"
    srv_as_owner "$state" "$bin" pair --state "$state" --addr "$ADVERTISE" --qr >&"$UI_FD" 2>&1 || return 1
    ui_note "Or set a password: $(srv_owner_prefix "$state")$(srv_bin_display) passwd --state $(shq "$state")"
    return 0
  fi
  ui_menu "This server has no account yet. How will you sign in?" \
    "Pair a phone or tablet|scan a QR code; the first device creates the account" \
    "Set a password here|then sign in from the web client or the app" \
    "Later|re-run the installer and choose Pair a device" || return "$STEP_SKIPPED"
  case "$UI_INDEX" in
    1) server_pair_and_wait ;;
    2)
      srv_as_owner "$state" "$bin" passwd --state "$state" <&"$UI_IN" >&"$UI_FD" 2>&"$UI_FD" || return 1
      srv_has_account "$state" || return 1
      SRV_ACCOUNT=yes
      ui_ok "Account created. Sign in at $(web_url) or from the app."
      ;;
    *) return "$STEP_SKIPPED" ;;
  esac
}

server_pair_and_wait() {
  local state="$DATA_ROOT/state" bin
  bin=$(srv_bin)
  ui_info "Scan this with the Storm app on your phone or tablet; it must be on the same network as this $MACHINE."
  srv_as_owner "$state" "$bin" pair --state "$state" --addr "$ADVERTISE" --qr <&"$UI_IN" >&"$UI_FD" 2>&"$UI_FD" || return 1
  if ui_wait_until "Waiting for your device to create the account" srv_has_account "$state"; then
    SRV_ACCOUNT=yes
    return 0
  fi
  return "$STEP_SKIPPED"
}

server_uninstall() {
  case "$OS" in
    linux)
      as_root storm-server down || :
      as_root env DEBIAN_FRONTEND=noninteractive apt-get remove -y storm-server
      ;;
    macos) "$SRV_INSTALLED_BIN" uninstall ;;
  esac
}

# ------------------------------------------------------------ component: runtime

runtime_available() {
  local name
  REASON=""
  case "$OS" in
    macos)
      release_ok || return 1
      name=$(asset_name runtime-macos "$REL_VERSION")
      checksum_has "$name" && return 0
      REASON="$name isn't in v$REL_VERSION's checksums.txt"
      return 1
      ;;
    linux) linux_pkg_available runtime ;;
    *)
      REASON="the installer supports macOS and Debian/Ubuntu Linux"
      return 1
      ;;
  esac
}

runtime_detect() {
  local out
  RT_INSTALLED=0 RT_VERSION="" RT_ENROLLED=no RT_RUNNING=no
  case "$OS" in
    macos) [ -x "$RT_BIN" ] && RT_INSTALLED=1 ;;
    linux) command -v storm-runtime >/dev/null 2>&1 && RT_INSTALLED=1 ;;
    *) return 0 ;;
  esac
  [ "$RT_INSTALLED" = 1 ] || return 0
  out=$(probe "$RT_BIN" --version) && RT_VERSION=$(printf '%s\n' "$out" | parse_version_out)
  if [ "$OPT_DRY_RUN" = 1 ] || ! can_sudo_quietly; then
    RT_ENROLLED=unknown RT_RUNNING=unknown
    return 0
  fi
  probe as_root -n test -f "$RT_STATE/host.json" && RT_ENROLLED=yes
  case "$OS" in
    linux) probe systemctl is-active --quiet storm-runtime && RT_RUNNING=yes ;;
    macos)
      out=$(probe as_root -n launchctl print system/dev.storm.runtime) || out=""
      case "$out" in *"state = running"*) RT_RUNNING=yes ;; esac
      ;;
  esac
  return 0
}

runtime_status_line() {
  if [ "$RT_INSTALLED" != 1 ]; then
    printf 'not installed'
    return 0
  fi
  printf 'installed%s %s enrolled: %s %s running: %s' "${RT_VERSION:+ v$RT_VERSION}" "$G_DOT" "$RT_ENROLLED" "$G_DOT" "$RT_RUNNING"
}

rt_as_display() {
  if [ "$OS" = macos ]; then
    printf 'cd / && sudo -u %s %s' "$RT_ACCOUNT" "$(shq "$RT_BIN")"
  else
    printf 'sudo -u %s storm-runtime' "$RT_ACCOUNT"
  fi
}

rt_enroll_display() {
  if [ "$OS" = macos ]; then
    printf '(cd / && sudo -u %s %s enroll)' "$RT_ACCOUNT" "$(shq "$RT_BIN")"
  else
    printf 'sudo -u %s storm-runtime enroll' "$RT_ACCOUNT"
  fi
}

runtime_plan() {
  local mode state
  mode=$(comp_mode runtime)
  state="$DATA_ROOT/state"
  if [ "$mode" = uninstall ]; then
    case "$OS" in
      linux)
        plan_add spin 1 "Stop the Runtime Host and remove the storm-runtime package (workspaces stay)" \
          "sudo systemctl disable --now storm-runtime\nsudo apt-get remove -y storm-runtime" runtime_uninstall
        ;;
      macos)
        plan_add spin 1 "Remove the Runtime Host's LaunchDaemon and binary (workspaces stay)" \
          "sudo $(shq "$RT_BIN") uninstall${RT_PURGE:+ --purge}" runtime_uninstall
        ;;
    esac
    return 0
  fi
  if [ "$mode" != keep ]; then
    case "$OS" in
      linux)
        if [ -z "$OPT_FROM_DIR" ]; then apt_plan_repo; fi
        plan_add spin 1 "Install the storm-runtime package" "$(apt_install_display runtime "$mode")" runtime_install package "$mode"
        if [ "$mode" != install ] && [ "$RT_ENROLLED" != no ]; then
          plan_add spin 1 "Restart the Runtime Host on the new files" "sudo systemctl try-restart storm-runtime" runtime_install restart
        fi
        ;;
      macos)
        plan_fetch runtime
        plan_add spin 1 "Install the Runtime Host (a LaunchDaemon as $RT_ACCOUNT; workspaces shared with $ME)" \
          "sudo ./storm-runtime install --operator $ME" runtime_install daemon
        ;;
    esac
  fi
  case "$RT_ENROLL_ACTION" in
    enroll | reenroll)
      if [ "$LOCAL_SERVER" = 1 ]; then
        plan_add spin 1 "Enroll this runtime with the server on this $MACHINE" \
          "$(srv_owner_prefix "$state")$(srv_bin_display) host-enrollment --state $(shq "$state") --url http://127.0.0.1:$PORT | $(rt_enroll_display)" \
          runtime_configure enroll-local
      else
        plan_add fg 1 "Enroll this runtime with your Storm server (paste its enrollment string)" \
          "printf '%s\\\\n' <the enrollment string> | $(rt_enroll_display)" runtime_configure enroll-paste
      fi
      if [ "$OS" = linux ]; then
        plan_add spin 1 "Start the Runtime Host" "sudo systemctl enable --now storm-runtime" runtime_configure start
      fi
      ;;
  esac
  if [ "$RT_ENROLL_ACTION" != skip ] || [ "$RT_ENROLLED" = yes ]; then
    plan_add spin 1 "Check the runtime reaches its server" "$(rt_as_display) check" runtime_verify
  fi
  return 0
}

runtime_install() {
  case "$1" in
    package) apt_install_pkg storm-runtime "${2:-install}" ;;
    restart) as_root systemctl try-restart storm-runtime ;;
    daemon)
      if [ ! -x "$WORK_DIR/runtime/storm-runtime" ]; then
        echo "the runtime tarball has no storm-runtime"
        return 1
      fi
      (cd "$WORK_DIR/runtime" && as_root ./storm-runtime install --operator "$ME")
      ;;
  esac
}

# The enrollment string arrives on stdin; never in argv.
rt_enroll_stdin() {
  case "$OS" in
    macos) (cd / && run_as "$RT_ACCOUNT" "$RT_BIN" enroll) ;;
    *) run_as "$RT_ACCOUNT" storm-runtime enroll ;;
  esac
}

rt_enrolled_now() {
  as_root test -f "$RT_STATE/host.json"
}

enroll_string_ok() {
  case "$1" in
    *[[:space:]]*) return 1 ;;
    storm-enroll:v1:?*) return 0 ;;
  esac
  return 1
}

runtime_configure() {
  case "$1" in
    enroll-local) runtime_enroll_local ;;
    enroll-paste) runtime_enroll_paste ;;
    start)
      if ! rt_enrolled_now; then
        echo "not enrolled, so not started"
        return "$STEP_SKIPPED"
      fi
      as_root systemctl enable --now storm-runtime
      ;;
  esac
}

runtime_enroll_local() {
  local state="$DATA_ROOT/state"
  if [ "$RT_ENROLL_ACTION" = enroll ] && rt_enrolled_now; then
    echo "already enrolled; kept"
    return 0
  fi
  if ! srv_has_account "$state"; then
    echo "the server has no account yet: set one up (pair or password), then re-run the installer and choose Enroll this runtime"
    return 1
  fi
  # If host-enrollment fails it prints nothing, and enroll fails on that.
  if ! srv_as_owner "$state" "$(srv_bin)" host-enrollment --state "$state" --url "http://127.0.0.1:$PORT" | rt_enroll_stdin; then
    echo "enrollment failed"
    return 1
  fi
}

runtime_enroll_paste() {
  local s="" rc
  if [ "$RT_ENROLL_ACTION" = enroll ] && rt_enrolled_now; then
    ui_ok "This runtime is already enrolled; kept"
    return 0
  fi
  if [ "$INTERACTIVE" != 1 ]; then
    if [ "$OPT_ENROLL_STDIN" != 1 ]; then
      ui_note "Not enrolled: pass --enroll-from-stdin with the string on stdin, or re-run interactively."
      return "$STEP_SKIPPED"
    fi
    IFS= read -r s || :
    if ! enroll_string_ok "$s"; then
      s=""
      ui_fail "stdin didn't hold an enrollment string (one line starting storm-enroll:v1:)"
      return 1
    fi
    if printf '%s\n' "$s" | logged rt_enroll_stdin; then rc=0; else rc=$?; fi
    s=""
    [ "$rc" = 0 ] && ui_ok "Enrolled"
    return "$rc"
  fi
  ui_info "To get an enrollment string: in the Storm app open Agents › Hosts › Enroll a host"
  ui_info "and copy the string it shows. Paste it here; it isn't shown or logged."
  ui_secret_pipe "Enrollment string (Enter alone skips)" enroll_string_ok \
    "That isn't an enrollment string: those start with storm-enroll:v1:" logged rt_enroll_stdin
  rc=$?
  case "$rc" in
    0) ui_ok "Enrolled" ;;
    "$STEP_SKIPPED") ;;
    *) ui_fail "The runtime refused the string; the log says why: $LOG_FILE" ;;
  esac
  return "$rc"
}

runtime_verify() {
  local i=0
  if ! rt_enrolled_now; then
    echo "not enrolled; nothing to check"
    return "$STEP_SKIPPED"
  fi
  while :; do
    if [ "$OS" = macos ]; then
      (cd / && run_as "$RT_ACCOUNT" "$RT_BIN" check) && return 0
    else
      run_as "$RT_ACCOUNT" storm-runtime check && return 0
    fi
    i=$((i + 1))
    [ "$i" -ge 3 ] && return 1
    sleep "$POLL_SLEEP"
  done
}

runtime_uninstall() {
  case "$OS" in
    linux)
      as_root systemctl disable --now storm-runtime || :
      as_root env DEBIAN_FRONTEND=noninteractive apt-get remove -y storm-runtime
      ;;
    macos)
      if [ -n "${RT_PURGE:-}" ]; then
        as_root "$RT_BIN" uninstall --purge
      else
        as_root "$RT_BIN" uninstall
      fi
      ;;
  esac
}

# ------------------------------------------------------------ component: agents

agents_available() {
  REASON=""
  case "$OS" in
    macos | linux) return 0 ;;
  esac
  REASON="the installer supports macOS and Debian/Ubuntu Linux"
  return 1
}

agents_find() {
  local d
  for d in $AGENT_BIN_DIRS; do
    if [ -x "$d/$1" ]; then
      printf '%s' "$d/$1"
      return 0
    fi
  done
  return 1
}

agents_detect() {
  local p
  CLAUDE_BIN="" OPENCODE_BIN="" CLAUDE_IN_HOME="" OPENCODE_IN_HOME="" BREW_BIN=""
  AG_TOKEN=unknown AG_PROVIDERS=unknown
  CLAUDE_BIN=$(agents_find claude) || CLAUDE_BIN=""
  OPENCODE_BIN=$(agents_find opencode) || OPENCODE_BIN=""
  for p in "$HOME/.local/bin/claude" "$HOME/.claude/local/claude" "$HOME/.npm-global/bin/claude"; do
    if [ -x "$p" ]; then
      CLAUDE_IN_HOME=$p
      break
    fi
  done
  for p in "$HOME/.opencode/bin/opencode" "$HOME/.local/bin/opencode"; do
    if [ -x "$p" ]; then
      OPENCODE_IN_HOME=$p
      break
    fi
  done
  if [ "$OS" = macos ]; then
    BREW_BIN=$(command -v brew 2>/dev/null) || BREW_BIN=""
    for p in /opt/homebrew/bin/brew /usr/local/bin/brew; do
      if [ -z "$BREW_BIN" ] && [ -x "$p" ]; then BREW_BIN=$p; fi
    done
  fi
  if [ -r "$RT_CONFIG" ]; then
    if grep -Eq '^[[:space:]]*\[\[providers\]\]' "$RT_CONFIG"; then AG_PROVIDERS=yes; else AG_PROVIDERS=no; fi
  fi
  if [ "$OPT_DRY_RUN" != 1 ] && [ "$RT_INSTALLED" = 1 ] && can_sudo_quietly; then
    if probe as_root -n test -f "$RT_STATE/claude.env"; then AG_TOKEN=yes; else AG_TOKEN=no; fi
  fi
  return 0
}

agents_status_line() {
  printf 'Claude Code: %s %s OpenCode: %s %s token: %s %s providers listed: %s' "${CLAUDE_BIN:-not where the host can run it}" "$G_DOT" \
    "${OPENCODE_BIN:-not where the host can run it}" "$G_DOT" "$AG_TOKEN" "$G_DOT" "$AG_PROVIDERS"
}

agents_plan() {
  local mode env_file restart home_note brew_hint=""
  mode=$(comp_mode agents)
  [ "$mode" = uninstall ] && return 0
  if [ "$RT_INSTALLED" != 1 ] && ! in_sel runtime; then
    plan_note "Agent CLIs: skipped; they're for the Runtime Host, which isn't installed or selected."
    return 0
  fi
  home_note="CLIs in your home directory are out of the host account's reach, by design."
  [ "$OS" = macos ] && brew_hint=" (Homebrew: https://brew.sh)"
  if [ -z "$CLAUDE_BIN" ]; then
    if [ "$OS" = macos ] && [ -n "$BREW_BIN" ]; then
      plan_add spin 0 "Install Claude Code with Homebrew, where $RT_ACCOUNT can run it" "brew install --cask claude-code" agents_install claude
    else
      plan_note "Claude Code isn't installed where $RT_ACCOUNT can run it ($AGENT_BIN_DIRS); install it there$brew_hint. $home_note"
    fi
    [ -n "$CLAUDE_IN_HOME" ] && plan_note "Found $CLAUDE_IN_HOME, which $RT_ACCOUNT can't run. $home_note"
  fi
  if [ -z "$OPENCODE_BIN" ]; then
    if [ "$OS" = macos ] && [ -n "$BREW_BIN" ]; then
      plan_add spin 0 "Install OpenCode with Homebrew, where $RT_ACCOUNT can run it" "brew install opencode" agents_install opencode
    else
      plan_note "OpenCode isn't installed where $RT_ACCOUNT can run it ($AGENT_BIN_DIRS); install it there$brew_hint. $home_note"
    fi
    [ -n "$OPENCODE_IN_HOME" ] && plan_note "Found $OPENCODE_IN_HOME, which $RT_ACCOUNT can't run. $home_note"
  fi
  if [ "$INTERACTIVE" != 1 ]; then
    plan_note "Agent logins need a terminal: re-run the installer, choose Custom… and tick Agent CLIs."
    return 0
  fi
  env_file="$RT_STATE/claude.env"
  plan_add fg 1 "Log Claude Code in for the host (a token from claude setup-token)" \
    "claude setup-token   # you, in another window\nprintf 'CLAUDE_CODE_OAUTH_TOKEN=%s\\\\n' <token> | (cd / && sudo -u $RT_ACCOUNT sh -c 'umask 077; cat > $(shq "$env_file")')" \
    agents_configure claude-token
  plan_add fg 1 "Name the providers in runtime.toml (only if it lists none)" \
    "printf '[[providers]] id = \"claude-code\" env_file = …' | sudo tee -a $(shq "$RT_CONFIG")" agents_configure providers
  if [ "$OS" = macos ]; then
    restart="sudo launchctl kickstart -k system/dev.storm.runtime"
  else
    restart="sudo systemctl try-restart storm-runtime"
  fi
  plan_add fg 1 "Restart the Runtime Host if its logins changed" "$restart" agents_configure restart
  plan_add fg 1 "Log OpenCode in for the host" "cd / && sudo -u $RT_ACCOUNT -H opencode auth login" agents_configure opencode-login
  return 0
}

agents_install() {
  case "$1" in
    claude) "$BREW_BIN" install --cask claude-code ;;
    opencode) "$BREW_BIN" install opencode ;;
  esac
}

claude_token_ok() {
  case "$1" in
    *[[:space:]]*) return 1 ;;
    sk-ant-?*) return 0 ;;
  esac
  return 1
}

# The token arrives on stdin and leaves on stdin: never in an argument.
write_claude_env() {
  local target="$RT_STATE/claude.env"
  {
    printf 'CLAUDE_CODE_OAUTH_TOKEN='
    cat
  } | (
    # shellcheck disable=SC2016
    cd / && run_as "$RT_ACCOUNT" sh -c 'umask 077; cat > "$1"' sh "$target"
  )
}

providers_block() {
  printf '\n[[providers]]\nid = "claude-code"\nenv_file = "%s"\n\n[[providers]]\nid = "opencode"\n\n[[providers]]\nid = "shell"\n' "$RT_STATE/claude.env"
}

agents_configure() {
  case "$1" in
    claude-token) agents_claude_token ;;
    providers) agents_providers ;;
    restart) agents_restart ;;
    opencode-login) agents_opencode_login ;;
  esac
}

agents_verify() {
  agents_detect
  [ -n "$CLAUDE_BIN" ] || [ -n "$OPENCODE_BIN" ]
}

agents_claude_token() {
  local rc
  [ "$INTERACTIVE" = 1 ] || return "$STEP_SKIPPED"
  if as_root test -f "$RT_STATE/claude.env"; then
    ui_confirm "Claude Code already has a token for the host. Replace it?" n || return "$STEP_SKIPPED"
  fi
  ui_info "Claude Code's own login needs a Keychain the host's account doesn't have,"
  ui_info "so the host uses a long-lived token instead:"
  ui_info "  1. In another terminal window, run:  ${C_BOLD}claude setup-token${C_RESET}"
  ui_info "  2. Paste the token here. It isn't shown, logged, or put on a command line."
  ui_secret_pipe "Claude Code token (Enter alone skips)" claude_token_ok \
    "That isn't a Claude Code token: those start with sk-ant-" write_claude_env
  rc=$?
  case "$rc" in
    0)
      AGENTS_CHANGED=1
      ui_ok "Saved for $RT_ACCOUNT in $RT_STATE/claude.env (mode 0600)"
      ;;
    "$STEP_SKIPPED") ;;
    *) ui_fail "Couldn't write $RT_STATE/claude.env" ;;
  esac
  return "$rc"
}

agents_providers() {
  if as_root grep -Eq '^[[:space:]]*\[\[providers\]\]' "$RT_CONFIG" 2>/dev/null; then
    ui_ok "runtime.toml already lists its providers; left as it is"
    return 0
  fi
  if ! as_root test -f "$RT_STATE/claude.env"; then
    ui_note "No Claude Code token yet, so runtime.toml keeps its defaults"
    return "$STEP_SKIPPED"
  fi
  providers_block | as_root tee -a "$RT_CONFIG" >/dev/null || return 1
  AGENTS_CHANGED=1
  ui_ok "Listed claude-code (with the token), opencode and shell in $RT_CONFIG"
}

agents_restart() {
  [ "${AGENTS_CHANGED:-0}" = 1 ] || return "$STEP_SKIPPED"
  rt_enrolled_now || return "$STEP_SKIPPED"
  if [ "$OS" = macos ]; then
    as_root launchctl kickstart -k system/dev.storm.runtime
  else
    as_root systemctl try-restart storm-runtime
  fi
}

agents_opencode_login() {
  local oc=$OPENCODE_BIN
  [ -n "$oc" ] || oc=$(agents_find opencode) || oc=""
  if [ -z "$oc" ]; then
    ui_note "OpenCode isn't installed where $RT_ACCOUNT can run it; skipped"
    return "$STEP_SKIPPED"
  fi
  ui_confirm "Log OpenCode in for the host now? It asks for a provider, then its key." y || return "$STEP_SKIPPED"
  (cd / && run_as -H "$RT_ACCOUNT" "$oc" auth login) <&"$UI_IN" >&"$UI_FD" 2>&"$UI_FD"
}

# The CLIs are yours (Homebrew's); the host's logins live in its state.
agents_uninstall() { return 0; }

# ------------------------------------------------------------ component: app

app_available() {
  local name
  REASON=""
  case "$OS" in
    macos)
      if [ "$ARCH" != arm64 ]; then
        REASON="Storm.app is built for Apple silicon only; on this Intel Mac use the web client"
        return 1
      fi
      release_ok || return 1
      name=$(asset_name app-macos "$REL_VERSION")
      checksum_has "$name" && return 0
      REASON="$name isn't in v$REL_VERSION's checksums.txt"
      return 1
      ;;
    linux) return 0 ;;
  esac
  REASON="the installer supports macOS and Debian/Ubuntu Linux"
  return 1
}

app_detect() {
  APP_INSTALLED=0 APP_VERSION=""
  [ "$OS" = macos ] || return 0
  [ -d "$APP_DIR/Storm.app" ] || return 0
  APP_INSTALLED=1
  APP_VERSION=$(probe defaults read "$APP_DIR/Storm.app/Contents/Info" CFBundleShortVersionString) || APP_VERSION=""
  return 0
}

app_status_line() {
  if [ "$OS" != macos ]; then
    printf 'none for Linux: use the web client'
  elif [ "$APP_INSTALLED" = 1 ]; then
    printf 'installed%s (%s/Storm.app)' "${APP_VERSION:+ v$APP_VERSION}" "$APP_DIR"
  else
    printf 'not installed'
  fi
}

# /Applications if we can write there (admins can), else ~/Applications.
app_dest() {
  if [ -w "$APP_DIR" ]; then printf '%s' "$APP_DIR"; else printf '%s' "$HOME/Applications"; fi
}

app_plan() {
  local mode dest disp
  mode=$(comp_mode app)
  if [ "$OS" != macos ]; then
    [ "$mode" = uninstall ] && return 0
    if [ "$LOCAL_SERVER" = 1 ]; then
      plan_add fg 0 "Show the web client's address (there's no Linux desktop app yet)" "$(srv_bin_display) qr $(web_url)" app_configure web
    else
      plan_note "Desktop app: there's none for Linux yet; use your Storm server's web client."
    fi
    return 0
  fi
  if [ "$mode" = uninstall ]; then
    plan_add spin 0 "Remove Storm.app from $APP_DIR" "rm -rf $(shq "$APP_DIR/Storm.app")" app_uninstall
    return 0
  fi
  [ "$mode" = keep ] && return 0
  dest=$(app_dest)
  plan_fetch app
  disp="ditto -x -k $(asset_name app-macos "$REL_VERSION") $(shq "$dest")"
  [ -d "$dest/Storm.app" ] && disp="rm -rf $(shq "$dest/Storm.app")\n$disp"
  plan_add spin 0 "Install Storm.app into $dest" "$disp" app_install "$dest"
  return 0
}

app_install() {
  local dest=$1 zip x
  zip="$WORK_DIR/app/$(asset_name app-macos "$REL_VERSION")"
  x="$WORK_DIR/app/x"
  if [ ! -f "$zip" ]; then
    echo "missing $zip"
    return 1
  fi
  rm -rf "$x" && mkdir -p "$x" || return 1
  ditto -x -k "$zip" "$x" || return 1
  if [ ! -d "$x/Storm.app" ]; then
    echo "the zip has no Storm.app"
    return 1
  fi
  mkdir -p "$dest" || return 1
  if [ -d "$dest/Storm.app" ]; then rm -rf "$dest/Storm.app" || return 1; fi
  ditto "$x/Storm.app" "$dest/Storm.app"
}

app_configure() {
  case "$1" in
    web)
      ui_info "There's no Linux desktop app yet. Open the web client in a browser:"
      ui_info "  ${C_BOLD}$(web_url)${C_RESET}"
      show_qr "$(web_url)"
      ;;
  esac
}

app_verify() {
  [ "$OS" != macos ] || [ -d "$(app_dest)/Storm.app" ]
}

app_uninstall() {
  rm -rf "$APP_DIR/Storm.app"
}

# ------------------------------------------------------------ component: mobile

mobile_available() {
  REASON=""
  return 0
}

mobile_detect() { return 0; }

mobile_status_line() { printf 'pair from the app or the web client'; }

mobile_apk_url() {
  local name
  [ -z "$REL_ERROR" ] || return 1
  name=$(asset_name apk "$REL_VERSION")
  checksum_has "$name" || return 1
  release_url "$name"
}

mobile_plan() {
  local mode disp="" apk
  mode=$(comp_mode mobile)
  [ "$mode" = uninstall ] && return 0
  if apk=$(mobile_apk_url); then disp="$(srv_bin_display) qr $apk\n"; fi
  disp="$disp$(srv_bin_display) qr $(web_url)"
  plan_add fg 0 "Show how to connect your phone or tablet" "$disp" mobile_configure
  return 0
}

mobile_configure() {
  local apk
  ui_line ""
  ui_line "  ${C_BOLD}Your phone or tablet${C_RESET}"
  if [ "$LOCAL_SERVER" = 1 ] && [ "${SRV_ACCOUNT:-}" != yes ]; then
    ui_info "This server has no account yet: its pairing QR (choose Pair a device) is the way in."
  elif [ "$LOCAL_SERVER" = 1 ]; then
    ui_info "To add a device, open Settings › Devices in the app or the web client."
  fi
  if apk=$(mobile_apk_url); then
    ui_info "Android: scan to download the app ($apk)"
    show_qr "$apk"
  else
    ui_note "Android: this release has no APK; see $STORM_REPO_URL/releases"
  fi
  if [ "$LOCAL_SERVER" = 1 ]; then
    ui_info "iPhone and iPad use the web client: $(web_url)"
    show_qr "$(web_url)"
    ui_note "The phone must be on the same network as this $MACHINE."
  else
    ui_info "iPhone and iPad use the web client your Storm server serves."
  fi
  return 0
}

mobile_install() { return 0; }
mobile_verify() { return 0; }
mobile_uninstall() { return 0; }

# ------------------------------------------------------------ component: relay

relay_available() {
  REASON="coming soon — reach this machine from anywhere"
  return 1
}
relay_detect() { return 0; }
relay_status_line() { printf 'coming soon'; }
relay_plan() { return 0; }
relay_install() { return 1; }
relay_configure() { return 1; }
relay_verify() { return 1; }
relay_uninstall() { return 0; }

# ------------------------------------------------------------ apt (Linux)

apt_source_line() {
  printf 'deb [signed-by=%s] %s %s main' "${KEYRING_PATH#"${STORM_TEST_SYSROOT:-}"}" "$APT_ROOT" "$CHANNEL"
}

apt_plan_repo() {
  plan_once apt-repo || return 0
  plan_add spin 1 "Add Storm's apt repository (signing key and source) and refresh apt" \
    "curl -fsSL -o storm-archive-keyring.gpg $KEYRING_URL\nsudo tee $KEYRING_PATH < storm-archive-keyring.gpg >/dev/null\nprintf '$(apt_source_line)\\\\n' | sudo tee $LIST_PATH >/dev/null\nsudo apt-get update" \
    apt_setup
}

# The same key and source the apt bootstrap always wrote.
apt_setup() {
  local key="$WORK_DIR/storm-archive-keyring.gpg"
  # A binary OpenPGP keyring (not ASCII armor): apt's signed-by expects that.
  if ! fetch_url "$KEYRING_URL" >"$key" || [ ! -s "$key" ]; then
    echo "couldn't download $KEYRING_URL"
    return 1
  fi
  as_root mkdir -p "$KEYRING_DIR" "$(dirname "$LIST_PATH")" || return 1
  as_root tee "$KEYRING_PATH" <"$key" >/dev/null || return 1
  as_root chmod 0644 "$KEYRING_PATH" || return 1
  printf '%s\n' "$(apt_source_line)" | as_root tee "$LIST_PATH" >/dev/null || return 1
  as_root chmod 0644 "$LIST_PATH" || return 1
  as_root env DEBIAN_FRONTEND=noninteractive apt-get update -y
}

apt_install_display() {
  local reinstall="" deb
  [ "$2" = repair ] && reinstall="--reinstall "
  if [ -n "$OPT_FROM_DIR" ]; then
    deb=$(deb_in_dir "storm-$1") || deb="$OPT_FROM_DIR/storm-$1_<version>.deb"
    printf '%s %s   # must match checksums.txt\ncd %s && sudo apt-get install -y %s./%s' "$(sha_tool_display)" "${deb##*/}" \
      "$(shq "$OPT_FROM_DIR")" "$reinstall" "${deb##*/}"
  else
    printf 'sudo apt-get install -y %sstorm-%s' "$reinstall" "$1"
  fi
}

apt_install_pkg() {
  local pkg=$1 mode=$2 deb
  local -a extra
  extra=()
  [ "$mode" = repair ] && extra=(--reinstall)
  if [ -n "$OPT_FROM_DIR" ]; then
    if ! deb=$(deb_in_dir "$pkg"); then
      echo "there's no ${pkg}_*.deb in $OPT_FROM_DIR"
      return 1
    fi
    verify_file "$deb" "${deb##*/}" || return 1
    (cd "$OPT_FROM_DIR" && as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y ${extra[@]+"${extra[@]}"} "./${deb##*/}")
  else
    as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y ${extra[@]+"${extra[@]}"} "$pkg"
  fi
}

# ------------------------------------------------------------ downloads (macOS)

plan_fetch() {
  local c=$1 name disp
  name=$(asset_name "$c-macos" "$REL_VERSION")
  if [ -n "$OPT_FROM_DIR" ]; then
    disp="cp $(shq "$OPT_FROM_DIR/$name") ."
  else
    disp="curl -fL -o $name $(release_url "$name")"
  fi
  disp="$disp\n$(sha_tool_display) $name   # must match checksums.txt"
  case "$name" in *.tar.gz) disp="$disp\ntar -xzf $name" ;; esac
  plan_add spin 0 "Download and verify $name" "$disp" fetch_asset "$name" "$c"
}

# fetch_asset NAME DIR: into $WORK_DIR/DIR, verified; a tarball is extracted.
fetch_asset() {
  local name=$1 dir="$WORK_DIR/$2"
  mkdir -p "$dir" || return 1
  if [ -n "$OPT_FROM_DIR" ]; then
    cp "$OPT_FROM_DIR/$name" "$dir/$name" || return 1
  else
    echo "downloading $(release_url "$name")"
    download_to "$(release_url "$name")" "$dir/$name" || return 1
  fi
  if ! verify_file "$dir/$name" "$name"; then
    rm -f "$dir/$name"
    return 1
  fi
  case "$name" in
    *.tar.gz) tar -xzf "$dir/$name" -C "$dir" || return 1 ;;
  esac
  return 0
}

# ------------------------------------------------------------ the plan

plan_reset() {
  PLAN_N=0 PLAN_NOTES_N=0 PLAN_KEYS=" " AGENTS_CHANGED=0
  PLAN_KIND=() PLAN_SUDO=() PLAN_DESC=() PLAN_CMD=() PLAN_RUN=() PLAN_RESULT=() PLAN_NOTES=()
}

# plan_add KIND SUDO DESC DISPLAY FN [ARG...]
#   KIND spin: runs in the background with a spinner, its output in the log.
#   KIND fg:   runs in the foreground, and may ask questions.
#   SUDO 1 marks a step that runs a command through sudo.
#   DISPLAY is what the plan screen shows ("\n" separates commands).
plan_add() {
  local kind=$1 sudo=$2 desc=$3 disp=$4
  shift 4
  PLAN_N=$((PLAN_N + 1))
  PLAN_KIND[PLAN_N]=$kind
  PLAN_SUDO[PLAN_N]=$sudo
  PLAN_DESC[PLAN_N]=$desc
  PLAN_CMD[PLAN_N]=$disp
  PLAN_RUN[PLAN_N]=$(printf '%q ' "$@")
  PLAN_RESULT[PLAN_N]=pending
}

plan_once() {
  case "$PLAN_KEYS" in *" $1 "*) return 1 ;; esac
  PLAN_KEYS="$PLAN_KEYS$1 "
  return 0
}

plan_note() {
  PLAN_NOTES_N=$((PLAN_NOTES_N + 1))
  PLAN_NOTES[PLAN_NOTES_N]=$1
}

plan_sudo_count() {
  local i=1 n=0
  while [ "$i" -le "$PLAN_N" ]; do
    [ "${PLAN_SUDO[$i]}" = 1 ] && n=$((n + 1))
    i=$((i + 1))
  done
  printf '%s' "$n"
}

plan_has_sudo() {
  [ "$(plan_sudo_count)" -gt 0 ]
}

# plan_print: the plan screen on stdout. Every command; the sudo ones marked.
plan_print() {
  local i=1 tag line
  printf '\n  %sPlan%s: %s steps, %s with sudo (marked [sudo]). Nothing has changed yet.\n\n' \
    "$C_BOLD" "$C_RESET" "$PLAN_N" "$(plan_sudo_count)"
  while [ "$i" -le "$PLAN_N" ]; do
    tag="      "
    [ "${PLAN_SUDO[$i]}" = 1 ] && tag="${C_YELLOW}[sudo]${C_RESET}"
    printf '  %2d. %s %s\n' "$i" "$tag" "${PLAN_DESC[$i]}"
    printf '%b\n' "${PLAN_CMD[$i]}" | while IFS= read -r line; do
      printf '             %s$ %s%s\n' "$C_DIM" "$line" "$C_RESET"
    done
    i=$((i + 1))
  done
  if [ "$PLAN_NOTES_N" -gt 0 ]; then
    printf '\n  Notes:\n'
    i=1
    while [ "$i" -le "$PLAN_NOTES_N" ]; do
      printf '   - %s\n' "${PLAN_NOTES[$i]}"
      i=$((i + 1))
    done
  fi
  printf '\n  Log: %s\n' "$LOG_PATH"
}

plan_print_plain() {
  local C_BOLD="" C_RESET="" C_DIM="" C_YELLOW=""
  plan_print
}

# Run every step in order. A failure offers Retry / Skip / Quit; without a
# terminal it stops. STORM_STEP tells what the step runs which step it is.
plan_execute() {
  local i=1 rc
  PLAN_FAILED=0
  while [ "$i" -le "$PLAN_N" ]; do
    STORM_STEP=$i
    export STORM_STEP
    log "step $i: ${PLAN_DESC[$i]}"
    log "  \$ $(printf '%b' "${PLAN_CMD[$i]}" | tr '\n' ';')"
    if [ "${PLAN_SUDO[$i]}" = 1 ]; then sudo_keepalive >/dev/null 2>&1 || :; fi
    if [ "${PLAN_KIND[$i]}" = spin ]; then
      ui_spinner "${PLAN_DESC[$i]}" eval "${PLAN_RUN[$i]}"
      rc=$?
    else
      ui_line ""
      ui_line "  ${C_BOLD}${PLAN_DESC[$i]}${C_RESET}"
      eval "${PLAN_RUN[$i]}"
      rc=$?
    fi
    log "step $i: exit $rc"
    if [ "$rc" = 0 ]; then
      PLAN_RESULT[i]=ok
      i=$((i + 1))
      continue
    fi
    if [ "$rc" = "$STEP_SKIPPED" ]; then
      PLAN_RESULT[i]=skipped
      i=$((i + 1))
      continue
    fi
    PLAN_RESULT[i]=failed
    ui_note "The log has the details: $LOG_FILE"
    if [ "$LOG_FILE" != /dev/null ]; then
      tail -n 6 "$LOG_FILE" 2>/dev/null | sed 's/^/      /' >&"$UI_FD"
    fi
    if [ "$INTERACTIVE" != 1 ]; then
      PLAN_FAILED=1
      unset STORM_STEP
      return 1
    fi
    ui_menu "Step $i failed. What now?" "Retry|run it again" "Skip|carry on without it" "Quit|stop here" || UI_INDEX=3
    case "$UI_INDEX" in
      1) ;;
      2)
        PLAN_FAILED=1
        i=$((i + 1))
        ;;
      *)
        PLAN_FAILED=1
        unset STORM_STEP
        return 1
        ;;
    esac
  done
  unset STORM_STEP
  [ "$PLAN_FAILED" = 0 ]
}

local_server_now() {
  LOCAL_SERVER=0
  if in_sel server && [ "$(comp_mode server)" != uninstall ]; then LOCAL_SERVER=1; fi
  if [ "$SRV_INSTALLED" = 1 ] && ! { in_sel server && [ "$(comp_mode server)" = uninstall ]; }; then LOCAL_SERVER=1; fi
  return 0
}

# The steps for the selected components, in registry order.
plan_build() {
  local c
  plan_reset
  local_server_now
  for c in $STORM_COMPONENTS; do
    if in_sel "$c"; then comp_call "$c" plan; fi
  done
  apply_pending_notes
  return 0
}

# ------------------------------------------------------------ questions

# DATA_ROOT, PORT and ADVERTISE from the flags, what's installed, defaults.
settle_config() {
  DATA_ROOT=${OPT_DATA:-${SRV_DATA:-$DEFAULT_DATA}}
  PORT=${OPT_PORT:-${SRV_PORT:-$STORM_DEFAULT_PORT}}
  if [ -n "$OPT_ADDR" ]; then
    case "$OPT_ADDR" in
      *:*) ADVERTISE=$OPT_ADDR ;;
      *) ADVERTISE="$OPT_ADDR:$PORT" ;;
    esac
  else
    ADVERTISE="$(lan_addr):$PORT"
  fi
}

# Installed already? Then upgrade, repair or keep; without a terminal, keep
# (or upgrade, under `upgrade`).
choose_mode() {
  local c=$1 have want
  case "$c" in
    server | runtime | app) ;;
    *)
      set_mode "$c" install
      return 0
      ;;
  esac
  if [ "$c" = app ] && [ "$OS" != macos ]; then
    set_mode "$c" install
    return 0
  fi
  if ! comp_installed "$c"; then
    set_mode "$c" install
    return 0
  fi
  have=$(comp_version "$c")
  want=$REL_VERSION
  if ! comp_available "$c" || [ -z "$want" ]; then
    set_mode "$c" keep
    return 0
  fi
  if [ "$SUBCOMMAND" = upgrade ]; then
    if [ -z "$have" ] || ver_lt "$have" "$want"; then set_mode "$c" upgrade; else set_mode "$c" keep; fi
    return 0
  fi
  if [ "$INTERACTIVE" != 1 ]; then
    set_mode "$c" keep
    return 0
  fi
  if [ -n "$have" ] && ver_lt "$have" "$want"; then
    ui_menu "$(comp_label "$c") v$have is installed." \
      "Upgrade|to v$want" "Keep|leave v$have as it is" "Repair|reinstall v$want's files" || UI_INDEX=2
    case "$UI_INDEX" in
      1) set_mode "$c" upgrade ;;
      3) set_mode "$c" repair ;;
      *) set_mode "$c" keep ;;
    esac
  else
    ui_menu "$(comp_label "$c")${have:+ v$have} is installed." \
      "Keep|leave it as it is" "Repair|reinstall v$want's files" || UI_INDEX=1
    case "$UI_INDEX" in
      2) set_mode "$c" repair ;;
      *) set_mode "$c" keep ;;
    esac
  fi
}

choose_enroll_action() {
  RT_ENROLL_ACTION=skip
  in_sel runtime || return 0
  [ "$(comp_mode runtime)" = uninstall ] && return 0
  if [ "$RT_ENROLLED" = yes ]; then
    RT_ENROLL_ACTION=keep
    if [ "$INTERACTIVE" = 1 ]; then
      ui_menu "This runtime is already enrolled with a server." \
        "Keep it|nothing changes" \
        "Re-enroll|only after revoking it in the app (Agents › Hosts)" \
        "Skip|decide later" || UI_INDEX=1
      case "$UI_INDEX" in
        2) RT_ENROLL_ACTION=reenroll ;;
        3) RT_ENROLL_ACTION=skip ;;
      esac
    fi
    return 0
  fi
  if [ "$LOCAL_SERVER" = 1 ] || [ "$INTERACTIVE" = 1 ] || [ "$OPT_ENROLL_STDIN" = 1 ]; then
    RT_ENROLL_ACTION=enroll
  else
    if [ "$RT_ENROLLED" = unknown ]; then
      plan_note_pending "Runtime: if it isn't enrolled yet, pass --enroll-from-stdin with its enrollment string on stdin, or re-run interactively."
    else
      plan_note_pending "Runtime: not enrolled. Pass --enroll-from-stdin with its enrollment string on stdin, or re-run interactively."
    fi
  fi
  return 0
}

# Notes decided before the plan exists wait here for plan_build.
plan_note_pending() {
  PENDING_NOTES="${PENDING_NOTES:+$PENDING_NOTES
}$1"
}

apply_pending_notes() {
  local n
  [ -n "${PENDING_NOTES:-}" ] || return 0
  while IFS= read -r n; do
    [ -n "$n" ] && plan_note "$n"
  done <<EOF
$PENDING_NOTES
EOF
  return 0
}

ask_questions() {
  local c
  PENDING_NOTES=""
  for c in $SEL; do choose_mode "$c"; done
  settle_config
  if [ "$INTERACTIVE" = 1 ] && in_sel server && [ "$(comp_mode server)" = install ]; then
    ui_head "  A few questions (Enter keeps the default)"
    if [ -z "$OPT_DATA" ]; then
      ui_note "Your vaults, the server's state and its backups live here."
      ui_input "Data folder" "$DATA_ROOT"
      case "$UI_RESULT" in
        \~/*) UI_RESULT="$HOME/${UI_RESULT#\~/}" ;;
      esac
      DATA_ROOT=$UI_RESULT
    fi
    if [ -z "$OPT_PORT" ]; then
      while :; do
        ui_input "Port" "$PORT"
        if valid_port "$UI_RESULT"; then
          PORT=$((10#$UI_RESULT))
          break
        fi
        ui_warn "A port is a number from 1 to 65535."
      done
      [ -z "$OPT_ADDR" ] && ADVERTISE="${ADVERTISE%:*}:$PORT"
    fi
  fi
  if [ "$INTERACTIVE" = 1 ] && [ -z "$OPT_ADDR" ] && { in_sel server || in_sel mobile; } &&
    { [ "$(comp_mode server)" = install ] || [ "$SRV_ACCOUNT" != yes ]; }; then
    ui_note "Phones and other machines reach the server at this address (this $MACHINE's on the network)."
    ui_input "Address" "$ADVERTISE"
    case "$UI_RESULT" in
      *:*) ADVERTISE=$UI_RESULT ;;
      *) ADVERTISE="$UI_RESULT:$PORT" ;;
    esac
  fi
  local_server_now
  choose_enroll_action
}

# Drop what can't be installed here, and say why.
filter_selection() {
  local c out=""
  for c in $SEL; do
    if comp_available "$c" || comp_installed "$c"; then
      out="${out:+$out }$c"
    else
      ui_warn "$(comp_label "$c") is unavailable: $(comp_reason "$c")"
    fi
  done
  SEL=$out
}

choose_components() {
  local c state note v
  local -a items
  items=()
  for c in $STORM_COMPONENTS; do
    note=$(comp_blurb "$c")
    if comp_available "$c" || comp_installed "$c"; then
      if in_sel "$c"; then state=on; else state=off; fi
      if comp_installed "$c"; then
        v=$(comp_version "$c")
        note="installed${v:+ v$v} $G_DOT $note"
      fi
    else
      state=disabled
      note=$(comp_reason "$c")
    fi
    items[${#items[@]}]="$c|$(comp_label "$c")|$state|$note"
  done
  ui_checklist "Pick components" "${items[@]}" || return 1
  SEL=$UI_RESULT
  [ -n "$SEL" ]
}

# ------------------------------------------------------------ screens and flows

status_lines() {
  local c
  for c in server runtime agents app relay; do
    printf '  %-14s %s\n' "$(comp_label "$c")" "$(comp_call "$c" status_line)"
  done
}

status_screen() {
  printf '\n  Storm on this %s (%s, %s)%s\n\n' "$MACHINE" "$OS_LABEL" "$ARCH" "${REL_VERSION:+ $G_DOT latest release v$REL_VERSION}"
  status_lines
  if [ "$SRV_INSTALLED" = 1 ]; then printf '  %-14s %s\n' "Web client" "$(web_url)"; fi
  printf '  %-14s %s\n\n' "Install log" "$LOG_PATH"
  if [ "$SRV_ACCOUNT" = unknown ] || [ "$RT_ENROLLED" = unknown ]; then
    printf '  "unknown" needs sudo to tell: run sudo -v first to see it.\n\n'
  fi
}

refuse_no_tty() {
  cat >&2 <<EOF
install.sh: there's no terminal to ask questions on, and --yes wasn't given.
Say what to set up, for example:
  curl -fsSL $STORM_INSTALL_URL | sh -s -- --yes --everything
  curl -fsSL $STORM_INSTALL_URL | sh -s -- --yes --server --app --data ~/Storm --port 8484
  sh install.sh --yes --runtime --enroll-from-stdin < enrollment.txt
  sh install.sh --dry-run --everything     # show the plan, change nothing
  sh install.sh status
Run it with --help for every option.
EOF
}

first_screen() {
  ui_header
  ui_line ""
  status_lines >&"$UI_FD"
  ui_menu "What would you like to set up on this $MACHINE?" \
    "Everything|Server, runtime, app — and pair your phone" \
    "Server and app|Your notes on this $MACHINE, reachable from your devices" \
    "Runtime only|Run coding agents here for a Storm server elsewhere" \
    "Custom…|Pick components" \
    "-" \
    "Status|What's installed and running" \
    "Pair a device|Pairing QR, the Android app, the web address" \
    "Enroll this runtime|Connect this runtime to a Storm server" \
    "Uninstall|Remove components; notes and workspaces stay" || return 1
  case "$UI_INDEX" in
    1) SEL=$(preset_components everything) ;;
    2) SEL=$(preset_components server-app) ;;
    3) SEL=$(preset_components runtime) ;;
    4)
      SEL=""
      choose_components || return 1
      ;;
    6) FIRST_ACTION=status ;;
    7) FIRST_ACTION=pair ;;
    8) FIRST_ACTION=enroll ;;
    9) FIRST_ACTION=uninstall ;;
  esac
  return 0
}

# Show the plan, ask, run it. 20 means "change components".
plan_confirm_run() {
  local line
  if [ "$OPT_DRY_RUN" = 1 ]; then
    plan_print
    printf '\n  Dry run: nothing was changed.\n'
    return 0
  fi
  plan_print >&"$UI_FD"
  if [ "$PLAN_N" = 0 ]; then
    ui_info "Nothing to do."
    return 0
  fi
  if [ "$INTERACTIVE" = 1 ]; then
    ui_menu "Go ahead?" "Install|run these $PLAN_N steps" "Change components|back to the checklist" "Quit|change nothing" || UI_INDEX=3
    case "$UI_INDEX" in
      1) ;;
      2) return 20 ;;
      *)
        ui_info "Nothing was changed."
        return 0
        ;;
    esac
  fi
  sudo_prime || return 1
  plan_print_plain | while IFS= read -r line; do log "$line"; done
  plan_execute
}

flow_install() {
  local rc
  if [ -n "$OPT_COMPONENTS" ]; then
    SEL=$OPT_COMPONENTS
  elif [ "$INTERACTIVE" = 1 ]; then
    FIRST_ACTION=""
    first_screen || return 0
    case "$FIRST_ACTION" in
      status)
        settle_config
        status_screen >&"$UI_FD"
        return 0
        ;;
      pair)
        flow_pair
        return
        ;;
      enroll)
        flow_enroll
        return
        ;;
      uninstall)
        flow_uninstall
        return
        ;;
    esac
  else
    arg_error "nothing to install: pass --everything, or components like --server --runtime"
    return 2
  fi
  while :; do
    filter_selection
    if [ -z "$SEL" ]; then
      ui_fail "Nothing selected can be installed on this $MACHINE."
      return 1
    fi
    ask_questions
    plan_build
    plan_confirm_run
    rc=$?
    if [ "$rc" = 20 ]; then
      choose_components || return 0
      continue
    fi
    break
  done
  [ "$OPT_DRY_RUN" = 1 ] && return "$rc"
  if [ "$PLAN_N" -gt 0 ] && [ "${PLAN_RESULT[1]}" != pending ]; then summary_screen; fi
  return "$rc"
}

# What `curl … | sudo sh` always did: the apt repo and the storm-server
# package, nothing else (no `up`: that needs choices).
flow_legacy_bootstrap() {
  local rc
  ui_info "No terminal and no options: doing what the apt bootstrap always did, the apt"
  ui_info "repository and the storm-server package. For the rest, run it in a terminal"
  ui_info "or see --help."
  if ! comp_available server; then
    ui_fail "$(comp_reason server)"
    return 1
  fi
  plan_reset
  apt_plan_repo
  plan_add spin 1 "Install the storm-server package" "sudo apt-get install -y storm-server" apt_install_pkg storm-server install
  plan_confirm_run
  rc=$?
  [ "$rc" = 0 ] || return "$rc"
  ui_line ""
  ui_info "Installed. Configure and start with:"
  ui_info "  sudo storm-server up"
  ui_info "  sudo storm-server status"
  ui_info "Or run the installer in a terminal: curl -fsSL $STORM_INSTALL_URL | sh"
  return 0
}

flow_upgrade() {
  local c rc
  SEL=""
  for c in server runtime app; do
    if comp_installed "$c"; then SEL="${SEL:+$SEL }$c"; fi
  done
  if [ -z "$SEL" ]; then
    ui_info "Nothing from Storm is installed here to upgrade."
    return 0
  fi
  if [ -z "$REL_VERSION" ]; then
    ui_fail "$REL_ERROR"
    return 1
  fi
  ask_questions
  plan_build
  plan_confirm_run
  rc=$?
  if [ "$OPT_DRY_RUN" != 1 ] && [ "$PLAN_N" -gt 0 ] && [ "${PLAN_RESULT[1]}" != pending ]; then summary_screen; fi
  return "$rc"
}

flow_pair() {
  local state
  SEL=mobile
  if [ "$SRV_INSTALLED" != 1 ]; then
    ui_info "There's no Storm server on this $MACHINE. Add devices from your server:"
    ui_info "Settings › Devices in the app or the web client."
  elif [ "$SRV_ACCOUNT" != yes ]; then
    SEL="server mobile"
  fi
  set_mode server keep
  settle_config
  if [ "$INTERACTIVE" = 1 ] && [ -z "$OPT_ADDR" ] && [ "$SRV_INSTALLED" = 1 ]; then
    ui_input "Address phones use to reach this $MACHINE" "$ADVERTISE"
    ADVERTISE=$UI_RESULT
  fi
  LOCAL_SERVER=$SRV_INSTALLED
  plan_reset
  if in_sel server; then
    state="$DATA_ROOT/state"
    plan_add fg "$(srv_owner_sudo "$state")" "Pair a device (the first one creates the account)" \
      "$(srv_owner_prefix "$state")$(srv_bin_display) pair --state $(shq "$state") --addr $ADVERTISE --qr" server_configure account
  fi
  mobile_plan
  plan_confirm_run
}

flow_enroll() {
  if [ "$RT_INSTALLED" != 1 ]; then
    ui_info "The Runtime Host isn't installed here. Choose Runtime only (or Custom…) to install it."
    return 0
  fi
  SEL=runtime
  set_mode runtime keep
  settle_config
  local_server_now
  RT_ENROLL_ACTION=enroll
  [ "$RT_ENROLLED" = yes ] && choose_enroll_action
  plan_build
  plan_confirm_run
}

flow_uninstall() {
  local c rc
  local -a items
  items=()
  SEL=""
  for c in server runtime app; do
    comp_installed "$c" || continue
    items[${#items[@]}]="$c|$(comp_label "$c")|off|$(comp_call "$c" status_line)"
  done
  if [ "${#items[@]}" = 0 ]; then
    ui_info "Nothing from Storm is installed here."
    return 0
  fi
  if [ "$INTERACTIVE" = 1 ]; then
    ui_checklist "Remove which? Vaults, notes and workspaces are never deleted." "${items[@]}" || return 0
    SEL=$UI_RESULT
  else
    for c in $OPT_COMPONENTS; do
      if comp_installed "$c"; then SEL="${SEL:+$SEL }$c"; fi
    done
    if [ -z "$SEL" ]; then
      arg_error "uninstall: say which installed components to remove (--server --runtime --app, or --everything)"
      return 2
    fi
  fi
  [ -n "$SEL" ] || return 0
  RT_PURGE=""
  if in_sel runtime && [ "$OS" = macos ] && [ "$INTERACTIVE" = 1 ]; then
    ui_confirm "Also remove the runtime's state (enrollment, agent logins, config, its account)? Workspaces stay either way." n && RT_PURGE=1
  fi
  for c in $SEL; do set_mode "$c" uninstall; done
  settle_config
  RT_ENROLL_ACTION=skip
  PENDING_NOTES=""
  plan_build
  plan_confirm_run
  rc=$?
  [ "$OPT_DRY_RUN" = 1 ] && return "$rc"
  ui_head "  What remains"
  in_sel server && ui_info "Your data: $DATA_ROOT (vaults, state, backups)"
  in_sel runtime && ui_info "The runtime's workspaces: $RT_WORKSPACES"
  in_sel runtime && [ -z "$RT_PURGE" ] && ui_info "The runtime's state and config: $RT_STATE, $RT_CONFIG"
  [ "$OS" = linux ] && ui_info "Storm's apt source: $LIST_PATH (remove it to stop updates)"
  ui_info "This installer's log: $LOG_FILE"
  return "$rc"
}

summary_screen() {
  local i failed=""
  OPT_DRY_RUN=0
  detect_all
  ui_head "  Summary"
  status_lines >&"$UI_FD"
  ui_line ""
  if [ "$LOCAL_SERVER" = 1 ]; then
    if [ "$OS" = macos ]; then
      ui_info "The server runs as $ME (LaunchAgent dev.storm.server); your data is in $DATA_ROOT."
    else
      ui_info "The server runs under systemd (storm-server); your data is in $DATA_ROOT."
    fi
    ui_info "Web client: ${C_BOLD}$(web_url)${C_RESET}"
  fi
  if [ "$RT_INSTALLED" = 1 ]; then
    if [ "$OS" = macos ]; then
      ui_info "The Runtime Host runs as $RT_ACCOUNT (LaunchDaemon dev.storm.runtime); workspaces: $RT_WORKSPACES"
    else
      ui_info "The Runtime Host runs as $RT_ACCOUNT (systemd storm-runtime); workspaces: $RT_WORKSPACES"
    fi
  fi
  i=1
  while [ "$i" -le "$PLAN_N" ]; do
    case "${PLAN_RESULT[$i]}" in
      failed) failed="$failed\n    $G_FAIL ${PLAN_DESC[$i]}" ;;
      skipped) failed="$failed\n    - ${PLAN_DESC[$i]} (skipped)" ;;
      pending) failed="$failed\n    - ${PLAN_DESC[$i]} (not run)" ;;
    esac
    i=$((i + 1))
  done
  if [ -n "$failed" ]; then
    ui_line ""
    ui_line "  Not done:"
    printf '%b\n' "$failed" >&"$UI_FD"
  fi
  ui_line ""
  ui_info "Log: $LOG_FILE"
  ui_info "Come back any time. The same command shows what's installed and offers"
  ui_info "upgrades, pairing, enrollment and uninstall:"
  ui_info "  curl -fsSL $STORM_INSTALL_URL | sh"
  if [ "$OS" = macos ] && [ "$LOCAL_SERVER" = 1 ]; then
    ui_line ""
    ui_info "What to expect:"
    ui_info " - The first time a device connects over the network, macOS may ask whether"
    ui_info "   storm-server may accept incoming connections. Allow it."
    ui_info " - The server runs while you're logged in: it stops at logout, and starts at login."
  fi
  if in_sel mobile; then
    ui_info " - Phones and tablets must be on the same network as this $MACHINE."
  fi
  return 0
}

# ------------------------------------------------------------ main

cleanup() {
  if [ -n "$SPIN_PID" ]; then kill "$SPIN_PID" 2>/dev/null || :; fi
  if [ "$INTERACTIVE" = 1 ] && [ "$RAW_KEYS" = 1 ]; then
    if [ -n "$STTY_SAVED" ]; then stty "$STTY_SAVED" <&8 2>/dev/null || :; fi
    stty echo <&8 2>/dev/null || :
    printf '\033[?25h' >&9 2>/dev/null || :
  fi
  if [ -n "$WORK_DIR" ] && [ -d "$WORK_DIR" ]; then rm -rf "$WORK_DIR"; fi
  return 0
}

storm_main() {
  local rc=0
  parse_args "$@" || return 2
  if [ "$OPT_HELP" = 1 ]; then
    usage
    return 0
  fi
  detect_platform
  set_paths
  ui_init
  trap cleanup EXIT
  trap 'cleanup; exit 130' INT
  trap 'cleanup; exit 143' TERM
  if [ "$OS" = other ]; then
    ui_fail "Storm's installer supports macOS and Debian/Ubuntu Linux; this is $OS_LABEL."
    ui_info "Release binaries: $STORM_REPO_URL/releases"
    return 1
  fi
  if [ "$OS" = macos ] && [ "$MY_UID" = 0 ]; then
    ui_fail "Run the installer as yourself, not with sudo: the server runs as you,"
    ui_info "and the installer asks for sudo only for the steps that need it."
    return 2
  fi
  # `curl … | sudo sh` with no terminal and no options is how the old apt
  # bootstrap was run, by people and by scripts: keep doing exactly that.
  LEGACY_BOOTSTRAP=0
  if [ "$OS" = linux ] && [ "$MY_UID" = 0 ] && [ "$INTERACTIVE" != 1 ] && [ $# -eq 0 ]; then
    LEGACY_BOOTSTRAP=1 OPT_YES=1
  fi
  if [ "$INTERACTIVE" != 1 ] && [ "$OPT_YES" != 1 ] && [ "$OPT_DRY_RUN" != 1 ] && [ "$SUBCOMMAND" != status ]; then
    refuse_no_tty
    return 2
  fi
  log_init "$SUBCOMMAND $*"
  if ! WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/storm-install.XXXXXX"); then
    ui_fail "couldn't make a temporary directory"
    return 1
  fi
  release_init
  detect_all
  availability_all
  if [ "$LEGACY_BOOTSTRAP" = 1 ]; then
    flow_legacy_bootstrap || rc=$?
    return "$rc"
  fi
  case "$SUBCOMMAND" in
    status)
      settle_config
      status_screen
      ;;
    upgrade) flow_upgrade || rc=$? ;;
    uninstall) flow_uninstall || rc=$? ;;
    *) flow_install || rc=$? ;;
  esac
  return "$rc"
}

storm_install_start() {
  [ "${STORM_INSTALL_TEST:-}" = 1 ] && return 0
  if [ "${BASH_VERSINFO[0]}" -lt 3 ] || { [ "${BASH_VERSINFO[0]}" = 3 ] && [ "${BASH_VERSINFO[1]}" -lt 2 ]; }; then
    echo "Storm's installer needs bash 3.2 or newer." >&2
    exit 1
  fi
  storm_main "$@"
  exit $?
}

storm_install_start "$@"
__STORM_INSTALLER_BODY__
}

storm_installer_entry() {
  if [ -n "${BASH_VERSION:-}" ]; then
    # macOS runs /bin/sh as bash in POSIX mode; the body wants plain bash.
    # shellcheck disable=SC3040
    set +o posix 2>/dev/null || :
    eval "$(storm_installer_body)"
    return
  fi
  if ! command -v bash >/dev/null 2>&1; then
    echo "Storm's installer needs bash (3.2 or newer). Install bash, then run it again." >&2
    return 1
  fi
  storm_tmp=$(mktemp "${TMPDIR:-/tmp}/storm-install.XXXXXX") || return 1
  if ! storm_installer_body >"$storm_tmp"; then
    rm -f "$storm_tmp"
    return 1
  fi
  STORM_INSTALL_SELF_TMP=$storm_tmp
  export STORM_INSTALL_SELF_TMP
  exec bash "$storm_tmp" "$@"
}

storm_installer_entry "$@"
