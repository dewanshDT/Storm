---
title: Sync, offline and conflicts
description: What happens when devices disagree, and what the status bar is telling you.
---

The server holds the real copy of every note. Each device keeps a cache of
the notes it has seen and a queue of changes it has not sent yet.

## Offline

Creating, editing, renaming, moving and deleting all work offline. The
changes queue on the device and are sent, in order, when the server is back.
The status bar shows **Offline**, or **N unsent**, whenever the server does
not have everything — check it before closing an app you edited offline.

Search needs the server; notes this device has cached stay readable.

## When two devices edit the same note

Every save carries the version it was based on. The server merges the two
edits line by line when it can, and the device adopts the merged text. When
it cannot, the conflicting lines are written **into the note itself**, between
markers, where you can see and resolve them — never into a hidden copy.

## Live updates

A change made on one device appears on the others within moments, over a
live connection. Edits made to the files directly — in another editor, over
the network share — are picked up and pushed the same way.

## Away from home

If your server has a [relay](/docs/start/concepts/#relays), the apps reach it
through the relay when its own address does not answer, and use whichever
route answers first.
