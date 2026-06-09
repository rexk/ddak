# CONTEXT_HANDOFF — living context root

Product pivot from ddak v0.1. This document is the canonical handoff context
for all design threads. It renders for humans and is digestible by LLM agents
(R3). PROJECT_SPEC.md (v0.1) is a salvage source, not a baseline.

Status legend:
- **ESTABLISHED** — decided in dialogue with the owner; binding.
- **PROPOSED** — produced by a design thread; pending owner ratification.
- **OPEN** — fork surfaced, deliberately not closed.

---

## 1. Product statement (ESTABLISHED)

Scaffolding for orchestration-layer work — what Git and the pull request were
to the individual contributor, this is to the agent-manager. It sits across
heterogeneous AI coding agents (Claude Code local + web, Cursor web, Codex
web, others) and gives the human one place to dispatch, re-orient, share, and
condense work executed by many agents in many locations. It does not own the
agents' execution; it owns the connective tissue between sessions, branches,
tickets, decisions, and knowledge.

Core pain: after any context switch, re-orientation requires crawling agent
transcripts across many tabs. The pain compounds with parallelism.

## 2. Ground rules (ESTABLISHED)

- R1. On any competing idea, make NO assumptions. Surface the fork, define
  terms, decide explicitly, log it.
- R2. Be imaginative; do not prematurely cap ideas at human-practical limits.
- R3. Every artifact must render for humans AND be digestible by an LLM agent.

Owner's working mode: concrete workflow over abstractions; conventional terms
("no wordplay"); prose dialogue until shape is stable; surface forks plainly.

## 3. Direction (ESTABLISHED, from D1–D11 + dialogue)

- Pivot replaces the v0.1 direction. Salvage: deterministic projection
  rebuild from an append-only event log.
- Not issue-canonical. External trackers are attachable references, never
  core schema.
- Native frame: design from LLM-driven workflow pains, not from existing tool
  categories. Those become integrations.
