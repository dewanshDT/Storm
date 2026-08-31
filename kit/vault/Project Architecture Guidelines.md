---
key: guidelines
kind: guidelines
status: active
summary: How to lay out any project in Storm so that both a human and an agent can work it — the five note kinds, the spec/work split, the checkbox task-record format, frontmatter schemas, sizing rules, the orchestrator loop contract, and the write rules.
tags: [kit, guidelines, agents, storm, project-structure]
---

# Project Architecture Guidelines

How to lay a project out in a Storm vault so that it stays readable to a human
**and** cheap for an agent to work from.

This note is **project-agnostic**. Everything specific to one project — its ID
prefix, its area vocabulary, its milestone list — lives in that project's own
`CONVENTIONS.md`.

---

## 0. Applying this

This note is the **output spec** — what a laid-out project looks like. The
**procedure** for producing one from architecture documents is
[[Storm Architect]] in this vault.

That split is deliberate and matches the rest of these rules: the architect
reads this note rather than restating it, so the layout has exactly one home.

Storm stores; it does not execute. Each agent host gets a thin adapter that
points at these notes — see `kit/install/` in the Storm distribution.

---

## 1. The constraint that drives everything

Storm has two properties that decide the whole design. Do not optimise them
away:

1. **There are no partial reads.** `get_note` returns the entire note. There is
   no way to fetch one section. **Note size is therefore the token cost of a
   lookup**, and chunking is the only lever.
2. **`update_note` replaces the whole body.** Writing one character means
   sending the whole note back. A note that is written often must be small, and
   must not be carrying anything large that does not change.

Two consequences, and every rule below follows from them:

- **Split by read profile, not by topic.** What gets read together lives
  together.
- **Separate the slow substrate from the fast ledger.** Design notes are large,
  stable, read-mostly. Progress notes are small, volatile, write-often. Putting
  a checkbox inside a design note makes every tick an expensive, risky rewrite
  of content that did not change.

---

## 2. The five kinds of note

Every note declares `kind:` in its frontmatter.

| kind | Answers | Size | Written |
|---|---|---|---|
| `index` | *Where is everything, and what do I read for this task?* | ≤2KB + map | by agent, regenerated |
| `conventions` | *What are this project's rules, ids, areas?* | ≤2KB | rarely |
| `spec` | *What is true?* | ≤3KB target | rarely, deliberately |
| `work` | *What is done, what is next?* | ≤2KB | constantly |
| `log` | *What happened?* | append-only | constantly |

The distinction that keeps this coherent:

> **The log records what happened. The spec records what is true.**

A finding lands in the log. If the finding changes what is true, **edit the
spec note and say so in the log**. Never leave the spec stale and the truth
only in the journal — the next agent reads the spec.

---

## 3. Folder shape

```text
<Project>/
├── INDEX.md          kind: index        the only always-load note
├── CONVENTIONS.md    kind: conventions  this project's rules
│
├── spec/             kind: spec         design. slow. authoritative for truth.
│   └── <one note per subsystem>
│
├── work/             kind: work         progress. fast.
│   ├── BOARD.md                         rollup — counts, current, blocked
│   ├── CHECKLIST.md                     every task, flat and tickable
│   ├── M0.md … Mn.md                    one note per milestone, holds tasks
│   └── spikes.md                        open empirical questions + results
│
└── log/
    └── YYYY-MM.md    kind: log          append-only journal
```

Keep note **filenames project-prefixed** (`Widget Protocol.md`, not
`Protocol.md`). A vault holds many projects; generic titles collide in search
and make wikilinks ambiguous. The short handle for agents is the `key:` field,
not the filename.

**BOARD and CHECKLIST are generated mirrors.** Milestone notes are the source
of truth for a task; those two are regenerated from them, the same way the
INDEX's uuid map is. Worth the duplication because "where are we" is the single
most-asked question and neither a human nor an agent should have to open eight
notes to answer it.

---

## 4. Frontmatter schemas

Uniform frontmatter lets an agent *parse* rather than *read*.

