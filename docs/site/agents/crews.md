---
title: Crews and roles
description: Group agent sessions that work toward one goal, and label each session with the role it plays.
---

A **crew** is a named, coloured group of [sessions](/docs/agents/sessions/)
working toward one goal: say an implementer, a reviewer and a docs pass on
the same release. A **role** is a label on one session, taken from a note in
your Kit, that says what the session is for.

Both only organise the Agents page. A crew or a role never changes what a
session can read or write, which host it runs on, or how it runs and ends.

## Crews

A session is in one crew at most, and sessions from different hosts and
workspaces can share a crew. Crews are listed under **CREWS**, above
**RUNNING** and **ENDED**, which then list only the sessions in no crew.

**Making a crew:**

- **On a desktop**, drag one session onto another. A crew named "New crew"
  appears with its name ready to type over.
- **Several at once:** ⌘-click (Ctrl-click on Windows and Linux) or
  Shift-click sessions in the sidebar, then choose **Make crew** in the bar
  at its foot.
- **On a phone**, long-press a session and choose **New crew…**.
- **When you launch** from a note, the launcher's **Crew** row offers
  **New crew: {the note's title}**, or any crew you already have.

**Moving sessions:**

- On a desktop, drag a session onto a crew to add it, between two members to
  put it there, or onto **RUNNING** or **ENDED** to take it out. Drag crews
  by their headers to reorder them. **Move to…** in the foot bar moves every
  selected session.
- Anywhere, a session's **About** has **Move to crew**, and on a phone the
  long-press sheet lists the crews.

Each crew has a menu, from the ··· on its header or a right-click (on a
phone, from ··· on the crew's screen): open its page, rename it, change its
colour, collapse it, start a new session in it, dismiss its ended sessions,
or end all of its running ones.

A crew's page shows its members, the hosts they run on, the notes they
started from and everything they wrote, newest first. **End all running**
lists every session it will stop before it stops any. A member on a host
that can't be reached now is listed as such and skipped; nothing is queued
for later.

**Agents a session started** belong to its crew with it: they are listed
under it, indented, count toward its running number and are ended by **End
all running**. They can't be moved on their own; where a move would be,
they say "moves with the agent that started it". Moving the session that
started them, or making a crew from it, takes them along.

When one of them stops at its own permission prompt, it shows **Needs you**,
and so do the session that started it and its crew. Tap the badge to open
that agent and answer in its terminal: no agent, and nothing in Storm,
answers it for you.

Collapsing a crew, or the whole CREWS section, is remembered on each device
separately. Everything else about a crew is kept on your server, so your
other devices show a change within about 15 seconds.

## When a crew empties

A crew with no sessions left is removed: when its last session is dismissed,
or moved to another crew. Storm says so, with **Undo**:

> Docs was removed: its last session was dismissed. **Undo**

Undo brings the crew back with the same name, colour and place in the list,
and if a move emptied it, moves that session back. A dismissed session
itself does not come back. Undo is offered for a few seconds; a crew holds
nothing but its sessions, so letting it go loses nothing else.

## Roles

A role is any note in your Kit vault's `agents/` folder, or in a project's
`projects/<name>/agents/` folder: the notes Kit calls its agent roles. The
role picker lists each with its summary, grouped by folder. A role is shown
as a small tag beside the session's name, and several sessions can share one.

Only you set a role. An agent cannot give itself one.

### The launch role and the label

**At launch**, the launcher's **Role** row hands the role note to the agent
along with the context note: the agent reads both through Storm when it
begins. That is the session's **launch role**, and it is fixed for the life
of the session. Resuming the session keeps it.

**Afterwards**, changing the role only changes the **label** on the
session's row, from **About › Change label** or a phone's long-press sheet.
The agent has already read its launch role and is not told about the new
label. When the two differ, About shows both: the role the agent read and
the label you set since.

A role set on a session that was launched without one is a label only, and
About says the agent didn't read it. Renaming a role note updates every tag
that uses it; a deleted role note leaves its tag struck through.

A role note can tell the agent how to behave, "write nothing in the vault"
for example. That is advice to the agent, not a rule: what a session may
write is still the **Can write to** you chose at launch. The launcher says so
under Role when you grant a write vault.

### Agents that can't read roles

An agent reads its role through Storm's tools, so an agent without them
can't read one. Today that is a **shell**, **Cursor** and **Command Code**
(which run as terminals only, see [sessions](/docs/agents/sessions/#what-an-agent-can-touch)).
For those the launcher's Role row is turned off with the reason, and a role
set later is a label only.
