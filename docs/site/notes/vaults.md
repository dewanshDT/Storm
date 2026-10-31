---
title: Vaults and folders
description: How Storm stores notes on disk, and how to bring an existing vault in.
---

The server's **storage root** is a folder that holds one folder per vault:

```
vaults/
├── personal/
├── work/
└── recipes/
```

Each vault is a plain tree of Markdown files and folders. Storm's own records
live in a separate `state/` folder beside the vaults, never inside them, so a
vault stays readable, greppable and portable with any other tool.

## Adding a vault

Create one in the app, or put a folder of Markdown into the storage root and
restart the server: it adopts every folder it finds there at startup. It does
**not** notice a new folder while running — a vault copied in with rsync
appears after the next restart. Edits to notes inside existing vaults are
picked up live, from any editor.

Renaming a vault, or its folder, keeps its history: Storm tracks each vault by
an id, not its name.

## Folders

Create, rename and delete folders in the app. An empty folder you created is
kept; folders left empty by moving or deleting notes are tidied away.

A `/` in a new note's name puts it in folders, making any that are missing:
`Ideas/Plan` is the note *Plan* in *Ideas* inside the folder you're in, and
`/Ideas/Plan` starts from the top of the vault. A name with `..`, or with
nothing after its last `/`, is refused.

Deleting a folder that holds notes asks first — "Delete folder X and its N
notes? They go to Trash." — and moves the folder and everything in it to the
Trash as one item. Storm's own settings folder, `_storm`, cannot be deleted.

## Trash

Nothing you delete is gone at once. A deleted note or folder moves to the
vault's `.trash/` folder, keeping its id and its history, and appears under
**Trash** in the vault's sidebar with where it was, when it was deleted and
by which device. This holds for a delete from the app, from an MCP key, and
for a note removed outside Storm (by another machine or a sync tool): the
server keeps the last version it had.

- **Restore** puts it back where it was, as the same note. If something new
  now has that name, the restored one is called `Name (restored).md` —
  nothing is overwritten. Attachments only that note used come back with it.
- **Delete forever** and **Empty trash** ask first, then remove the files
  and their history for good.
- Items are kept for **30 days**, then removed automatically. Change this
  per vault in the Trash view (7, 30 or 90 days, or never); it is stored as
  `storm.trash.retention` in `_storm/vault.md`.

An agent's deletion lands here only after you
[approve it](/docs/agents/sessions/#deletes-wait-for-you). Agents never see the Trash: no MCP tool lists, restores or empties it, and
trashed notes appear in no search, listing or backlinks.

## Moving the storage root

**Storm never moves vault folders.** Changing the storage root (in
**Settings › Storage**) points the server at folders you have already moved
yourself. If the new place holds none of the vaults the server knows, it
refuses, listing what would be lost, rather than starting with nothing in it.

## Bringing in an existing vault

Storm gives every note an `id` in its frontmatter the first time it sees it,
which is a write to every file that lacks one. **Dry-run against a copy
first:**

```sh
storm-server dry-run --vault-root /path/to/vaults-copy --state /tmp/storm-state
```

It reports, per vault, how many files would change, and writes nothing.
Everything else in your frontmatter is passed through untouched: Storm edits
single lines and never rewrites your YAML.
