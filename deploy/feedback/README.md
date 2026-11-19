# Feedback sync

Pulls pending reports out of the intake Worker and files each into the vault
as `Storm/Feedback/Inbox/FB-####.md`, deleting it from the Worker once it
lands. Not part of the Storm product — a standalone Python 3 script plus a
systemd timer, stdlib only, so it runs on the prod VM with no `pip install`.

It runs as its own paired Storm device, "feedback-sync": pairing happens
once, by hand; every run after that gets a session the same way any client
does (a stored refresh token, refreshed when it's close to expiring).

## Before you start

- `Storm/Feedback/Feedback.md` must already exist in the target vault, with
  an `## Inbox (new)` heading — the sync only edits under that heading.
- You need the intake Worker's `SYNC_TOKEN` and base URL.
- You need the account's own Storm password once, for the first `pair` run
  only. It is never stored.

## Install

```sh
sudo mkdir -p /opt/storm-feedback-sync
sudo cp feedback-sync.py /opt/storm-feedback-sync/
sudo cp feedback-sync.service feedback-sync.timer /etc/systemd/system/
sudo cp feedback-sync.env.example /etc/storm-feedback-sync.env
sudoedit /etc/storm-feedback-sync.env     # set STORM_URL, STORM_VAULT, INTAKE_URL, SYNC_TOKEN
sudo chmod 600 /etc/storm-feedback-sync.env
sudo systemctl daemon-reload
```

## Pair

Get a pairing code from the account's own device: **Settings > Devices &
access > Add a device**, then copy the `storm://pair?...` text (not just the
QR image) rather than scanning it. Then, as root (so the device credential
lands under `/var/lib/storm-feedback-sync`, the same `StateDirectory` the
unit uses):

```sh
sudo python3 /opt/storm-feedback-sync/feedback-sync.py pair \
    --server http://<storm-host>:8484 \
    --code 'storm://pair?v=1&sid=...&pk=...&n=...&exp=...&addr=...'
```

It prompts for the account password once (never stored), proves the server
at `--server` holds the key named in the pairing code (the same
challenge/verify handshake the app runs), and stores the device credential
in `/var/lib/storm-feedback-sync/device.json`, mode `0600`. Add
`--password-stdin` to read the password from stdin instead of a prompt (for
scripting the pair step itself — the service's own runs never touch the
password again).

A pairing code is a one-time nonce and expires in a few minutes; if it's
expired or already used, get a fresh one from the app and run `pair` again.

## Enable

Nothing above enables or starts anything. Once paired:

```sh
sudo systemctl enable --now feedback-sync.timer
```

Run it once by hand to check it works before waiting on the timer:

```sh
sudo systemctl start feedback-sync.service
sudo journalctl -u feedback-sync.service -n 50
```

Logs are ids, counts and booleans only — never a report's text, email,
tokens or diagnostics bundle.

## Config (`/etc/storm-feedback-sync.env`)

| Variable | Required | Meaning |
|---|---|---|
| `STORM_URL` | yes | The Storm server's base URL. |
| `STORM_VAULT` | yes | The vault id reports are filed into. |
| `FEEDBACK_FOLDER` | no (default `Storm/Feedback`) | Where `Inbox/` and `Feedback.md` live, relative to the vault root. |
| `INTAKE_URL` | yes | The intake Worker's base URL. |
| `SYNC_TOKEN` | yes | The Worker's bearer secret for `GET`/`DELETE /v1/reports*`. |
| `STATE_DIR` | no (default `/var/lib/storm-feedback-sync`) | Device credential + the synced-report map. Change only together with the unit's `StateDirectory=`. |

## How it stays idempotent

Each report is tracked by the intake id in `STATE_DIR/state.json`
(`id -> FB ref`). If a run crashes anywhere after creating a note — before
the screenshot upload, before the `Feedback.md` line, or before the Worker
delete — the next run finds the note already there (by the state file, or,
if that's lost too, by scanning the vault's `Inbox/` folder for a note whose
`intake:` frontmatter matches) and redoes only the steps after note
creation: the screenshot attachment (its `PUT` just overwrites the same
bytes) and the `Feedback.md` line (a no-op once the ref is already in the
section), then the Worker delete. Only the note creation itself is ever
skipped on a rerun — the one step that genuinely isn't safe to repeat.

## Uninstall

```sh
sudo systemctl disable --now feedback-sync.timer
sudo rm /etc/systemd/system/feedback-sync.service /etc/systemd/system/feedback-sync.timer
sudo systemctl daemon-reload
sudo rm -rf /opt/storm-feedback-sync /var/lib/storm-feedback-sync /etc/storm-feedback-sync.env
```

Revoke the device from the account's own **Settings > Devices & access**
list too — removing the files above does not revoke the credential on the
server.
