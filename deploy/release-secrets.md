# Release secrets (one-time)

Needed before the first useful `v*.*.*` tag. Apt signing and Pages are set up;
Android signing is optional for the first server release.

## Status

| Item | State |
|---|---|
| `STORM_APT_GPG_PRIVATE` | set (2026-08-11) |
| `STORM_APT_GPG_PASSPHRASE` | set empty (key is unprotected for CI) |
| Pages source = GitHub Actions | enabled → https://dewanshdt.github.io/Storm/ |
| Public keyring in repo | `deploy/storm-archive-keyring.gpg` (+ `.asc`) |
| Android upload keystore | **not set** — first APK will be debug-signed |

## Android upload keystore (optional until you ship APKs)

```sh
keytool -genkey -v -keystore storm-upload.jks -keyalg RSA -keysize 2048 \
  -validity 10000 -alias storm
base64 -i storm-upload.jks | pbcopy
gh secret set STORM_UPLOAD_STORE_BASE64 -R dewanshDT/Storm
gh secret set STORM_UPLOAD_STORE_PASSWORD -R dewanshDT/Storm
gh secret set STORM_UPLOAD_KEY_ALIAS -R dewanshDT/Storm --body storm
gh secret set STORM_UPLOAD_KEY_PASSWORD -R dewanshDT/Storm
```

Keep the `.jks` offline; losing it means every install must uninstall to upgrade.

## Apt signing key (already done)

Regenerate only if the secret is lost:

```sh
gpg --batch --pinentry-mode loopback --passphrase '' \
  --quick-generate-key 'Storm apt <storm-apt@dewanshdt.github.io>' default default never
gpg --armor --export-secret-keys <KEYID> | gh secret set STORM_APT_GPG_PRIVATE -R dewanshDT/Storm
gpg --export <KEYID> > deploy/storm-archive-keyring.gpg
gpg --armor --export <KEYID> > deploy/storm-archive-keyring.asc
```

## After a tag (v0.2.2+ is live)

1. `git tag vX.Y.Z && git push origin vX.Y.Z`
2. Watch **Release** — it publishes the GitHub Release and then calls
   **Apt repository** (a `GITHUB_TOKEN` release does not fire other workflows
   on its own; `workflow_call` covers that).
3. Clean install on the VM (see below) — do **not** keep the hand-rolled
   `/home/dewansh/storm` layout as the long-term target.

First public artifacts: **`v0.2.2`** — https://github.com/dewanshDT/Storm/releases/tag/v0.2.2
Apt: https://dewanshdt.github.io/Storm/ (`storm-server` `0.2.2-1`).

## Clean install on the VM

**Done 2026-08-11** on `proxmox-mcp-vm`: `apt install storm-server` (0.2.2-1),
`storm-server up` with vaults on NAS and state under `/srv/storm`. Hand-rolled
`~/storm` and `~/storm-m15-cutover` removed.

Live layout:

| Path | Role |
|---|---|
| `/mnt/media/Docs/storm` | vault root (NFS) |
| `/srv/storm/state` | indexes + `vaults.json` |
| `/srv/storm/backups` | backup target |
| `/usr/share/storm/web` | package web client |
| `/etc/storm/storm.env` | token + paths (mode 600) |

```sh
# Preferred: one-liner (adds key + source, then apt install)
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh

# Or by hand:
# 1. Add the apt source (Pages root IS the apt root)
curl -fsSL https://dewanshdt.github.io/Storm/storm-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/storm.gpg >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main' \
  | sudo tee /etc/apt/sources.list.d/storm.list
sudo apt update
sudo apt install storm-server

# Later — update (no storm-server upgrade command):
# sudo apt update && sudo apt install --only-upgrade storm-server
# sudo systemctl restart storm-server

# 2. Stop any hand-started process / old unit that shadows the package
sudo systemctl disable --now storm-server 2>/dev/null || true
sudo rm -f /etc/systemd/system/storm-server.service
sudo systemctl daemon-reload

# 3. Migrate state (vaults stay where the registry points — often NAS)
sudo mkdir -p /srv/storm/{vaults,state,backups}
# rsync old state into /srv/storm/state, chown to the operator that can
# write the vault root (NFS cannot be chown'd to User=storm).
sudo storm-server up \
  --data-root /srv/storm \
  --vault-root /mnt/media/Docs/storm \
  --state /srv/storm/state

# 4. Pair each client again — the shared token is gone, so a device that
#    predates the cutover has no credential the server will accept.
#    `storm-server pair --qr` on the host prints a single-use pairing code
#    while the user table is empty (without `--qr` it prints the URI only);
#    after that, add devices from a signed-in client.
```

`up` runs as the state-dir owner when vaults are on NFS — do not force
`User=storm` against an unchownable mount.
