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
| **Permission mode** | How much the agent may do without asking you: **Manual** (the agent asks; the default), **Accept edits** (file edits go ahead without a prompt; shell commands still ask) or **Auto** (the agent's own safety check decides routine calls, shell commands included). Claude Code offers all three, OpenCode Manual and Accept edits; other agents always ask. There is never an option that turns the agent's prompts off. Agents the session starts get the same mode or a stricter one. The session's row and About show it, and a resume keeps it. |
| **Context** | The note it starts from. The agent reads it through Storm when it begins; remove it to start blank. |
| **Can write to** | The one vault the agent may write to, or none (read only). Offered only when **Settings › AI access** allows writes chosen at launch, and never for a shell, Cursor or Command Code. |
| **Crew** | A [crew](/docs/agents/crews/) to list the session in, a new one named after the context note, or none. |
| **Role** | A role note from your Kit the agent reads at launch, as a [role](/docs/agents/crews/#roles). Off for an agent that can't read Storm notes. |

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

## Switching to Auto

Unless you launched a Claude Code session in **Auto**, Auto is not among the
modes Shift+Tab cycles through. The agent can ask for it: a card on the
session shows its reason, word for word, with **Approve Auto** and **Deny**,
and expires after 10 minutes. Approving restarts the session in Auto with
its conversation kept, and takes you to it. An agent another agent started
can ask only while that agent is in Auto, and a session with agents of its
own running can't ask until they end.

## When it ends

An ended Claude Code, OpenCode or Codex session can be **resumed**: it reopens the
agent's own conversation on the same host and workspace. **Run again** starts
a fresh session with the same choices; **Dismiss** clears it from the list.

## What an agent can touch

- It reads your notes through Storm's tools, and writes only to the vault
  chosen at launch. It is never offered `delete_note`.
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