**Every note:**

```yaml
key: <stable handle>        # spec.protocol, work.m2, index — never changes
kind: index|conventions|spec|work|log
area: [<area>, ...]         # optional; the areas this note concerns
summary: <one line>         # used for relevance during search
tags: [...]
```

**`spec` adds:**

```yaml
status: locked | proposed | superseded
answers: <what this note is authoritative for, one line>
```

`answers:` is the routing signal. It is what the INDEX quotes, and what lets an
agent decide *not* to read a note.

**`work` adds:**

```yaml
milestone: M2
state: not-started | in-progress | blocked | done
```

Do not put a per-note `size:` field — it drifts. Sizes live in the INDEX, which
is regenerated.

---

## 5. The task record

The unit of assignable work. Lives inside its milestone note. **A markdown
checkbox, not a heading** — headings do not render as task lists, and a plan
you cannot tick is a document rather than a tracker.

```markdown
- [ ] **<PREFIX>-M2-04** · Layout validator: overlap + bounds
  - state: todo · area: config · deps: WG-M2-02
  - reads: spec.data-model §5, spec.invariants #9
  - done-when: overlapping spans rejected as a structural error; fixture
    tests/fixtures/overlap.yaml fails validation; a valid input is unaffected
  - notes: —
```

**The checkbox and `state:` are not redundant.** The checkbox is the binary a
human scans; `state:` carries what a checkbox cannot express — `doing`,
`blocked`, `review`. The rule that keeps them honest:

> **`[x]` iff `state: done`.** Tick the box in the same write that sets it.

Each field earns its place:

- **`reads:`** is the token optimisation. It names *exactly* the notes and
  sections a worker needs. A worker reads INDEX + CONVENTIONS + its milestone
  note + the `reads:` set, and nothing else.
- **`done-when:`** is the acceptance criterion, written *before* the work.
  Verification then never has to re-derive intent — which is where scope
  quietly changes.
- **`deps:`** is what makes scheduling mechanical rather than judged.
- **`state:`** lives here and **only** here. Mirrors carry aggregates.

### ID scheme

```text
<PREFIX>-<MILESTONE>-<NN>      WG-M2-04
```

Stable forever. Referenced from commits, from the board, from acceptance
criteria. Never renumber — if a task dies, mark it `dropped`, do not reuse the
id.

### State vocabulary

```text
todo → doing → review → done
             ↘ blocked ↗
```

Five values plus `dropped`, no others. `blocked` requires a reason in `notes:`.

---

## 6. Sizing rules

- **Spec note target ≤3KB.** Above that, ask whether it holds two things.
- **Split a spec note when a section is both independently implementable and
  >2KB.** Both conditions. A large section that is always read with its
  neighbours should stay put — splitting it just adds a second fetch.
  **One exception worth taking:** a small section that is *volatile* — a
  platform delta, an unresolved spike — earns its own note even under 2KB,
  because it is the part that gets rewritten and it should not drag a stable
  contract through every edit.
- **Do not otherwise split below ~1KB.** Fetch overhead and link maintenance
  stop paying.
- **Work note ≤2KB**, roughly 8–12 tasks. If a milestone exceeds that, it is
  two milestones.
- **INDEX ≤2KB of routing content**, plus the generated `key → uuid` map. The
  map grows with note count and is not prose; the routing half is what must
  stay small, because it is loaded on every single task. If *that* half is
  growing, it is accumulating content that belongs in a spec note.

---

## 7. The INDEX is a routing table, not a file listing

It is keyed by **task type**, not by document. An agent should be able to read
it and know what to fetch without a search call.

Required contents:

1. One-line statement of what the project is.
2. A routing table: *working on X → read these*.
3. A `key → answers` table, one line each, lifted from each spec note's
   `answers:` field.
4. Pointers to `work/BOARD`, `work/CHECKLIST` and `CONVENTIONS`.
5. A fenced `key → uuid` map at the end.