- Protocol is bidirectional but BOOKENDED by the agent-session lifecycle
  (D4'): `context.pull` at session start; `context.push` at checkpoints and
  end. Not a chatty live broker.

### Pending owner's nod
- **D12 (vision principle, PROPOSED):** the product makes orchestration-layer
  work tangible, reviewable, and accomplishable. Agents get the commits;
  today the manager gets scattered threads. This is the missing scaffolding
  for the manager's work.
- **D13 (differentiator, PROPOSED):** tracker↔knowledge interop exists
  (Jira↔Confluence). The novel thing is AI-native management: every context
  record is associated with the AI session(s) that worked it, and management
  operations (condense, digest, query, re-orient, review) are AI-assisted.

## 4. Primitives (ESTABLISHED — conventional terms, four total)

- **Task** — unit of intent. Has status, accumulates context, decomposes into
  sub-tasks.
- **Session** — one execution attempt by one agent in one place (CC-local,
  CC-web, Cursor-web, …). Has runtime state and a resume handle.
- **Document** — doc with its own lifecycle. Kinds: spec, design, report,
  knowledge-entry, review-record. Same shape, different lifecycle per kind.
- **External reference** — Linear ticket, GitHub PR, CI run, branch URL.
  Optional anchor for a Task.

### Linking patterns
- Task ↔ Session — one task, many sessions; strict parent + weak refs to
  other tasks a session incidentally touched.
- Task ↔ Document — input / output.
- Task ↔ External ref — anchors.
- Task ↔ Task — parent/child for decomposition; `supersedes` / `related-to`
  for cross-cutting. Typed relations are the recorded form of human judgment,
  not schema plumbing.

## 5. Surfaces (ESTABLISHED)

Conceptually one substance on a temperature gradient — knowledge is cooled
workflow re-entering as future input.

- **Workflow** (hot, solo) — live ops: dispatch, re-orient, resume.
- **Review** (warm, collaborative) — team's view of in-stream work needing
  engagement: spec/design review, mid-stream sanity checks, fork decisions,
  knowledge promotions. Where management work becomes legible to teammates.
- **Knowledge** (cool, team-durable) — settled memory: search, reference,
  subscribe.

Bridging: condense → promote (workflow → knowledge, routed through review by
default); reference-on-dispatch (knowledge → workflow at session start). Two
distinct share acts: share-progress (point-in-time digest URL) vs
share-knowledge (durable, subscribable entry).

## 6. Killer flow — re-orientation (ESTABLISHED)

1. Open product. See active / waiting / done.
2. Pick a piece of work.
3. Synthesized digest: intent · decisions · findings · per-session state ·
   open edge ("next: …"). Built from structured pushes, not transcript
   scraping.
4. One action → back inside the right agent session at the right location.

## 7. Protocol verbs (ESTABLISHED, lifecycle-bookended)

```
context.pull          — at session start: relevant workflow + knowledge +
                        prior reviews
context.push          — at checkpoints / end: decisions, findings, progress,
                        questions
session.register      — runtime identity + branch/ticket binding
session.resume_handle — emit re-entry info
review.propose        — author marks item for review, names reviewers, sets
                        the ask
review.act            — reviewer comments / approves / requests changes
review.conclude       — outcome captured; item continues or gates
work.query            — NL/structured query → a view
view.save             — pin a query as a persistent custom dashboard
handoff.render        — produce the digest/artifact
```

---

## 8. Thread output — delegate-and-detach, context.pull, review surface, promotion gate (PROPOSED)

Produced 2026-06-09 by the thread covering all four focus areas. Everything
in this section is pending owner ratification.

### 8.1 Convergence principles

- **C1 — One digest, three consumers.** The `context.pull` core payload, the
  `handoff.render` artifact, and the re-orientation screen are the same
  digest, rendered once for next-you, a teammate, and the next agent. R3 is
  satisfied by construction.
- **C2 — Pushes, not sessions, are the source of truth.** Resume-after-death
  and re-orientation are the same mechanism: a new session seeded by
  `context.pull` over the predecessor's pushes. A live resume handle is an
  optimization. Losing a session loses keystrokes, never context.
- **C3 — Adoption rides on session-start hooks.** Dispatch can begin inside
  the agent: a hook performs `context.pull` + `session.register`
  automatically. Product adoption does not depend on human discipline.
- **C4 — Review is a decision-hardening pass.** `review.conclude` writes the
  outcome back to the task as a decision; that decision becomes binding
  context served by future `context.pull`s. Review is not a separate world.
- **C5 — Knowledge carries provenance.** Every knowledge entry links to the
  session(s) and task that established it (D13 made concrete).

### 8.2 Delegate-and-detach flow

1. **Frame** — task exists or is created at dispatch; see fork F-A for
   session-first dispatch.
2. **Choose executor** — adoption is the contract (any agent speaking
   pull/push is in); launch is per-adapter convenience. See fork F-B.
3. **Context assembly** — `context.pull` (see 8.3).
4. **Detach** — human holds a dispatch receipt: task link, session entry,
   declared checkpoint cadence. The product is staleness-aware, not
   liveness-aware: no polling; the digest reports "last heard Xm ago, last
   state: …" and marks sessions `silent` past their declared cadence.
5. **Re-attach triggers** — (a) agent pushes a question → "waiting on you"
   in workflow (hot/solo — not review); (b) human re-orients by choice;
   (c) external-ref event (CI fail, PR review) attaches to the task.
6. **Resume** — `session.resume_handle` (URL for web sessions; directory +
   session id for CC-local). Dead handle ⇒ new session on same task seeded
   by pull (C2).

Pressure-tested cases:
- **Bake-off**: one task, N parallel sessions; digest shows per-session
  state side by side; the human's pick is recorded as a decision on the
  task; losing sessions marked `abandoned` with reason.
- **Scope drift**: agent declares "also touched X" in a push; AI matches to
  an existing task (weak ref) or proposes a new one.

### 8.3 `context.pull` response shape

Layered hot → cool:

1. Task header — intent, status, anchors with live external-ref snapshot.
2. Decision log — this task + inherited from parent, newest first, with whys.
3. Findings — established facts from prior sessions.
4. Open edge — "next: …" from last push + open questions.
5. Session ledger — prior attempts and outcomes.
6. Review context — concluded outcomes (binding) + open reviews that gate.
7. Knowledge table of contents — suggested entries (title, one-liner, id),
   bodies fetched on demand (fork F-C).

Pull is point-in-time (bookended). Agents may re-pull at checkpoints;
nothing is pushed to them mid-flight.

### 8.4 Review surface

A review record holds: **the ask** (one line), **the subject** (document /
digest / fork statement / knowledge candidate, version-pinned per fork F-E),
**AI-assembled context** ("what you need to know" with drill-down links into
workflow), **reviewers + state**, **response controls** matching `review.act`.

Lifecycle: `proposed → open → (comment / request-changes cycles) →
concluded(outcome) → archived-but-linked`. Conclusion writes back to the
task as a decision (C4).

Review kinds: spec/design review · mid-stream sanity check (default
non-blocking) · fork decision (R1 at team scale — the ask is "pick one") ·
knowledge promotion (the promotion gate's review path).

AI assistance: assembles the context packet; drafts a review brief; can
pre-review (fork F-F); condenses the concluded thread into the outcome
record.

### 8.5 Promotion gate

- Trigger: AI proposes at **boundaries** — task close, explicit condense,
  review conclusion — never continuous scanning (fork F-G). Explicit promote
  is always available.
- Mechanics: AI drafts candidate entry from the push stream (condense) →
  human edits → review (blocking by default for knowledge) → on approval,
  knowledge entry with provenance links (C5).
- Conflict handling: at promotion review, AI checks candidate against
  existing entries and flags contradictions ("contradicts K-12 —
  supersede?"), reusing the `supersedes` relation.

### 8.6 Forks surfaced this thread

| ID | Fork | Options | Recommendation | Status |
|----|------|---------|----------------|--------|
| F-A | Dispatch entry | task-first / session-first / both | Session-first allowed: bare `session.register` creates a provisional task, AI-titled from first push, owner confirms or merges later | PROPOSED |
| F-B | Launch vs adopt sessions | product launches / product adopts / both | Adoption is the contract; launch is per-adapter sugar where APIs exist | PROPOSED |
| F-C | Knowledge relevance at pull | deterministic-only / deterministic + AI ToC / fully agentic | Deterministic core + suggested knowledge ToC, bodies on demand; agentic query available as follow-up verb | PROPOSED |
| F-D | Pull during blocking review | refuse / warn / proceed | Warn-and-proceed default; hard-stop only where gate explicitly set blocking | PROPOSED |
| F-E | Review subject mutability | live subject / version-pinned | Pin a version per review round; show diffs across revisions in the record | PROPOSED |
| F-F | AI pre-review | always / on-request | Always for knowledge-promotion candidates; on-request elsewhere | PROPOSED |
| F-G | Promotion trigger | AI-proposes / always / explicit-only | AI proposes at boundaries (task close, condense, review conclude) + explicit always available | PROPOSED |
| F-H | Knowledge decay | TTL / supersede-only | Supersede-only, no TTL | OPEN (parked) |

---

## 9. Open areas (OPEN — close only with explicit decision)

- Promotion default — F-G recommendation above, not yet ratified.
- Team scope — start single-team (recommended); shape data for multi-team.
- Reviewer assignment — AI suggests, owner confirms (recommended) vs always
  explicit.
- Review gating — non-blocking default for checkpoints; blocking for spec /
  knowledge promotions (recommended).
- Home screen — deliberately deferred; design flows first, let home emerge.
- Descriptive vs prescriptive stance — record how you orchestrate vs assert
  a method. Flagged, not decided.
- Original P1–P3 and OQ1–OQ6 from the pre-pivot handoff remain live unless
  explicitly resolved.
