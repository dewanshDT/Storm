---
title: Writing notes
description: Read and Edit mode, links, search, properties, tags and attachments.
---

## Read and Edit

A note opens in **Read** mode — rendered Markdown — or in **Edit** mode, the
Markdown source with live styling and a formatting toolbar. Switch with the
button in the note's header or `⌘/Ctrl E`. Which one a note opens in is
**Settings › This device › Open notes in Read mode**.

Edits save on their own, a moment after you stop typing.

## Links

Link to another note with `[[Note name]]`. Typing `[[` offers the vault's
notes as you type, and following a link opens the note. Under each note,
**Linked mentions** lists the notes that link to it (it needs the server).

## Search

`⌘/Ctrl K` searches the whole vault's text, with the matching snippet
highlighted. Search runs on the server, so it covers notes this device has
never opened.

## Properties

The properties panel edits a note's frontmatter as typed fields: text,
number, checkbox, date, date and time, list, select, URL and colour. Storm rewrites only the line (or list) you
changed and passes every other byte through, so key order and comments
survive. A property's type is stored per vault, in a hidden note.

Notes can have a **colour** — coral, peach, sand, sage, mint, sky, lavender,
blossom — stored in the note as a word (`color: sage`), so it means something
outside Storm too. Vaults can have one as well.

## Tags

Tags come from the note's `tags:` property, as a list. Through MCP, AI apps
can list a vault's tags with counts and find notes that share them.

## Attachments

**Attach a file** adds it to the vault and links it from the note. Images are
shown under the note; the Markdown link stays visible and editable. A file can
be up to 64 MiB, and a note itself up to 16 MiB.

## Appearance

**Settings › This device** holds the text size and the note font (Newsreader,
Sans or Monospace). Themes — Storm dark, Storm light and SlowFlow earth — are
chosen there too. These are per device.
