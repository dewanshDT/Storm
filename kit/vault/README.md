---
key: readme
kind: index
status: active
summary: What the kit vault is for — reusable, agent-facing tooling kept separate from content vaults. Lists the layout spec and the five agent roles, and states what does not belong here.
tags: [kit, index]
---

# kit

**Reusable, agent-facing tooling.** Patterns, procedures and role definitions
that apply across every project.

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

## What does not belong here

- **Projects.** Those go in a content vault. `kit` describes *how* a project is
  laid out; it never holds one.
- **Anything project-specific.** A project's own rules live in its
  `CONVENTIONS.md`.
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
