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

Goal: FHS `/srv/storm`, `storm` user, package-owned web, systemd. Migrate data
onto that layout, then remove the old tree and `~/storm-m15-cutover`.

```sh
# 1. Add the apt source (Pages root IS the apt root)
curl -fsSL https://dewanshdt.github.io/Storm/storm-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/storm.gpg >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main' \
  | sudo tee /etc/apt/sources.list.d/storm.list
sudo apt update
sudo apt install storm-server

# 2. Stop the hand-started process
pkill -f '/home/dewansh/storm/storm-server' || true

# 3. Move data into the packaged layout (example — adjust if paths differ)
sudo mkdir -p /srv/storm/{vaults,state,backups}
# Prefer the NAS root the registry already uses, or copy vault dirs under vaults/
# Storm never moves vault dirs itself — point or copy deliberately.
sudo storm-server up   # creates env, enables systemd as User=storm

# 4. Point clients at the new token in /etc/storm/storm.env
# 5. Remove leftovers once healthy across a reboot:
#    ~/storm-m15-cutover, old run.sh binary, stale /home/dewansh/storm/vaults copy
```

Until you cut over, leave the current `run.sh` server alone.
