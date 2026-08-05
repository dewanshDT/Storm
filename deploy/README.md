# Deploying Storm

A single static binary, a systemd unit, and a nightly backup. No container, no
runtime dependencies — the server is a ~5 MB musl binary that runs on a bare
Ubuntu or Debian box with nothing installed.

See decision 8 in `PLAN.md` for why this isn't a Docker image.

## First-time setup

On the server, once:

```sh
sudo useradd --system --home /srv/storm --shell /usr/sbin/nologin storm
sudo mkdir -p /srv/storm/{vault,state,web,backups} /etc/storm
sudo chown -R storm:storm /srv/storm

sudo cp deploy/storm-server.service /etc/systemd/system/
sudo cp deploy/storm-backup.service deploy/storm-backup.timer /etc/systemd/system/
sudo cp deploy/storm.env.example /etc/storm/storm.env
sudo chmod 600 /etc/storm/storm.env      # it holds the token
sudoedit /etc/storm/storm.env            # set STORM_TOKEN and the backup path
```

Generate a token with `openssl rand -hex 32`. It lives in the env file rather
than on the command line because process arguments are readable by every local
user through `/proc`.

Then from your machine:

```sh
make deploy HOST=you@your-server
sudo systemctl enable --now storm-server storm-backup.timer
```

## Importing a real vault

**Always dry-run against a copy first.** The import adds `id` frontmatter to
every note that lacks one, which is a write to every file:

```sh
storm-server --vault /srv/storm/vault --state /srv/storm/state --dry-run
```

It reports how many files would gain frontmatter and writes nothing. Only run
it for real once the count looks right.

## Everyday use

```sh
make deploy HOST=...     # cross-compile, push binary + web client, restart
make deploy-check HOST=... # health, service state, next backup
journalctl -u storm-server -f
```

`make deploy` restarts the service and fails loudly if it doesn't come back.

## Backups

`storm-backup.timer` runs nightly at 03:30 with a randomised delay, and
`Persistent=true` so a night the box was off is caught up rather than skipped.

Both halves are backed up, for different reasons:

- **`vault/`** — the notes. Plain files, so `rsync -a --delete` into a dated
  directory. Dating it means an accidental mass-delete is still recoverable
  from yesterday.
- **`state/index.db`** — version history, which the 3-way merge uses as its
  base. Everything else in `state/` can be rebuilt by rescanning the vault;
  this cannot.

The index is **not** rsynced. It runs in WAL mode with the server holding it
open, so a file copy can catch committed pages still sitting in the `-wal` and
produce a database that opens but silently lacks history. `storm-server
--backup-db` uses SQLite's `VACUUM INTO`, which is correct against a live
database and yields one compacted file with no sidecars to keep together.

The script then reopens the snapshot to check it works. A backup nobody has
read is a guess.

Point `STORM_BACKUP_DEST` at the TrueNAS mount. A local path only protects
against mistakes, not against losing the disk.

### Restoring

```sh
sudo systemctl stop storm-server
sudo rsync -a --delete /path/to/backup/vault/ /srv/storm/vault/
sudo cp /path/to/backup/index.db /srv/storm/state/index.db
sudo rm -f /srv/storm/state/index.db-wal /srv/storm/state/index.db-shm
sudo chown -R storm:storm /srv/storm
sudo systemctl start storm-server
```

Delete the `-wal` and `-shm`: they belong to the database you just replaced,
and leaving them behind can corrupt the restored one.

If you only have the vault, that is still enough — start the server against it
and the scan rebuilds the whole index. You lose version history, so the first
sync from a device that edited offline may conflict rather than merge cleanly.

## Security

v1 is **LAN-only**: one shared bearer token, no TLS. That is defensible on a
home network and nowhere else. Before this is reachable from the internet it
needs TLS and per-device tokens — see decision 4 in `PLAN.md`. Do not
port-forward it as it stands.

The unit runs as a dedicated `storm` user with `ProtectSystem=strict` and
write access to `/srv/storm` only.
