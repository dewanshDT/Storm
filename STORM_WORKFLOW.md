# Storm Development Workflow

**Mandatory for every coding agent.** An implementation slice is not complete
until code verification and Storm state synchronization have both succeeded.

    IMPLEMENTED + TESTS PASS + STORM UPDATED = COMPLETE

Not:

    IMPLEMENTED + TESTS PASS = COMPLETE

This document defines the seven-phase development lifecycle. Agent-specific
instruction files (`CLAUDE.md`, `AGENTS.md`, `.cursor/rules/`) enforce this
workflow; do not maintain separate copies of these rules.

## Terminology

- **Storm** — the product/repository being developed.
- **Storm Vault** — the knowledge and project-state vault containing notes
  such as Active Work, Global Todo, implementation TODOs, ADRs, plans, and
  other project documentation.
- **Storm MCP** — the MCP interface used by coding agents to read and modify
  the Storm Vault. Always say "Storm MCP" when referring to this interface.
- **Storm Server** — the server component of the Storm product.
- **Storm Client** — the client applications of the Storm product.

## Phase 1 — READ

Before modifying code, inspect the current Storm state. The vault is part of
the project's persistent development state — it is not optional documentation.

1. Read this repository's instruction file (`CLAUDE.md` / `AGENTS.md`).
2. Read `PLAN.md` — the **Status** table, the **Programs** table and the
   **Decision log**. Decisions are settled; do not relitigate without a reason.
3. Read `Storm/Active Work` via Storm MCP — the short in-flight queue.
4. Read `Storm/Global Todo` via Storm MCP — the aggregate tracking surface.
5. Read the design pack for the work, **from the vault, not `docs/`** — see the
   pull table below.
6. Identify the currently approved implementation slice, its dependencies,
   blockers, and acceptance criteria.

### Which surface is authoritative for what

| Surface | Authoritative for |
|---|---|
| `PLAN.md` | **Decisions and milestone evidence** — why a choice was made |
| `Storm/Active Work` | **The in-flight queue** — what is open now, who is blocked |
| `Storm/Global Todo` | **The aggregate view** — milestones, program phases, backlog |

**A disagreement between them is a defect**, not a choice to make silently. It
means one was not updated in the same change as the work. Say so and fix it.

### Pull the note for the job, not the whole pack

`docs/storm-*.md` are per-milestone briefs and are **not maintained after their
milestone ships**. The living design lives in the vault:

| Working on | Pull |
|---|---|
| Auth, identity, pairing, sessions, MCP keys | `Storm/Remote Connectivity` → `Storm Authentication`, `Storm Auth Data Model`, `Storm Auth Protocol`, `Storm MCP Keys` |
| **The relay — wire protocol** | `Storm/Relay Protocol` |
| **The relay — server-side code** | `Storm/Relay Server Integration` — **read before touching `api.rs` for relay work**; its checklist table is the authoritative list of `storm-server` changes, and four of them fail in ways that look like something else |
| **The relay — trust or abuse limits** | `Storm/Relay Security` |
| **"Why is it this way" / "was that tried"** | `Storm/Relay Review Log` |
| Architectural decisions (R1–R13, A1–A14) | `Storm/Remote Decisions` |
| What is still genuinely undecided | `Storm/Remote Open Questions` |
| The agent runtime | `Storm/Agent Runtime` |

**Do not infer project state purely from Git history or the codebase** when
Storm documentation already contains it. Git/code is the *implementation*
state. Storm is the *project/work/knowledge* state. Both must be considered.

## Phase 2 — PLAN

Before implementation, identify:

- Current slice and intended scope
- Acceptance criteria
- Dependencies and blockers
- Expected files and components
- Tests and verification required
- Whether the task changes architecture or ADRs

**Do not silently expand into the next slice.** If implementation reveals a
new architectural question or scope change, stop and ask rather than silently
changing the plan.

**An `OPEN QUESTION` is not a gap for the implementer to fill.**
`Storm/Remote Open Questions` marks what is genuinely undecided. Resolving one
means **moving it into `Storm/Remote Decisions` with the reasoning** — in the
same change — not picking an option quietly in code. A lean recorded there is
not a decision.

**A question can also be wrong.** Q13 was retired rather than answered because
it contained a false premise that contradicted an existing ADR in its own
sentence. If a question cannot be answered without violating a decision, say
that instead of answering it.

## Phase 3 — IMPLEMENT

Implement only the approved slice. Maintain existing discipline:

- Small, reviewable slices
- Additive changes where required by the current phase
- Preserve existing tests
- Do not silently merge unrelated work
- Do not begin the next slice because the current one finishes
- Do not touch staging/VM/production unless explicitly authorized

Follow all repository-specific rules in `CLAUDE.md` / `AGENTS.md` and the
invariants in `Storm/Invariants.md`.

## Phase 4 — VERIFY

Before declaring a slice complete, run the appropriate verification. Depending
on the slice this may include:

