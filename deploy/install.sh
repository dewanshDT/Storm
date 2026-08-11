#!/bin/sh
# Storm apt bootstrap — same idea as Tailscale's install.sh:
#   curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
#
# Adds the signing key + apt source, then installs storm-server.
# Does not run `storm-server up` — that needs vault/state choices.
#
# Env:
#   STORM_APT_ROOT  apt/Pages root (default: https://dewanshdt.github.io/Storm)
#   STORM_CHANNEL   apt suite (default: stable)

set -eu

# All work is inside main so a truncated download cannot execute half a script.
main() {
  APT_ROOT="${STORM_APT_ROOT:-https://dewanshdt.github.io/Storm}"
  CHANNEL="${STORM_CHANNEL:-stable}"
  KEYRING_URL="${APT_ROOT%/}/storm-archive-keyring.gpg"
  KEYRING_PATH=/usr/share/keyrings/storm.gpg
  LIST_PATH=/etc/apt/sources.list.d/storm.list

  if [ "$(id -u)" -ne 0 ]; then
    echo "Re-run as root, e.g.:" >&2
    echo "  curl -fsSL ${APT_ROOT%/}/install.sh | sudo sh" >&2
    exit 1
  fi

  if [ ! -f /etc/os-release ]; then
    echo "Need /etc/os-release (Debian/Ubuntu)." >&2
    exit 1
  fi
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}:${ID_LIKE:-}" in
    debian:*|ubuntu:*|*:debian*|*:ubuntu*)
      ;;
    *)
      echo "Unsupported OS '${ID:-unknown}'. Storm's apt repo is Debian/Ubuntu only." >&2
      echo "Grab a release binary from https://github.com/dewanshDT/Storm/releases" >&2
      exit 1
      ;;
  esac

  if ! command -v apt-get >/dev/null 2>&1; then
    echo "apt-get not found." >&2
    exit 1
  fi

  FETCH=
  if command -v curl >/dev/null 2>&1; then
    FETCH="curl -fsSL"
  elif command -v wget >/dev/null 2>&1; then
    FETCH="wget -q -O-"
  else
    echo "Need curl or wget to download the apt key." >&2
    exit 1
  fi

  echo "Storm installer"
  echo "  apt root: $APT_ROOT"
  echo "  channel:  $CHANNEL"
  echo

  mkdir -p /usr/share/keyrings
  # Binary OpenPGP keyring (not ASCII armor) — apt signed-by expects this.
  $FETCH "$KEYRING_URL" >"$KEYRING_PATH"
  chmod 0644 "$KEYRING_PATH"

  printf 'deb [signed-by=%s] %s %s main\n' \
    "$KEYRING_PATH" "${APT_ROOT%/}" "$CHANNEL" >"$LIST_PATH"
  chmod 0644 "$LIST_PATH"

  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y
  apt-get install -y storm-server

  echo
  echo "Installed. Configure and start with:"
  echo "  sudo storm-server up"
  echo "  sudo storm-server status"
  echo
  echo "Later, update with:"
  echo "  sudo apt update && sudo apt install --only-upgrade storm-server"
  echo "  sudo systemctl restart storm-server"
  echo
  echo "Custom vaults/state: see deploy/README.md or the install page."
}

main "$@"
