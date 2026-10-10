#!/usr/bin/env bash
# One fake for every program the installer drives; it acts by the name it is
# called as. Each call is logged, argv only, to $FAKE_LOG as
#   step=<STORM_STEP> <name> <args...>
# (environment probes -- uname, sw_vers, ipconfig, ip, id, stat -- are not
# logged).
# Whatever a fake reads on stdin is appended to $FAKE_STDIN, so a test can
# check a secret arrived on stdin and never in argv.
#
# Knobs (env): FAKE_UNAME_S/M, FAKE_SERVER_VERSION, FAKE_RUNTIME_VERSION,
# FAKE_PORT, FAKE_HEALTH_DOWN, FAKE_LATEST, FAKE_ASSETS, FAKE_DIR,
# STORM_TEST_SYSROOT.

name=${0##*/}
self=$0

case "$name" in
  uname | sw_vers | ipconfig | ip | id | stat) ;;
  *) printf 'step=%s %s %s\n' "${STORM_STEP:-}" "$name" "$*" >>"$FAKE_LOG" ;;
esac

capture_stdin() {
  {
    printf '%s: ' "$1"
    cat
    printf '\n'
  } >>"$FAKE_STDIN"
}

macos() { [ "${FAKE_UNAME_S:-Linux}" = Darwin ]; }

rt_state() {
  if macos; then
    printf '%s' "$STORM_TEST_SYSROOT/Library/StormRuntime/state"
  else
    printf '%s' "$STORM_TEST_SYSROOT/var/lib/storm-runtime"
  fi
}

# Install a copy of this fake under another path, keeping its name.
place() {
  mkdir -p "$(dirname "$1")"
  cp "$self" "$1"
  chmod 0755 "$1"
}

