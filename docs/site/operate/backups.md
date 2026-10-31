---
title: Backups and restore
description: What Storm backs up every night, why, and how to restore it.
---

Backups run every night. On Linux, `storm-backup.timer` runs at 03:30 with a
randomised delay, and a night the machine was off is caught up rather than
skipped. On macOS, `storm-server up` installs a nightly LaunchAgent; on
Windows, a nightly task runs `storm-server backup`.

Three things are backed up, for different reasons:

- **the storage root** — every vault's notes. Plain files, so
  `rsync -a --delete` into a dated directory. Dating it means an accidental
  mass-delete is still recoverable from yesterday. A file that has not changed
  since the night before is a hard link to that night's copy, so a month of
  nights costs one copy plus what changed (Windows' `storm-server backup`
  copies in full). **A root that holds no files while the registry lists
  vaults is refused** — the backup exits with an `ERROR` and writes nothing,
  because that is what an unmounted share looks like, and copying it would
  leave a night that verifies and holds no notes. **It is the root the server
  actually uses** (`storm-server storage-root` prints it), which is not
  necessarily `STORM_VAULT_ROOT`: that only seeds a first run, and a root
  changed in the app is never written back to it. The log's `root:` line names
  the directory copied. Each vault's `.trash/` is copied with
  it, so a restore brings the Trash back too.
- **`state/<vault-id>/index.db`** — one index per vault, holding version
  history that the 3-way merge uses as its base and who deleted each item in
  the Trash, plus `vaults.json`. The rest
  of an index can be rebuilt by rescanning; the history cannot.
- **`state/auth.db` and `state/identity/`** — the server's own identity: its
  `server_id`, the public half of its Ed25519 credential, **its account**, and
  its devices' sessions. **Nothing rebuilds this.** An index restores itself
  from the markdown; a server that loses its identity cannot prove who it is to
  a single paired device, and a restore without it is a server holding every
  note that nobody can log into. The private key files are copied
  as files at mode `0600` — they are written once and never modified, so there
  is no torn state to worry about, and they have to travel with `auth.db`:
  the database alone restores a server that knows which key is active and
  cannot sign with it.

The databases are **not** rsynced. They run in WAL mode with the server holding
them open, so a file copy can catch committed pages still sitting in the `-wal`
and produce a database that opens but silently lacks rows. `storm-server
backup-db <dir>` uses SQLite's `VACUUM INTO`, which is correct against a live
database, and writes everything in the same layout as `state/` — so restoring
is a plain copy.

`state/gateway/gateway.db` and its data key in `state/gateway/keys/` are
snapshotted the same way, and together: they hold your integrations'
credentials and, if you use [Claude login sync](/docs/agents/claude-login/),
your Claude token, sealed under that key. **A backup with both holds your
token**: protect it like the server. A restore without `keys/` still starts,
and the app asks you to sync the token again.

`vaults.json` is copied alongside them. Without it the snapshots are a pile of
UUID-named directories nobody can match back to a vault.

The script then reopens the snapshots to check they work. A backup nobody has
read is a guess.

**Point `STORM_BACKUP_DEST` at another disk** — a NAS share, say. A path on
the same disk only protects against mistakes, not against losing the disk.
`STORM_BACKUP_KEEP_DAYS` (default 30) decides how long dated copies are kept.

## Restoring

```sh
sudo systemctl stop storm-server
sudo rsync -a --delete /path/to/backup/vaults/ /srv/storm/vaults/
sudo rsync -a --delete /path/to/backup/index/ /srv/storm/state/
sudo find /srv/storm/state -name 'index.db-wal' -delete
sudo find /srv/storm/state -name 'index.db-shm' -delete
sudo chown -R storm:storm /srv/storm
sudo systemctl start storm-server
```

Delete the `-wal` and `-shm`: they belong to the databases you just replaced,
and leaving them behind can corrupt the restored ones.

That `rsync` of the snapshot directory carries `auth.db` and `identity/` with
it, which is the point of them being written into the same layout. Check them
afterwards — the private keys must still be `0600` and owned by the service
account:

```sh
sudo ls -l /srv/storm/state/identity/
curl -s http://127.0.0.1:8484/v1/server    # same server_id as before the restore
```

`/v1/server` is unauthenticated, so this works before anything is logged in. A
**different** `server_id` there means the identity did not come back and every
paired device would have to pair again.

If you only have the vaults, that is still enough to read your notes — start
the server against them and the scan rebuilds every index, re-registering each
directory. You lose version history, so the first sync from a device that
edited offline may conflict rather than merge cleanly, and the server comes up
with a **new** identity.
