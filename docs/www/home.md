---
tags: [storm, www, docs]
route: /
---

# Storm Website Home

Content for route **`/`**. Parent: [[Storm Website]].

## Meta

- **Title:** Storm
- **Description:** Self-hosted markdown notes. Your server owns the vault; phone, Mac, and browser sync to it.

## Hero

Brand is the hero signal (BrandMark + wordmark). Visible headline is supporting
copy — do not let a bigger H1 overpower the mark.

**Headline**

> Your notes. Your server.
> Plain markdown, always.

**Support**

A small Rust sync server in the homelab owns the canonical vault. Flutter
clients on phone, Mac, and web keep pace — offline is normal, conflicts stay
visible in the file.

**CTAs**

| Label | Target |
|---|---|
| Install the server | `/install` |
| View on GitHub | `https://github.com/dewanshDT/Storm` |

## Section — Why it exists

**Eyebrow:** Why it exists

**Heading:** Obsidian + Syncthing, replaced

**Body**

Syncthing moves files without knowing what a note is. Storm keeps notes as
ordinary markdown on disk — greppable, backupable, openable in Obsidian if you
ever leave — and puts note-aware sync, merge, and search on a box you run.

## Design notes

- One composition in the first viewport: brand, one headline, one support
  sentence, one CTA group, atmospheric background — no cards in the hero.
- Storm dark tokens; mint BrandMark ground `#96F2D7` does not re-theme.
- Claims must match the live app / `docs/storm-ui.md` — do not invent features.