case "$name" in
  uname)
    case "${1:-}" in
      -s) echo "${FAKE_UNAME_S:-Linux}" ;;
      -m) echo "${FAKE_UNAME_M:-x86_64}" ;;
      *) echo "${FAKE_UNAME_S:-Linux}" ;;
    esac
    ;;
  sw_vers) echo "${FAKE_MACOS_VERSION:-15.1}" ;;
  id)
    case "$*" in
      -un | "-u -n" | "-nu") echo "$FAKE_USER" ;;
      *) exec "$FAKE_REAL_ID" "$@" ;;
    esac
    ;;
  stat)
    out=$("$FAKE_REAL_STAT" "$@") || exit
    if [ "$out" = "$FAKE_REAL_USER" ]; then echo "$FAKE_USER"; else printf '%s\n' "$out"; fi
    ;;
  ipconfig) [ "${2:-}" = en0 ] && echo 192.168.1.20 ;;
  ip) echo "1.1.1.1 via 192.168.1.1 dev eth0 src 192.168.1.10 uid 1000" ;;

  sudo)
    while [ $# -gt 0 ]; do
      case "$1" in
        -n | -H | -E | -S) shift ;;
        -u)
          shift 2
          ;;
        -v) exit 0 ;;
        *) break ;;
      esac
    done
    [ $# -eq 0 ] && exit 0
    if [ "$1" = env ]; then
      shift
      while [ $# -gt 0 ] && [ "${1#*=}" != "$1" ]; do shift; done
    fi
    # Run it for real: tests keep every system path under the sysroot.
    exec "$@"
    ;;

  storm-server)
    case "${1:-}" in
      --version) echo "storm-server ${FAKE_SERVER_VERSION:-0.6.0}" ;;
      status)
        echo "unit     : active"
        echo "listen   : 0.0.0.0:${FAKE_PORT:-8484}"
        ;;
      has-account) [ -f "$FAKE_DIR/account" ] ;;
      host-enrollment)
        if [ -t 1 ]; then
          echo "refusing to print an enrollment string on a terminal" >&2
          exit 1
        fi
        [ -f "$FAKE_DIR/account" ] || {
          echo "no account" >&2
          exit 1
        }
        echo "storm-enroll:v1:LOCALSECRET-fromhost-0001"
        ;;
      qr) echo "[qr ${2:-}]" ;;
      pair)
        echo "[pairing qr]"
        [ -n "${FAKE_PAIR_CREATES_ACCOUNT:-}" ] && touch "$FAKE_DIR/account"
        exit 0
        ;;
      passwd)
        case " $* " in
          *" --password-stdin "*) capture_stdin "storm-server passwd" ;;
        esac
        touch "$FAKE_DIR/account"
        ;;
      up)
        if macos; then
          data="$HOME/Storm"
          while [ $# -gt 0 ]; do
            [ "$1" = --data-root ] && data=$2
            shift
          done
          place "$HOME/Library/Application Support/Storm/bin/storm-server"
          mkdir -p "$HOME/Library/LaunchAgents" "$data/state"
          printf '<dict>\n<key>STORM_STATE</key>\n<string>%s/state</string>\n</dict>\n' "$data" \
            >"$HOME/Library/LaunchAgents/dev.storm.server.plist"
        fi
        echo "storm-server is up"
        ;;
      down) echo "stopped" ;;
      uninstall) rm -f "$HOME/Library/Application Support/Storm/bin/storm-server" ;;
    esac
    ;;

  storm-runtime)
    case "${1:-}" in
      --version) echo "storm-runtime ${FAKE_RUNTIME_VERSION:-0.6.0}" ;;
      install)
        place "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime"
        mkdir -p "$(rt_state)"
        [ -f "$STORM_TEST_SYSROOT/Library/StormRuntime/runtime.toml" ] ||
          printf '# runtime.toml\n#[[providers]]\n' >"$STORM_TEST_SYSROOT/Library/StormRuntime/runtime.toml"
        echo "Installed the Storm Runtime Host."
        ;;
      enroll)
        line=$(head -n 1)
        printf 'storm-runtime enroll: %s\n' "$line" >>"$FAKE_STDIN"
        case "$line" in
          storm-enroll:v1:*) ;;
          *)
            echo "not an enrollment string" >&2
            exit 2
            ;;
        esac
        mkdir -p "$(rt_state)"
        echo '{"host":"fake"}' >"$(rt_state)/host.json"
        echo "enrolled with http://127.0.0.1:8484 as host fake"
        ;;
      check) [ -f "$(rt_state)/host.json" ] && echo "online" ;;
      uninstall) rm -f "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime" ;;
    esac
    ;;

  apt-get)
    case "${1:-}" in
      install)
        for a in "$@"; do
          case "$a" in
            *storm-server*) place "$FAKE_DIR/installed/storm-server" ;;
            *storm-runtime*) place "$FAKE_DIR/installed/storm-runtime" ;;
          esac
        done
        ;;
      remove)
        for a in "$@"; do
          case "$a" in
            storm-server | storm-runtime) rm -f "$FAKE_DIR/installed/$a" ;;
          esac
        done
        ;;
    esac
    exit 0
    ;;

  systemctl)
    case "${1:-}" in
      is-active) exit "${FAKE_RUNTIME_INACTIVE:-0}" ;;
    esac
    exit 0
    ;;

  launchctl)
    [ "${1:-}" = print ] && echo "state = running"
    exit 0
    ;;

  curl)
    out="" url=""
    while [ $# -gt 0 ]; do
      case "$1" in
        -o)
          out=$2
          shift
          ;;
        -w | --max-time | --retry)
          shift
          ;;
        -*) ;;
        *) url=$1 ;;
      esac
      shift
    done
    emit() { if [ -n "$out" ] && [ "$out" != /dev/null ]; then cat >"$out"; else cat; fi; }
    case "$url" in
      */v1/health)
        [ -n "${FAKE_HEALTH_DOWN:-}" ] && exit 7
        echo '{"ok":true}'
        ;;
      *api.github.com*)
        printf '{\n  "url": "x",\n  "tag_name": "v%s",\n  "name": "v%s"\n}\n' "${FAKE_LATEST:-0.6.0}" "${FAKE_LATEST:-0.6.0}"
        ;;
      */storm-archive-keyring.gpg) printf 'KEYRING' | emit ;;
      */releases/download/*)
        f="${FAKE_ASSETS:-/nonexistent}/${url##*/}"
        [ -f "$f" ] || exit 22
        emit <"$f"
        ;;
      *) exit 22 ;;
    esac
    ;;

  brew)
    if [ "${1:-}" = install ]; then
      for a in "$@"; do
        case "$a" in
          claude-code) place "$STORM_TEST_SYSROOT/opt/homebrew/bin/claude" ;;
          opencode) place "$STORM_TEST_SYSROOT/opt/homebrew/bin/opencode" ;;
        esac
      done
    fi
    ;;

  ditto)
    if [ "${1:-}" = -x ]; then
      mkdir -p "$4/Storm.app/Contents"
      echo fake >"$4/Storm.app/Contents/Info.plist"
    else
      cp -R "$1" "$2"
    fi
    ;;

  defaults) echo "${FAKE_APP_VERSION:-0.5.0}" ;;
  claude | opencode) exit 0 ;;
esac