| Layer | Command |
|---|---|
| Lint + types | `make check` (clippy `-D warnings` + dart analyze) |
| Unit tests | `cargo test` + `flutter test` |
| Integration | `make test-live` |
| Formatting | `make fmt` |
| MCP tools | `tests/mcp_e2e.py` |
| Client tests | `flutter test` |
| Mutation tests | per-module mutation harness |
| Staging/VM | only when explicitly authorized |

**Report what was actually run.** Never claim a test passed if it was not run.
If verification fails, fix the implementation before proceeding to Phase 5.

## Phase 5 — UPDATE STORM

**This phase is mandatory.** After implementation and verification, but
*before* declaring work complete, synchronize Storm.

Use Storm MCP wherever possible (`list_vaults` → `search` → `get_note` →
`update_note` with `base_version`). Always `get_note` before `update_note`.

Update the relevant surfaces:

| Surface | When to update |
|---|---|
| `PLAN.md` | Milestone or **program phase** state changed, new blocker, a finding that changes a decision, Status/Programs table out of date |
| `Storm/Active Work` | Current slice finished/changed, new blocker, next slice identified |
| `Storm/Global Todo` | Milestone, program phase, or queue item changed state |
| `Storm/Remote Decisions` | Implementation introduced, confirmed, **amended** or invalidated an ADR. Amend with why — never delete an entry |
| `Storm/Remote Open Questions` | A question was resolved (move it to Decisions), sharpened, or **retired as ill-posed** |
| `Storm/Auth Data Model` / `Storm/Auth Protocol` | Implementation establishes or changes a contract represented there |
| `Storm/Relay Protocol` / `Relay Server Integration` / `Relay Security` | A relay design detail changed, or a code reference in them drifted |
| `Storm/Relay Review Log` | A review found something — **including when the finding was that a previous fix was wrong** |
| `TODO — Storm Relay` | A checklist item landed, or a new one is implied by a design change |

**A milestone or phase changing state touches at least three surfaces**
(`PLAN.md`, Active Work, Global Todo). Updating one and not the others is the
drift this workflow exists to prevent — and it has happened: the A10 cutover
shipped on 2026-08-20 with the vault left describing the pre-cutover world,
which made every tracking note wrong for a day.

**Code references rot.** A note that cites `api.rs:1182` is making a claim
about the current tree. When you move that code, fix the note in the same
change or the next agent trusts a stale line number.

**The Storm vault must describe what actually exists, not what the agent
intended to build.** If Sessions is implemented and verified:

- WRONG: `Sessions — NEXT`
- CORRECT: `Sessions — DONE` · `Pairing — NEXT`

If implementation is incomplete:

- WRONG: `Sessions — DONE`
- CORRECT: `Sessions — IN PROGRESS`

Every vault note an agent edits carries a `summary:` in its frontmatter. Add
it when creating; refresh it whenever you update.

## Phase 6 — VERIFY STORM

After updating Storm, **re-read** the relevant documents. Verify:

- Completed work is marked correctly
- Active Work reflects the current queue
- Global Todo mirrors the state change
- Next work is accurately identified
- Blockers are current
- Summaries are not stale
- No contradictory status exists
- No duplicate TODO was created
- Relevant ADR state is correct

A successful MCP write does not mean the documentation state is correct.
Re-read to confirm.

## Phase 7 — REPORT

The final response for an implementation task must contain:

```
Status:      <complete | partial | blocked>
Slice:       <exact slice name>
Implemented: <what was built>
Verification: <what was actually run and the result>
Storm updated:
  TODO:        <yes / no>
  Active Work: <yes / no>
  Global Todo: <yes / no / not needed>
  ADRs:        <yes / no / not needed>
Next:         <next approved slice>
Blocked by:   <none or explicit blocker>
```

If Storm was not updated, explicitly say why and do not describe the
implementation as fully complete.

## Anti-pattern: implementation without state synchronization

This is the failure mode this workflow exists to prevent:

    READ → IMPLEMENT → VERIFY → STOP

The required sequence is:

    READ → PLAN → IMPLEMENT → VERIFY → UPDATE STORM → VERIFY STORM → REPORT

An agent that skips Phase 5 forces the next agent to inspect code and Git
history to infer what was built. That is exactly the failure this workflow
eliminates.

## Multi-agent continuity

This workflow explicitly supports handoff between agents:

    Agent A: READ → PLAN → IMPLEMENT → VERIFY → UPDATE STORM → REPORT → stops
    Agent B: READ (sees A's completed work in Storm) → PLAN → continues

An agent MUST NOT require the previous agent's chat history to understand the
current project state. Storm contains enough state for another agent to
continue safely.

## Scope gate

This workflow applies to **every coding agent** working in the Storm
repository, regardless of tool:

- Claude Code (reads `CLAUDE.md`)
- Cursor Agent (reads `AGENTS.md` + `.cursor/rules/`)
- OpenCode (reads repo-level `AGENTS.md`)
- Future terminal-based agents

No agent is exempt. No agent may skip a phase without explicit human
authorization.
