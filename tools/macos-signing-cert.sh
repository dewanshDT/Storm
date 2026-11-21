#!/bin/sh
# Generates a self-signed macOS code-signing certificate for the Storm app
# (see deploy/release-secrets.md, "macOS app signing"), sets it as the repo
# secrets release.yml and acceptance.yml read, and keeps an offline backup.
#
# OPERATOR-RUN ONLY, once. Nothing in CI calls this — it needs
# `gh auth login` and is the thing that MAKES the secrets `gh secret set`
# would otherwise need already set.
#
# Why self-signed is enough here: the problem being fixed is macOS treating
# every ad-hoc build as a new code identity, so the keychain's "Always
# Allow" never survives an update. Any certificate with a *stable* identity
# fixes that — the app is unnotarized either way, so a $99/year Developer ID
# buys nothing this specific fix needs.
#
# `-legacy` on the PKCS#12 export: OpenSSL 3's default encryption is
# PBES2/AES, which macOS's `security import` cannot read (it wants the old
# RC2/3DES algorithms) — the openssl(1) manual documents `-legacy` as the
# knob for exactly that.

set -eu

CERT_CN="Storm App Signing"
VALID_DAYS=3650
KEY_BITS=3072
SECRET_P12="MACOS_SIGNING_P12"
SECRET_PASSWORD="MACOS_SIGNING_P12_PASSWORD"
BACKUP_DIR="$HOME/.config/storm-macos-signing"

command -v openssl >/dev/null 2>&1 || {
	echo "macos-signing-cert: openssl not found" >&2
	exit 1
}
command -v gh >/dev/null 2>&1 || {
	echo "macos-signing-cert: gh (GitHub CLI) not found" >&2
	exit 1
}
gh auth status >/dev/null 2>&1 || {
	echo "macos-signing-cert: not logged in — run 'gh auth login' first" >&2
	exit 1
}

work=$(mktemp -d)
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

key="$work/storm-app-signing.key"
cert="$work/storm-app-signing.crt"
p12="$work/storm-app-signing.p12"
ext="$work/codesign.ext"

cat >"$ext" <<EOF
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:FALSE
EOF

# A self-signed cert and key in one step: no CSR, nothing else to sign it.
openssl req -x509 -newkey "rsa:$KEY_BITS" -sha256 -days "$VALID_DAYS" \
	-nodes -keyout "$key" -out "$cert" \
	-subj "/CN=$CERT_CN" -extfile "$ext"

p12_password=$(openssl rand -base64 24)

# -legacy: see the header comment. macOS's security(1) import cannot read a
# plain OpenSSL-3 PKCS#12.
openssl pkcs12 -export -legacy \
	-inkey "$key" -in "$cert" \
	-name "$CERT_CN" \
	-passout "pass:$p12_password" \
	-out "$p12"

fingerprint=$(openssl x509 -in "$cert" -noout -fingerprint -sha1 | cut -d= -f2)
expiry=$(openssl x509 -in "$cert" -noout -enddate | cut -d= -f2)

base64 -i "$p12" | tr -d '\n' | gh secret set "$SECRET_P12"
printf '%s' "$p12_password" | gh secret set "$SECRET_PASSWORD"

mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
cp "$p12" "$BACKUP_DIR/storm-app-signing.p12"
printf '%s' "$p12_password" >"$BACKUP_DIR/storm-app-signing.p12.password"
chmod 600 "$BACKUP_DIR/storm-app-signing.p12" "$BACKUP_DIR/storm-app-signing.p12.password"

echo "Set $SECRET_P12 and $SECRET_PASSWORD."
echo "Backup saved to $BACKUP_DIR (mode 600) — keep it offline too; losing"
echo "both the secrets and this backup means re-running this script, which"
echo "resets everyone's keychain ACL once more."
echo "SHA-1 fingerprint: $fingerprint"
echo "Expires: $expiry"
