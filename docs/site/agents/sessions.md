---
title: Agent sessions
description: Run Claude Code, OpenCode, Codex, Cursor, Command Code or a shell on your own machines, from any of your devices.
---

A **session** is one agent running in a workspace on a
[Runtime Host](/docs/agents/hosts/): Claude Code, OpenCode, Codex, Cursor,
Command Code or a shell. You
start it from Storm and drive its terminal from any of your devices — a
session started on the laptop can be picked up on the phone.

## Starting one

Press **Start session** in a note's header, or **＋** beside **Agents** at the
top of the Agents sidebar (`⌘ ⇧ N`, `Ctrl+Shift+N` on Windows and Linux). The
launcher asks for three things:

| Field | What it means |
|---|---|
| **From** | The note it starts from, when there is one. The agent reads it through Storm when it begins; remove it to start blank. |
| **Agent** | Claude Code (the default), OpenCode, Codex, Cursor, Command Code or a shell. An agent the host does not have is listed as not installed. Under it, for Claude Code, which Claude login the session will use. |
| **Where** | A workspace and the host it is on, such as *storm on build-vm*. The list groups each online host's workspaces under its name. Under it: who the host runs sessions as, and whether another session is working in that workspace. **New workspace…** makes one (see below). |

Under them, **This session can** sums up the rest as a row of chips:
**Writes to {vault}**, **Starts agents**, **Uses {n} integrations** and
**Network: {host}'s policy**. A chip that is on has an accent outline; one
that is off is plain. Tap a chip to open its setting in **More**. A chip the
agent can't have stays, greyed out, and tapping it says why. Everything else
uses the agent's defaults.

**More: model, permission mode, crew, role ›** opens the rest in place:

