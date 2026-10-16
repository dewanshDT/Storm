---
title: Concepts
description: The parts of Storm and the words these docs use for them.
---

## How it fits together

```text
macOS · Android · Web  AI applications
       │ REST + WS        │ MCP
       ▼                  ▼
  storm-server (Rust / axum)  ◄──────  storm-runtime
       │                     dials out Claude Code ·
  ┌────┴─────┐                         OpenCode · shell
  ▼          ▼
vaults/    state/
*.md       indexes · auth.db ·
           agent.db · gateway.db
```

The apps talk to `storm-server` over HTTP and a live WebSocket; AI apps use
its MCP endpoint; Runtime Hosts connect out to it. Notes stay ordinary files
under `vaults/`, and the server's own records sit beside them in `state/`.

## The server

`storm-server` is a program you run on Linux, macOS or Windows. It owns your
notes: it merges edits from different devices, keeps each note's version
history, and answers searches. Everything else talks to it.

## Vaults and notes

A **vault** is a folder of ordinary Markdown files under the server's storage
root. You can open, grep, back up or version it with any tool. Storm's own
state — indexes, the account, agent and integration records — lives in a
separate `state/` folder beside the vaults, never inside them.

A **note** is one Markdown file. Storm tracks it by an id in its frontmatter,
so renaming or moving it keeps its history.

## Your account and devices

Storm has **one account**, with one password. Each app you sign in — on a
phone, a laptop, a browser — is a **device** with its own credential, which
you can revoke on its own under **Settings › Devices & access**. Nothing goes
through a cloud account.

## Sync and offline

Each device keeps a copy of the notes it has seen and a queue of changes made
offline, and sends them when the server is back. When two devices change the
same note, the server merges the edits. If it cannot, the conflict is written
into the note itself, where you can see it — not into a hidden copy.

## Runtime Hosts and sessions

A **Runtime Host** (`storm-runtime`) runs coding agents on a machine you
choose. It connects out to the server, so it needs no open port, and it holds
no notes: an agent reads and writes them through the server, like any other
client.

A **session** is one agent — Claude Code, OpenCode or a shell — running in a
workspace on a host. You can start one from a note, which becomes its
context, and drive its terminal from any device. An agent can write only to
the vault you chose when you started it, and only if **Settings › AI access**
allows agents to write at all. An ended Claude Code or OpenCode
session can be resumed.

## Integrations and MCP

**Integrations** are other MCP servers, such as Notion or Linear, that you
connect once in the app. The server keeps their credentials, encrypted, and
agents call them through the server. You choose which Runtime Hosts may use
each one, and which of its tools are allowed.

**MCP access** works the other way round: it lets AI apps outside Storm read
— and, if you allow it, write — your notes. Each app gets its own **key**,
shown once and revocable.

## Relays

A **relay** lets your devices reach the server from outside your network
without opening a port: the server connects out to the relay, and the app
tries the server's own address and its relays and uses whichever answers
first. A relay stores nothing and checks no credentials, but until end-to-end
encryption it can read what it carries, so run your own or use one you trust.
