---
key: readme
kind: index
status: active
summary: What the kit vault is for — reusable, agent-facing tooling kept separate from content vaults. Lists the layout spec, the agent roles, the skills folder, and the rule that a note's folder decides whether it installs globally or into one project.
tags: [kit, index]
---

# kit

**Reusable, agent-facing tooling.** Patterns, procedures, role definitions and
keyword-triggered skills. This is the **shared source of truth** — every agent
host reads these notes rather than keeping a private copy.

Kept separate from your content vaults so that tooling does not clutter real
notes, and searching for content does not turn up machinery.

This vault is created and seeded automatically when a Storm server first
starts. **It is yours from that moment** — edit it, extend it, delete what you
do not use. The server will not overwrite your changes.

## The layout spec

| Note | What it is |
|---|---|
| [[Project Architecture Guidelines]] | What a laid-out project looks like: note kinds, folder shape, frontmatter, the task-record format, sizing rules, write rules |

Everything else here reads that note rather than restating it.

## The agents

Five roles. Each exists because it has a **distinct read set and write
surface** — that is the test for whether a role is real or just a prompt with a
different personality.

| Agent | Runs | Reads | Writes in the vault |
|---|---|---|---|
| [[Storm Architect]] | main loop | source docs, conversation | `spec/`, `CONVENTIONS`, `INDEX` |
| [[Storm Lead]] | main loop | `work/`, `BOARD` | `work/` state, `log/`, the mirrors |
| [[Storm Researcher]] | subagent | a spike, `spec/`, the world | proposes a `spec/` edit; never applies it |
| [[Storm Coder]] | subagent | one task + its `reads:` set | **nothing** |
| [[Storm Reviewer]] | subagent | `done-when:` + the diff | **nothing** |

Two rules hold the set together:

- **The lead is the only writer to `work/`.** That is what lets several coders
  run at once without racing on the same milestone note.
- **Coders and reviewers write nothing in the vault.** They return results; the
  lead records them.

## The skills

`skills/` holds **keyword-triggered procedures** — the repeatable workflow that
is not tied to one role. Same idea as a host's own skill format, but stored
here so every host resolves one source of truth.

One note per skill. Filename lowercase and hyphenated, matching the `name` the
host will use, so there is a single handle across systems. Frontmatter carries
`key: skill.<name>`, `kind: skill`, and `domain: <area>`.

## Scope: global, or one project

**A note's folder decides where its adapter installs.** That is the whole rule.

```text
agents/                  every project        →  the host's global path
skills/                  every project        →  the host's global path
projects/<name>/agents/  that project only    →  that repo's local path
projects/<name>/skills/  that project only    →  that repo's local path
```

So a role every project needs lives in `agents/` and installs to
`~/.claude/agents/` or `~/.config/opencode/agents/`. A role only one codebase
needs lives in `projects/<name>/agents/` and installs to that repo's
`.claude/agents/` or `.opencode/agents/`, where it loads for that project and
nowhere else.

Two consequences worth stating plainly:

- **Put it in `projects/<name>/` when it names something only that project
  has** — its schemas, its vendors, its deploy targets. A global folder is for
  what survives moving to a different codebase.
- **The vault is the source of truth for both.** Project-scoped tooling is
  still versioned, synced and editable from anywhere; only its install path
  and its blast radius are narrower.

An installer that ignores `projects/` still produces a working global setup.
One that reads it produces the setup that project actually wants.

## What does not belong here

- **Projects.** Those go in a content vault. `kit` describes *how* a project is
  laid out; it never holds one.
- **A project's own rules and content.** Those live with the project, in its
  `CONVENTIONS.md` and its content vault. Project-scoped *tooling* is the
  exception and has a home here — `projects/<name>/` — but the project itself
  never does.
- **Personal notes.** Different vault, different purpose.

## How agents reach this

An agent host needs a local file to discover and trigger an agent, and Storm
cannot execute anything — so each host gets a **thin adapter** generated from
these notes. The adapter points here; the substance stays in the vault, where
it syncs, versions, and can be edited from anywhere.

Install instructions per host ship with Storm under `kit/install/`.

## A note on wikilinks

Wikilinks resolve **within** a vault only. The links on this page work; a
project in another vault cannot `[[link]]` here and must use a plain reference
plus a uuid instead.
