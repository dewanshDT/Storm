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
