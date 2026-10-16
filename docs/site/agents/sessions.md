---
title: Agent sessions
description: Run Claude Code, OpenCode or a shell on your own machines, from any of your devices.
---

A **session** is one agent running in a workspace on a
[Runtime Host](/docs/agents/hosts/): Claude Code, OpenCode or a shell. You
start it from Storm and drive its terminal from any of your devices — a
session started on the laptop can be picked up on the phone.

## Starting one

Press **Start session** in a note's header, or **New session** in Agents. The
launcher asks for:

| Field | What it means |
|---|---|
| **Host** | Which Runtime Host runs it. |
| **Workspace** | A folder on that host — usually a project checkout. |
| **Agent** | Claude Code (the default), OpenCode or a shell. |
| **Context** | The note it starts from. The agent reads it through Storm when it begins; remove it to start blank. |
| **Can write to** | The one vault the agent may write to, or none (read only). Offered only when **Settings › AI access** allows writes chosen at launch, and never for a shell. |

The session takes its name from the agent's own terminal title, and its row
shows the agent's logo and whether it is working or idle.

## The terminal

- **Images:** paste or drop an image into the session on desktop, the web and
  Android (including Gboard's image keyboard). It is staged on the host and
  handed to the agent as a file path.
- **Links** in the terminal open in your browser.
- **On a phone**, extra keys (Esc, Tab, arrows, Ctrl) sit above the keyboard.
- **Settings › Terminal** sets the text size, line spacing and padding.

Beside the terminal, **Context**, **Wrote** and **About** show the note it
started from, the notes it has written, and how it was launched. A note an
agent wrote says so on its version line, linking back to the session.

## When it ends

An ended Claude Code or OpenCode session can be **resumed**: it reopens the
agent's own conversation on the same host and workspace. **Run again** starts
a fresh session with the same choices; **Dismiss** clears it from the list.

## What an agent can touch

- It reads your notes through Storm's tools, and writes only to the vault
  chosen at launch. It is never offered `delete_note`.
- It can use the [integrations](/docs/agents/integrations/) allowed on its
  host, and only their allowed tools. A shell session gets none.
- Storm enforces those limits on its tools. The agent is also a process with
  its host account's file access, which is why a Runtime Host must
  [never share an account with the vaults](/docs/agents/hosts/#never-share-an-account-with-the-vaults).
