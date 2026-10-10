# shellcheck shell=bash
# Shared setup for the installer's bats tests.
#
# Every test gets a scratch world: HOME, a sysroot that stands in for /,
# and a PATH holding only the fakes (fake/fake.sh under many names) and a
# small allowlist of real tools -- never the machine's own sudo, apt-get or
# storm-* binaries. Then deploy/install.sh is sourced with
# STORM_INSTALL_TEST=1, which defines its functions and runs nothing.

INSTALLER="$BATS_TEST_DIRNAME/../../deploy/install.sh"
FAKE_SRC="$BATS_TEST_DIRNAME/fake/fake.sh"

# Real tools the installer and the fakes may use.
REAL_TOOLS="bash sh cat sed awk grep head tail tr cut date dirname basename mkdir rm cp mv ln
chmod mktemp tar gzip sha256sum sleep kill env tee touch ls sort wc find printf
test true false readlink stty tput"

# The fakes on PATH (id and stat only rename the user running the tests). storm-server and storm-runtime are not: like the real
# ones, they appear when a fake apt-get or a fake `up`/`install` puts them there.
FAKE_TOOLS="sudo curl systemctl launchctl apt-get brew ditto defaults uname sw_vers ipconfig ip id stat"

setup_world() {
  local t n p
  t=$BATS_TEST_TMPDIR
  # The real ones, found before PATH holds only the fakes.
  FAKE_REAL_USER=$(id -un)
  FAKE_REAL_ID=$(type -P id)
  FAKE_REAL_STAT=$(type -P stat)
  export REAL_PYTHON
  REAL_PYTHON=$(type -P python3 || :)
  export FAKE_DIR="$t/fake"
  export FAKE_LOG="$FAKE_DIR/argv.log"
  export FAKE_STDIN="$FAKE_DIR/stdin.log"
  mkdir -p "$FAKE_DIR/bin" "$FAKE_DIR/installed" "$FAKE_DIR/tools" "$t/sysbin" "$t/home" "$t/sysroot"
  : >"$FAKE_LOG"
  : >"$FAKE_STDIN"
  cp "$FAKE_SRC" "$FAKE_DIR/fake.sh"
  chmod 0755 "$FAKE_DIR/fake.sh"
  for n in $FAKE_TOOLS; do ln -s "$FAKE_DIR/fake.sh" "$FAKE_DIR/bin/$n"; done
  # Named copies the tests drop into place (tarballs, "installed" binaries).
  for n in storm-server storm-runtime claude opencode; do cp "$FAKE_SRC" "$FAKE_DIR/tools/$n"; chmod 0755 "$FAKE_DIR/tools/$n"; done
  for n in $REAL_TOOLS; do
    p=$(type -P "$n" 2>/dev/null) || continue
    case "$p" in /*) ln -sf "$p" "$t/sysbin/$n" ;; esac
  done
  export PATH="$FAKE_DIR/bin:$FAKE_DIR/installed:$t/sysbin"
  export HOME="$t/home"
  unset XDG_STATE_HOME
  export STORM_INSTALL_TEST=1
  export STORM_TEST_SYSROOT="$t/sysroot"
  export STORM_TEST_OS_RELEASE="$t/os-release"
  export STORM_TTY="$t/no-tty/tty"
  export NO_COLOR=1 LANG=C.UTF-8 TERM=dumb
  export FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64
  # Whoever runs the tests is "tester": on a machine where that's really
  # storm-runtime or storm, the installer would rightly skip sudo.
  export FAKE_USER=tester FAKE_REAL_USER FAKE_REAL_ID FAKE_REAL_STAT
  unset STORM_TTY_OUT STORM_LINE_MODE FAKE_HEALTH_DOWN FAKE_PAIR_CREATES_ACCOUNT FAKE_LATEST FAKE_ASSETS
  os_release ubuntu
  mkdir -p "$STORM_TEST_SYSROOT/Applications"
  # shellcheck disable=SC1090
  source "$INSTALLER"
  # Read by the sourced installer: don't wait between polls.
  # shellcheck disable=SC2034
  POLL_SLEEP=0 HEALTH_TRIES=2
}

os_release() {
  case "$1" in
    ubuntu) printf 'ID=ubuntu\nID_LIKE=debian\nPRETTY_NAME="Ubuntu 24.04 LTS"\n' ;;
    debian) printf 'ID=debian\nPRETTY_NAME="Debian GNU/Linux 12"\n' ;;
    mint) printf 'ID=linuxmint\nID_LIKE="ubuntu debian"\nPRETTY_NAME="Linux Mint 22"\n' ;;
    fedora) printf 'ID=fedora\nPRETTY_NAME="Fedora Linux 40"\n' ;;
  esac >"$STORM_TEST_OS_RELEASE"
}

use_macos() {
  export FAKE_UNAME_S=Darwin FAKE_UNAME_M=${1:-arm64}
}

use_linux() {
  export FAKE_UNAME_S=Linux FAKE_UNAME_M=${1:-x86_64}
}

# Detect the platform and paths the way storm_main does.
platform() {
  detect_platform
  set_paths
}

# make_assets VERSION [names...]: release files and their checksums.txt in
# $ASSETS. With no names, all of them.
make_assets() {
  local v=$1 d n stage
  shift
  ASSETS="$BATS_TEST_TMPDIR/assets"
  stage="$BATS_TEST_TMPDIR/stage"
  rm -rf "$ASSETS" "$stage"
  mkdir -p "$ASSETS" "$stage/server/web" "$stage/runtime"
  cp "$FAKE_DIR/tools/storm-server" "$stage/server/storm-server"
  echo '<html></html>' >"$stage/server/web/index.html"
  cp "$FAKE_DIR/tools/storm-runtime" "$stage/runtime/storm-runtime"
  for d in server runtime; do
    echo "license" >"$stage/$d/LICENSE"
    echo "readme" >"$stage/$d/README-macos.txt"
  done
  local names="$*"
  [ -n "$names" ] || names="server-tar runtime-tar app-zip apk server-deb runtime-deb"
  for n in $names; do
    case "$n" in
      server-tar) tar -C "$stage/server" -czf "$ASSETS/storm-server-$v-macos-universal.tar.gz" . ;;
      runtime-tar) tar -C "$stage/runtime" -czf "$ASSETS/storm-runtime-$v-macos-universal.tar.gz" . ;;
      app-zip) echo "zip" >"$ASSETS/Storm-$v-macos-arm64.zip" ;;
      apk) echo "apk" >"$ASSETS/storm-$v.apk" ;;
      server-deb) echo "deb" >"$ASSETS/storm-server_$v-1_amd64.deb" ;;
      runtime-deb) echo "deb" >"$ASSETS/storm-runtime_$v-1_amd64.deb" ;;
    esac
  done
  (cd "$ASSETS" && sha256sum -- * >checksums.txt)
  export FAKE_ASSETS="$ASSETS"
}

# Put a fake "installed" binary in place.
install_fake() {
  case "$1" in
    linux-server) cp "$FAKE_DIR/tools/storm-server" "$FAKE_DIR/installed/storm-server" ;;
    linux-runtime) cp "$FAKE_DIR/tools/storm-runtime" "$FAKE_DIR/installed/storm-runtime" ;;
    macos-server)
      mkdir -p "$HOME/Library/Application Support/Storm/bin" "$HOME/Library/LaunchAgents" "$HOME/${2:-Storm}/state"
      cp "$FAKE_DIR/tools/storm-server" "$HOME/Library/Application Support/Storm/bin/storm-server"
      printf '{\n  "data_root": "%s",\n  "state": "%s/state",\n  "host": "0.0.0.0",\n  "port": %s\n}\n' \
        "$HOME/${2:-Storm}" "$HOME/${2:-Storm}" "${3:-8484}" \
        >"$HOME/Library/Application Support/Storm/server.json"
      : >"$HOME/Library/LaunchAgents/dev.storm.server.plist"
      ;;
    macos-runtime)
      mkdir -p "$STORM_TEST_SYSROOT/Library/StormRuntime/bin" "$STORM_TEST_SYSROOT/Library/StormRuntime/state"
      cp "$FAKE_DIR/tools/storm-runtime" "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime"
      ;;
    macos-app) mkdir -p "$STORM_TEST_SYSROOT/Applications/Storm.app/Contents" ;;
  esac
}

# The plan's steps as "N sudo|user" lines, from plan_print output in $1.
plan_steps() {
  printf '%s\n' "$1" | awk '/^ +[0-9]+\. /{ n=$1; sub(/\./,"",n); if ($2=="[sudo]") print n, "sudo"; else print n, "user" }'
}

# Each step's description lines from plan_print output.
plan_descs() {
  printf '%s\n' "$1" | sed -n 's/^ *[0-9][0-9]*\. \(\[sudo\]\)\{0,1\} *//p'
}

# Fail unless every [sudo] step shows a sudo command, and no other step does.
assert_sudo_marks_consistent() {
  printf '%s\n' "$1" | awk '
    /^ +[0-9]+\. / { step=$1; sudo=($2=="[sudo]"); has[step]=0; mark[step]=sudo; next }
    /^ +\$ / && step != "" { if ($0 ~ /(^|[ (|])sudo /) has[step]=1 }
    END {
      bad=0
      for (s in mark) if (mark[s] != has[s]) { print "step " s ": marked sudo=" mark[s] " but shows sudo=" has[s]; bad=1 }
      exit bad
    }'
}

# Steps in which the fake sudo ran something other than a timestamp refresh.
sudo_steps_used() {
  awk '$2=="sudo" && $0 !~ / sudo (-n )?-v$/ { sub(/^step=/,"",$1); if ($1 != "") print $1 }' "$FAKE_LOG" | sort -un
}
