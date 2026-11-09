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

Press **Start session** in a note's header, or **New session** in Agents. The
launcher asks for:

| Field | What it means |
|---|---|
| **Host** | Which Runtime Host runs it. |
| **Workspace** | A folder on that host — usually a project checkout. **New workspace…** makes one (see below). |
| **Agent** | Claude Code (the default), OpenCode, Codex, Cursor, Command Code or a shell. An agent the host does not have is listed as not installed. |
| **Model** | The model the agent runs, or empty for the agent's own default. Claude Code takes `haiku`, `sonnet`, `opus` or a full id such as `claude-sonnet-4-5`; OpenCode takes `provider/model`; Codex a model id such as `gpt-5-codex`. Not offered for other agents. The session's row shows it. |
| **Permission mode** | Which of the agent's own permission modes it starts in. **The agent's own setting** (the default) passes nothing, so whatever you set up for that agent applies. Claude Code also offers **Accept edits**, **Plan**, **Auto**, **Don't ask** (it refuses whatever its rules don't allow, without asking) and **Bypass permissions** (it never asks); OpenCode offers **Accept edits**. Storm passes the one you pick to the agent as its own setting and adds nothing of its own. Agents the session starts run in the same mode, or in their own setting when their agent doesn't have it. The session's row and About show it, and a resume keeps it. |
| **Context** | The note it starts from. The agent reads it through Storm when it begins; remove it to start blank. |
| **Can write to** | The one vault the agent may write to, or none (read only). Offered only when **Settings › AI access** allows writes chosen at launch, and never for a shell, Cursor or Command Code. |
| **Crew** | A [crew](/docs/agents/crews/) to list the session in, a new one named after the context note, or none. |
| **Role** | A role note from your Kit the agent reads at launch, as a [role](/docs/agents/crews/#roles). Off for an agent that can't read Storm notes. |
| **Agents** | **Allow this session to start agents**, off by default: the session may then start, message and end agents of its own, which run in its [crew](/docs/agents/crews/) with it. Offered for Claude Code, OpenCode and Codex on any host that says which account it runs as; a host too old to say, or one set to isolate sessions whose isolation isn't working, doesn't offer it. No key, API call or agent can turn it on: only you, here or in the session's About. |
| **Agents' writes** | Shown only with **Agents** on and a **Can write to** vault chosen, and off by default: **The agents it starts can write to {vault}**. Off, they are read only. On, they write only to that vault, never another. About shows which. |

Any number of sessions can work in one workspace. The launcher marks a
workspace another session is working in; they see each other's changes as
they make them.

**New workspace…**, at the end of the Workspace list, makes an empty folder
on the host, in the first of its `workspace_roots`, and selects it. Give it a
name (one folder name: no `/`, not starting with a dot) and choose whether to
start it as a git repository (`git init` on branch `main`, on by default). A
name already taken is refused; nothing is ever overwritten. On a host with
[session isolation](/docs/agents/hosts/) on, the folder is prepared for
isolation when the first session starts in it. An older storm-runtime
can't make workspaces; update it, or make the folder on the host.

The session takes its name from the agent's own terminal title, and its row
shows the agent's logo and whether it is working or idle.

Sessions working toward one goal can be grouped into a
[crew](/docs/agents/crews/), and each can carry a role label.

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

Beside the terminal, **Context**, **Wrote** and **About** show the note it
started from, the notes it has written, and how it was launched. A note an
agent wrote says so on its version line, linking back to the session.

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
there, saying why. Nothing is deleted until you answer: a dialog opens on
every device you're signed in on, with the agent, its session (and the note
it started from), what would go (a note's path, size and last change, or a
folder's note count and first paths) and the agent's reason, shown as plain
text.

- **Approve** moves it to the vault's [Trash](/docs/notes/vaults/#trash);
  **Decline** keeps it and tells the agent no. There is no "always allow",
  and Auto mode does not skip the dialog.
- **Later**, Esc or clicking outside only closes the dialog on that device:
  the request keeps waiting and isn't declined. It doesn't pop up again
  there on its own (a new request does); your other devices still show it.
- The first answer wins; the dialog on your other devices closes and says
  where it was answered. While it waits, the session shows **Needs you**,
  and **Review** reopens the dialog.
- If the note is edited or moved, or the folder's notes change, before you
  approve, nothing is deleted and the agent is told to look again.
- Unanswered, it times out after 10 minutes. An agent asks for one deletion
  at a time, and an agent another agent started can't delete at all.

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
  why, and the session's **Details** list it too.
- **Cursor and Command Code run as terminals only:** no notes, no
  integrations, no write vault, and no Resume yet, until Storm has checked
  its integrations in each with a signed-in account. Their own MCP servers
  work as usual. Their sessions keep the name they launched with.
- Storm enforces those limits on its tools. The agent is also a process with
  its host account's file access, which is why a Runtime Host must
  [never share an account with the vaults](/docs/agents/hosts/#never-share-an-account-with-the-vaults).
