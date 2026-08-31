# Storm Agents

A set of agent roles for working on projects stored in Storm, plus the layout
spec they operate on.

Point any coding agent — Claude Code, opencode, cursor-agent — at these and it
knows how to use Storm as intended: where design lives, where progress lives,
what each role may read, and what it may write.

> Not to be confused with the repo-root `AGENTS.md`, which tells agents how to
> work **on Storm itself**. This directory is about working on **projects
> stored in** Storm.

## The idea

A project in Storm is laid out so that an agent can start any single task by
reading **four notes or fewer**. That falls out of two facts about Storm:
`get_note` has no partial read, and `update_note` replaces the whole body. So
note size is the cost of a lookup, and anything written often must be small and
kept away from anything large and stable.

```text
spec/    what is true      slow, read-mostly, large
work/    what is done      fast, small, written constantly
log/     what happened     append-only
```

## The roles

Each exists because it has a **distinct read set and write surface**. That is
the test — a role without its own row is a prompt with a personality, not an
agent.

| Agent | Runs | Reads | Writes in the vault |
|---|---|---|---|
| **Architect** | main loop | source docs, conversation | `spec/`, `CONVENTIONS`, `INDEX` |
| **Lead** | main loop | `work/`, `BOARD` | `work/` state, `log/`, the mirrors |
| **Researcher** | subagent | a spike, `spec/`, the world | proposes a `spec/` edit; never applies it |
| **Coder** | subagent | one task + its `reads:` set | **nothing** |
| **Reviewer** | subagent | `done-when:` + the diff | **nothing** |

Two rules hold it together:

- **The lead is the only writer to `work/`** — which is what lets several
  coders run at once without racing on the same milestone note.
- **Coders and reviewers write nothing in the vault.** They return results;
  the lead records them.

Architect and lead run in the main loop because both **ask the user questions**,
and a subagent cannot.

## Install

Your Storm server creates and seeds a `kit` vault on first start, so the notes
are already there. Then:

1. Configure Storm as an MCP server in your agent.
2. Find your ids: `list_vaults` → the vault named `kit`, then `search` it for
   the agent note ids.
3. Follow the guide for your host:

   - [`install/claude-code.md`](install/claude-code.md)
   - [`install/opencode.md`](install/opencode.md)
   - [`install/cursor.md`](install/cursor.md)

Or paste [`BOOTSTRAP.md`](BOOTSTRAP.md) into your agent and let it do all
three.

Every adapter is a **thin loader** — it fetches the role note and follows it.
Editing a note in Storm changes behaviour everywhere, with no reinstall.

## Host capability differences

These are real, and the adapters are honest about them rather than pretending
parity:

| | Claude Code | opencode | cursor-agent |
|---|---|---|---|
| main-loop roles | skills | `mode: primary` | manual rule |
| subagent roles | `.claude/agents/` | `mode: subagent` | **none** |
| parallel workers | yes | yes | **no** |
| enforce write surface | tool allow-list | **permission rules** | instruction only |

opencode can *deny* a coder the vault write tools. Claude Code can omit them
from the agent's `tools:` list. Cursor can only ask nicely.

## What is here

```text
kit/
├── README.md          this file
├── BOOTSTRAP.md       paste into an agent to install everything
├── vault/             seeded into your kit vault on first server start
│   ├── README.md
│   ├── Project Architecture Guidelines.md    the layout spec
│   └── agents/                               the five roles
└── install/           per-host adapter instructions
```

`vault/` is the canonical source. Once seeded, **your copy is yours** — edit
it, extend it, delete what you do not use. The server will not overwrite it.