| Field | What it means |
|---|---|
| **Write to** | The one vault the agent may write to, or none (read only). Only when **Settings › AI access** allows writes chosen at launch; off, and saying why, for a shell, Cursor or Command Code. |
| **Start agents** | On by default (turn it off to launch without): the session may start, message and end agents of its own, up to the host's limits (3 at once and 6 in its tree unless you changed them), which run in its [crew](/docs/agents/crews/) with it. Claude Code, OpenCode and Codex, on any host that says which account it runs as; a host too old to say, or one set to isolate sessions whose isolation isn't working, can't offer it, and the row says so. No key, API call or agent can turn it on: only you, here, from the session header's **Starts agents ▾**, or in its About. |
| **Let the agents it starts write to {vault}** | Off by default, and only with **Start agents** and **Write to** on. Off, they are read only. On, they write only to that vault, never another. About's **Starts agents** says which. |
| **Model** | The model the agent runs, or empty for the agent's own default. For Claude Code, **opus**, **sonnet** and **haiku** fill it in; it also takes a full id such as `claude-sonnet-4-5`. OpenCode takes `provider/model`; Codex a model id such as `gpt-5-codex`. A shell, Cursor and Command Code have none. The session's header shows it. |
| **Permission mode** | Claude Code and OpenCode only; Tab and ⇧Tab change it in the terminal. Which of the agent's own permission modes it starts in. **The agent's own setting** (the default) passes nothing, so whatever you set up for that agent applies. Claude Code also offers **Accept edits**, **Plan**, **Auto**, **Don't ask** (it refuses whatever its rules don't allow, without asking) and **Bypass permissions** (it never asks); OpenCode offers **Accept edits**. Storm passes the one you pick to the agent as its own setting and adds nothing of its own. Agents the session starts run in the same mode, or in their own setting when their agent doesn't have it. The session's header shows it, and a resume keeps it. |
| **Crew** | A [crew](/docs/agents/crews/) to list the session in, a new one named after the **From** note, or none. Opened from a crew, More starts open with that crew chosen. |
| **Role** | A role note from your Kit the agent reads at launch, as a [role](/docs/agents/crews/#roles). Off for an agent that can't read Storm notes. |

More also has the whole network sentence: the session inherits the host's
network policy, and can use your integrations (named) and read your vaults
with that access. Switching agents never adds or removes a row: a row the
agent can't use stays, greyed out, with the reason under it.

On a phone the launcher is a sheet with the same three fields and chips, and
More opens in the sheet.

Any number of sessions can work in one workspace. The launcher marks a
workspace another session is working in; they see each other's changes as
they make them.

**New workspace…**, under Where or at the end of its list, opens a field
right there (not a separate window) that makes an empty folder on the host,
in the first of its `workspace_roots`, and selects it. Give it a name (one folder name: no `/`, not starting with a dot) and choose whether to
start it as a git repository (`git init` on branch `main`, on by default). A
name already taken is refused; nothing is ever overwritten. On a host with
[session isolation](/docs/agents/hosts/) on, the folder is prepared for
isolation when the first session starts in it. An older storm-runtime
can't make workspaces; update it, or make the folder on the host.

The session takes its name from the agent's own terminal title, and its row
shows the agent's logo and whether it is working or idle.

Sessions working toward one goal can be grouped into a
[crew](/docs/agents/crews/), and each can carry a role label.

## Reading the sidebar

On a desktop, each row in the Agents sidebar says one thing per signal:

- **A red outline** means the session is waiting for you: a permission
  prompt, a question or a delete request. It is the only thing counted on
  the Agents icon in the rail.
- **The badge on the agent's mark** says what it is doing: a pulsing dot
  while it works, a ring while it's idle, grey while it starts, red once it
  failed. A parked session, one out of usage and one whose host is offline
  have none, and their names are dimmed.
- **The line under the name** says why, in words — "permission prompt ·
  3m", "blocked · waiting on test-gateway", "failed · host restarted", "out
  of usage · resets 15:00", "host offline · last seen 9m" — and otherwise
  the workspace and how long it has run. A child whose work you haven't
  applied says "2 commits to apply" and stays out of **ENDED** until you
  apply or delete it.
- **A dot after the name** means output you haven't seen on this device.

Sessions waiting for you, blocked, failed in the last hour or on an offline
host move to the top of their crew or section, and crews with one move up
too; everything else keeps your order. Nothing moves while the pointer is
over the sidebar. A spawned tree moves as one, and its parent's chevron
folds it to a line such as "2 agents · 1 needs you". **ENDED** starts folded
to its count, with **Dismiss all**; open it and it stays open on that
device.

## The terminal

- **Images:** paste or drop an image into the session on desktop, the web and
  Android (including Gboard's image keyboard). It is staged on the host and
  handed to the agent as a file path.
- **Links** in the terminal open in your browser.
- **Tab and Shift+Tab go to the agent** while the terminal has focus: in
  Claude Code, Shift+Tab switches between Manual, Accept edits and Plan. To
  move focus out of the terminal with the keyboard, press **Escape, then
  Tab**, or click outside it.
- **On a phone**, extra keys (Esc, Tab, ⇧Tab, arrows, Ctrl) sit above the
  keyboard.
- **Settings › Terminal** sets the text size, line spacing and padding.

Under the terminal, a line says how to leave it: *Tab and Shift+Tab go to
the agent · Esc, then Tab, leaves the terminal* (⇧Tab on a Mac).

Beside the terminal, at desktop width, an icon rail opens the session's
inspector: **Context** (the note it started from), **Wrote** (the notes it
has written, with their count on the icon), **Agents** (for a session that
can start agents or has), **About**, and **Work** for an agent with a
workspace of its own. Each icon names itself when you point at it. Clicking
the open tab's icon closes the panel, as does **×** or the header's details
button; the rail stays, so the terminal takes the room and the counts stay
in sight. Drag the panel's left edge to resize it. On a phone, tap the
session's name for the same details in a sheet. A note an agent wrote says
so on its version line, linking back to the session.

**About** says who and what the session is: its **Crew** (with **Move to
crew**), its **Role** (with **Change label**), whether it **Starts agents**
and within what limits, who it **Ran as**, and who **Started** it. A plugin
server the host left out is one line there, with **Fix in Integrations ›**.
The agent, workspace, model and mode are on the header's second line, and a
host's identity and isolation in its details in **Settings › Agents & hosts**.

**Agents** lists the agents the session started, each with its state in a
word (**Needs you**, **Running**, **Parked** or **Ended**), why, and **Open**
(or **Work** when an ended one has commits to apply), then **End all of
them…**, which asks under the header before it ends any. Below, **What it
sent them** is every start, message and end the session's agent sent, oldest
first, with the exact text it sent, and under each one that began a round
what the round took (*round 2 · 14s · 12.3k in*) where the agent counted it.

About's **Ran as** row is the account the session ran as, kept from when it
started running, so it stays as it was if the host changes later. On an
isolated host it reads *"Its own session user (isolated)"*, with the host's
account after it. A session from before Storm kept this reads *"Not
recorded"*; one whose host couldn't name its account reads *"An unknown
account"*.

## Switching modes

Switch modes in the terminal, as you would outside Storm: in Claude Code,
Shift+Tab cycles through the modes its own settings allow (the touch key
bar has **⇧Tab**). Storm doesn't add or remove any.

### Deletes wait for you

An agent that may write to a vault can ask to delete a note or a folder
there, saying why. Nothing is deleted until you answer. The request waits in
a strip under the session's header, never in a pop-up: **NEEDS YOU · 1 OF 1**
(or "1 of 2" when more is waiting), "*session* wants to delete *note*", the
agent's reason word for word, what would go (a note's path, size and last
change, or a folder's note count and first paths), and where it goes if you
approve: the vault's Trash, kept for as long as that vault keeps deleted
items.

- **Move to Trash** moves it to the vault's [Trash](/docs/notes/vaults/#trash),
  where you can restore it; **Decline** keeps it and tells the agent no.
  There is no "always allow", and Auto mode does not skip it.
- **Later** (or Esc, once the terminal doesn't have the key) folds the strip
  to one line with **Review**: the request keeps waiting and isn't
  declined.
- On any other screen, or a device not showing that session, a request
  raises one short message, "*session* wants to delete *note* · Review",
  once per request on each device. **Review** opens the session. The
  Agents button's count includes it while it waits.
- On a phone the strip is one line under the header; **Review** opens the
  whole request in a sheet with Move to Trash, Decline and Later.
- The first answer wins; if the session is open on another device, its
  strip goes and says where it was answered.
- If the note is edited or moved, or the folder's notes change, before you
  approve, nothing is deleted and the agent is told to look again.
- Unanswered, it times out after 10 minutes. An agent asks for one deletion
  at a time, and an agent another agent started can't delete at all.

Ending a session that started agents, ending all of them, and dismissing a
tree of sessions ask in the same strip: **CONFIRM**, with what is still
running, what already ended (and any commits not yet applied), and the
button that does it.

## When it ends

An ended Claude Code, OpenCode or Codex session can be **resumed**: it reopens the
agent's own conversation on the same host and workspace. **Run again** starts
a fresh session with the same choices; **Dismiss** clears it from the list.

## What an agent can touch

- It reads your notes through Storm's tools, and writes only to the vault
  chosen at launch. It deletes there only when you
  [approve it](#deletes-wait-for-you).
- It can use the [integrations](/docs/agents/integrations/) allowed on its
  host, and only their allowed tools. A shell session gets none.
- **Claude Code keeps its own MCP servers:** the ones in your
  `~/.claude.json`, its [plugins'](/docs/agents/plugins/), and a
  repository's `.mcp.json` (after Claude Code asks you) load beside your
  integrations, unless the host sets `agent_mcp = "storm-only"`.
- **OpenCode keeps your global config**, servers included, beside your
  integrations. A repository's own OpenCode config stays off: it could turn
  off the question OpenCode asks before each Storm tool.
- **Codex keeps its own MCP servers, [plugins](/docs/agents/plugins/) and
  apps** beside your integrations, and asks its own trust questions about a
  repository's `.codex/` settings.
- A host with `agent_mcp = "storm-only"` gives sessions Storm's servers
  alone. When such a session starts, the agent shows what Storm left out and
  why, and the session's **About** says so in one line.
- **Cursor and Command Code run as terminals only:** no notes, no
  integrations, no write vault, and no Resume yet, until Storm has checked
  its integrations in each with a signed-in account. Their own MCP servers
  work as usual. Their sessions keep the name they launched with.
- Storm enforces those limits on its tools. The agent is also a process with
  its host account's file access, which is why a Runtime Host must
  [never share an account with the vaults](/docs/agents/hosts/#never-share-an-account-with-the-vaults).