The uuid map is what turns `search` + `get_note` into just `get_note`, halving
calls on a cold start. It makes the INDEX a **build artifact** — regenerating
it is a required capability of whatever agent maintains the project.

---

## 8. Areas

Areas are the assignment axis and cut **across** milestones. Define the
vocabulary once, in `CONVENTIONS.md`.

Tag each task with `area:`. Do **not** build a second set of notes indexed by
area — that is duplicated state and it will drift. "Every open task in
`platform/linux`" is a vault search over the `area:` field, not a maintained
document.

---

## 9. The orchestrator loop

The contract [[Storm Lead]] implements:

```text
1. read INDEX + CONVENTIONS + work/BOARD
2. pick the next task whose `deps` are all `done`
3. dispatch a worker with: the task record + only the notes its `reads:` names
4. worker returns a result
5. tick the box and set `state` in the milestone note   ← single source
6. append one line to log/YYYY-MM
7. regenerate BOARD and CHECKLIST
8. repeat
```

**Only the lead writes to `work/`.** That is what makes one-note-per-milestone
safe: with a single writer there is no contention, and the note stays small and
browsable. If you ever need many concurrent writers, that is the moment to
split tasks into their own notes — not before.

---

## 10. Write rules

Non-negotiable:

- **Read the note, edit the text you read, send the whole thing back.**
  `update_note` replaces the body. Sending only the section you changed
  destroys the rest of the note. Check the returned `size` looks right.
- **Always send `base_version`** so a concurrent edit merges instead of being
  clobbered. If the result says merged or conflict, **re-read before editing
  again** — the returned content is the server's.
- **The milestone note wins.** BOARD and CHECKLIST are mirrors; when they
  disagree with a milestone note, regenerate them rather than trusting them.
- **Never put design in a work note.** If a task record starts explaining *why*,
  that paragraph belongs in a spec note and the task should reference it.
- **Never put progress in a spec note.** No checkboxes, no "done", no dates.
- **A task is not done until its state is written.** When Storm is the only
  state of record, an unwritten tick is a lost tick.
- **Create before delete.** When restructuring, create and verify the new notes
  first. Storm has no trash; a deleted note is gone immediately.
- **Wikilinks do not cross vaults.** A project in one vault cannot `[[link]]`
  to this note. Reference it by vault name and note title instead, and let
  agents resolve it by uuid.

---

## 11. Minimal worked example

```text
Widget/
├── INDEX.md
├── CONVENTIONS.md        prefix: WG · areas: api, ui, store
├── spec/
│   ├── Widget Architecture.md      answers: what the system is, the boundaries
│   └── Widget Data Model.md        answers: the schema and its validation
├── work/
│   ├── BOARD.md                    M0 done · M1 4/9 · 1 blocked
│   ├── CHECKLIST.md                all 20 tasks, flat
│   ├── M0.md   M1.md
│   └── spikes.md
└── log/2026-09.md
```

`work/M1.md`:

```markdown
- [ ] **WG-M1-03** · Reject duplicate ids at load
  - state: todo · area: store · deps: WG-M1-01
  - reads: spec.data-model §2
  - done-when: two records sharing an id fail validation with a structural
    error naming both; single-id load unaffected
```

A worker assigned `WG-M1-03` fetches INDEX, CONVENTIONS, `work/M1`, and
`spec.data-model`. Four notes, ~7KB. It never sees the architecture note,
because nothing it needs is in there.

---

## 12. Applying this to a new project

1. Write `spec/` first. Design before tasks — tasks derived from a vague spec
   are vague tasks.
2. Write `CONVENTIONS.md`: prefix, areas, milestone list.
3. Derive `work/` milestone notes from the spec. Every task gets `reads:` and
   `done-when:` at the moment it is written, not later.
4. Generate `BOARD.md` and `CHECKLIST.md` from those.
5. Generate `INDEX.md` last, once paths and uuids exist.
6. Open `log/YYYY-MM.md` with the first entry.

The test of whether the layout is right: **an agent can start any single task by
reading four notes or fewer.** If it cannot, the chunking is wrong.
