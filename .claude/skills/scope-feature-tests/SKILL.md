---
name: scope-feature-tests
description: >-
  Scope the testing for a feature in constructorfabric/insight — turn a GitHub issue (or a
  described feature) into a lean, code-grounded test scope: the axes/dimensions of checking,
  grouped test areas with a target suite each, an in/out-of-scope boundary, and an acceptance
  gate, then optionally file it as a linked test subtask. Use this whenever the user wants to plan HOW a feature will be
  tested — "scope the tests for #1602", "what do we need to check here", "how are we going to
  test this once it's done", "define the test dimensions / groups / axes", "make a test plan or
  test scope", "create a test subtask for this feature", or when reviewing a feature to decide
  QA coverage. The heart of the skill is grounding every check in the ACTUAL implementation and
  bronze→silver→gold data flow in the insight repo — this repo, or the sibling
  ../insight checkout when working from elsewhere — (not generic checklists) and
  correcting the feature's stated framing against what the code really does — including reviewing
  the issue's acceptance criteria first and proposing a corrected set when they are arbitrary,
  and mapping the touched components' malfunctions as a test axis. Its authoring counterpart is
  quality-vector-tests, which lays the resulting coverage into the feature body as tracked
  scenarios. This is the QA-scoping counterpart to file-bug-insight (which reports an observed defect): prefer this when
  the task is planning coverage for work being or about to be built, and keep the altitude at
  dimensions/groups, not individual test cases.
---

# Scope feature tests (Insight)

Turn a feature into a **test scope** a QA lead would actually trust: a short, code-grounded
document that says what dimensions to check, groups the tests, draws the in/out boundary, and
ends in a real acceptance gate. Then offer to file it as a linked test subtask.

The output is deliberately *not* a list of individual test cases. Cases come later and are
cheap; getting the **shape of coverage** right — the axes, the boundary, the gate — is the
expensive, high-leverage part, and it's what this skill produces.

## Why this skill exists (the one idea)

A test scope written from the issue text alone is a generic checklist and worthless. The value
comes from **grounding every check in the real implementation** and **correcting the feature's
stated framing against what the code actually does.** The issue is a hypothesis; the code and
the data flow are the truth. The single most valuable move you will make is catching where they
disagree — that's where real coverage decisions live, and it's what a naive checklist misses.

Concretely, in the exercise this skill is built from: the issue implied git was an identity
source and the debate was "github vs github-v2." Reading the dbt models showed **git emits no
identity signals at all** — the person is email-keyed by a *different* set of sources, and git
merely *resolves* onto it. That single correction reshaped the entire test scope. Chase that.

## Workflow

### 1. Read the feature — and hold its framing loosely
Pull the issue (`gh issue view <n> --repo constructorfabric/insight --json title,body,labels,assignees,projectItems`).
Extract: what it does, why it matters, who consumes it, and its **shape** — one of:
- *port* — behavior must match an existing impl. North star: parity, proven by a **differential**
  against the old impl.
- *refactor/consolidation* — rework or merge existing outputs into a new form. Almost always has a
  differential too: the new output must reproduce what it *retires* (old scattered rows, sibling
  keys it subsumes) on the same data. Hunt for what it supersedes; make that the headline gate.
- *new capability* — leans on functional correctness.
- *security / platform replacement* — a new security or infrastructure component replacing an old
  one that is *deleted outright* (no migration, no installs to preserve). There is **no
  differential** — nothing to reproduce. The gate is that the **security properties hold**
  (fail-closed everywhere, no secret leaks, revocation propagates) plus the spec's own acceptance
  criteria. Don't force a parity gate here; centre the scope on the unhappy path.
- *migration platform / rolling migration* — a new generic engine that will absorb a whole
  catalog of existing things **one wave at a time** (e.g. a unified metrics runtime that every
  domain migrates onto). Here the deliverable is not one differential but a **reusable per-wave
  gate**: the same harness re-runs for each wave (AI, then git, then crm…), and the first wave
  exists to *prove the gate*. Scope the machinery, not just the first wave.

The shape sets the whole strategy. Treat every claim in the body as a hypothesis to verify in
step 2.

**When the exact target set isn't final** (it'll be settled by a later scoping pass, or items will
merge/split), do not scope to a hand-count of items — scope the **machinery and invariants** that
hold regardless of the final list (registry-driven harnesses, sum/parity invariants). A scope
pinned to today's list rots the moment the list changes; one pinned to invariants survives it.

### 1b. Review the acceptance criteria before scoping against them
Run the AC review the `quality-vector-tests` skill's step 1 owns — the testable / complete /
real three-count check, the corrected AC-1..N list proposed to the user *before* anything is
designed against it, deferred ACs kept with an explicit reason and owner. Everything downstream
(the groups, the gate, the Testing section `quality-vector-tests` writes) maps to the agreed
set. Two scoping-side caveats on top of that contract: the **real** check usually needs step
2's grounding to answer — treat the review as provisional until grounding confirms it — and a
deferred AC can stay in scope as an executable `xfail` gate (see the boundary traps in step 5)
instead of dropping out.

### 2. Ground it in the real code and data flow
This is the core. Investigate this repo — `src/backend`, `src/ingestion` (and its dbt),
`src/frontend` for UI; when invoked from outside an insight checkout, the sibling `../insight`
clone is the fallback. Do not
scope from memory or the issue alone. Look at:
- The **implementation** being tested (the service / module / connector).
- The **data flow** end to end: Bronze → dbt Silver → Gold views → analytics → SPA. Trace
  where the thing under test actually *reads* vs *writes*. Which sources *seed* it vs merely
  *consume* it? What's the real join key at each hop?
- The **consumer contract** — who calls it and what shape they depend on (e.g.
  `src/frontend/src/api/`, `analytics` clients).
- **Design docs / specs** in the repo (`docs/`, `inbox/`, cypilot specs) for intended behavior.
- **The existing test coverage** — read the unit/integration/e2e tests already there. A good scope
  points at the *gaps* (what's unverified), not what's already green. Note which surfaces have zero
  tests — those are where the scope should spend first.
- **The component map and its malfunctions** — list the components the feature's path actually
  touches (SPA, gateway, authenticator, analytics, ClickHouse, dbt, a connector…), and for each
  ask what *down*, *slow*, *wrong data* and *stale* do to the feature's promise. Proportionate to
  the feature: a small feature gets a one-minute table-top pass, not a workshop, and only the
  failures that are real become checks. Each one carries the **expected degraded behaviour** as
  its oracle ("the error banner shows", never "doesn't crash") — and if the product defines no
  degraded behaviour, that is a design finding to raise, not a check to invent. This axis earns
  its place because error-handling paths have no requirement to derive from: AC-driven tests
  structurally miss them. Note the tooling split: *down/slow* need fault injection (kill or pause
  a container, a latency proxy); *wrong/stale data* is a fixture concern, not chaos tooling.

**In-progress features live on a branch, not `main`.** If `main` doesn't contain the code, find
the implementation: `gh pr list --search`, `git branch -r | grep`, and read the actual branch
(worktree it via `git worktree add`, or `git show <branch>:<path>`). Scope against
the real implementation + its `DESIGN.md`, not the issue's prose — the two often diverge, and a
branch's DESIGN doc is frequently the richest single source of the test surface.

Enumerate worktrees authoritatively (`git worktree list --porcelain`) — never glob directory
names. Spawn parallel `Explore` agents when the surface is broad. Reading the code, migrations,
and schema is normally enough to *scope* coverage; a live stand's ClickHouse is an optional
sharpener for a specific check, not a prerequisite — you're planning tests, not executing them.

**When the code contradicts the framing, say so out loud and let it reshape the scope.** This is
the skill's whole point, not a footnote. Grounding this deep regularly turns up an **actual defect**
(a race, an off-by-one, a broken invariant), not just a test dimension — don't drop it: fold it
into the relevant test group as a must-verify, flag it to the author, and offer to file it
separately (via `file-bug-insight`) if it's a merged-code defect rather than in-progress work.

### 3. Reframe to what a user/consumer observes
Shift from "does function X return Y" to **"does the right thing show up for whoever depends on
this"** — the right person on the dashboard, the right metric total, the right screen, the right
API shape. Name the consumers and the observable outcomes. Functional/behavioral framing surfaces
the checks that matter and hides the ones that don't.

### 4. Find the axes; build a matrix where there's variation
Coverage is measured along **axes** — the dimensions the behavior varies over (source × join-key,
tenant × role, connector × operation, input-shape × outcome). When two axes interact, a small
**matrix** with the expected outcome per cell is worth more than paragraphs — it makes the
coverage and the gaps legible at a glance. Identify the axis where the *risk actually lives*
(in the worked example, everything off the email column) and center the tests there.

The **concrete enumerated set is itself an axis** — not just abstract behavior. "Does sum/ratio ×
each view work" is the engine; "does *each real metric* produce the right value with its own
measure/dimensions/cohort" is the coverage that catches per-item config bugs. Name that axis and
list it (with rough counts — a count and a validation-effort estimate help the reader size the
work). When the set is large and will grow (100 metrics, N connectors), say so and specify a
**registry-driven / parametrized** harness that reads the source of truth — so coverage scales by
adding config, not test code. That design choice belongs in the scope, because authoring N
hand-written suites is the failure mode.

### 5. Draw the in/out boundary
Explicitly list what's **out of scope** — not built yet, owned by another layer, or deferred to
later work — so nobody scores the feature against unbuilt surface. For a **port** — and usually a
**refactor/consolidation** — add the parity framing: the new behavior must equal what it replaces,
verified by a **differential** (same input, outputs compared row by row, every expectation
tagged `exact` / `known-diff(direction)` / `merge` — see the differential-gate axis below; a
port is simply the case where the whole table is `exact`) as the headline gate.
For a port, "what it replaces" is the old impl; for a consolidation it's the retired rows / sibling
keys the new output subsumes. When that differential is the strongest available proof, give it its
own headline test group rather than burying it inside the correctness group.

Two boundary traps worth naming explicitly:
- **Don't gate on deliberately-deferred behavior.** The issue's generic ACs often imply behavior
  the code intentionally doesn't implement yet (e.g. "no cross-tenant leakage" when the platform
  deliberately runs single-tenant and leaves tenant filtering outside the current design scope). Gating on it fails
  by design and scores the feature against unbuilt work. But confirm the boundary with the user
  rather than silently excluding — they may want it **kept in scope as executable `xfail`**: the
  gate that turns green when the deferred piece lands, so a known gap stays visible instead of
  vanishing from the plan. (`xfail`-as-gate is the honest middle between "test it" and "drop it.")
- **Sequence unbuilt consumer validation, don't drop it.** If the UI / downstream consumer for
  this feature isn't built yet, the end-to-end validation is still *in scope* for the feature's
  definition of done — mark it **blocked on / sequenced behind** that build, not out of scope.
  "Not testable today" ≠ "not required."

### 6. Narrow to test groups — as an action plan
Collapse the dimensions into a handful of named **test groups** (aim for ~5–8), each a single line.
Add sub-bullets or a matrix only where they carry real signal. Resist enumerating individual
cases — if you're writing "test that email with trailing space unifies," you've dropped too low.

**Give each group a home: the target component, then the cheapest suite that can falsify it.**
The suite menu depends on the component the group targets — take it from the per-component
layer table in `quality-vector-tests` step 4 rather than from memory (the ladders differ by
backend function, and auth claims can terminate in `stand-ui`). The tag names the suite the
tests will land in, which is what makes a coverage gap checkable later. Suite tags and
criterion ids are exempt from the strip-internal-identifiers rule below — they are the
tracking scheme itself. Browser journeys pay a
permanent flake tax: a stand-ui group must say what is user-visible about the claim that a
cheaper suite cannot observe.

**Write each group as an action, not a property.** A QA lead reading it should know what to *do*.
Lead with a verb and name the setup → the check: "**Build** the metric×view harness: for each
seeded metric request every view, assert value/dimensions/cohort" beats "Coverage: every metric
serves in each view" — same content, but one is executable and one is a description. Order the
groups as a plan (reusable harnesses first, then the gate, then apply per case). Keep it
**scannable**: short sentences, no stacked parentheticals; if a reader has to re-read a line, cut
it down. This is a frequent failure mode — the first draft reads like a spec, not a plan.

**Deep analysis, lean artifact — they are not the same document.** The surface inventory, state-
machine list, coverage matrix, and code-symbol grounding you build to *find* the coverage are
working notes; they are not what you file. Do that analysis (it's how you get the coverage right),
then keep it in chat or a plan file. What you **file** is a short, plain-language plan a QA lead
skims in under a minute: the test groups ordered **biggest-risk-first**, each a plain sentence,
with the internal identifiers stripped out — table names, ADR numbers, symbol paths, the `S1/SM2/
T4` tags, the coverage matrix. Those are how *you* reasoned; to everyone else they read as a spec.
When someone asks for it "shorter" they mean **fewer concepts, not smaller font** — drop the
scaffolding, keep the risks. The end state to aim for is roughly six lines: one framing sentence,
then the handful of things that break the feature if they're wrong, worst first. That the analysis
was deep should show up as *good choices* in those six lines, not as six pages.

**Lead with the stand when the tests need a non-trivial environment.** If running these tests
means standing up new infrastructure (a new service, a fake provider, a compose-vs-kube choice,
seeded hooks), open the plan with a short **The stand** section naming exactly what to bring up
and how you drive it. A reader cannot act on a coverage spec that never says what to run it
against — that omission is what makes a scope read as abstract. Scopes over an already-implicit
stand (a running ClickHouse + dbt) can skip it; anything that stands up new infra cannot.

**Minimize without losing correctness by testing at the choke point.** When enforcement is
centralized — routes generated from one config, one shared verifier/middleware, one revoke
pipeline, one code path — test each property *once where it's enforced* plus a cheap invariant
that it's applied uniformly (a golden-file check that every generated route carries the auth
block), instead of re-testing per route / per service / per caller. The deletions are safe
precisely because the architecture forces those cases identical — say so, so the cut reads as
principled, not lazy. Keep the security core (fail-closed, revocation, no-token-leak) and the
cheap must-resolve spec ambiguities; those are correctness, not volume.

### 7. Write the lean scope doc
Use the template below. Short. Every line earns its place. There are two altitudes: the full
template when the reader wants the working plan, and a **compact form** — one framing line + ~6
risk-ordered bullets + an out-of-scope line + a rough day estimate — when they want just the main
effort. **Default toward the compact end** and expand only when asked; it is far easier to deepen
a scope on request than to get a reader to wade through an over-detailed one. A scope that is
correct but unread has failed.

### 8. Offer to file it
Offer to file the scope as a **linked test subtask** under the parent feature via the
`github-task` skill (Type `Task`, added to the Insights board, attached as a sub-issue). Draft
for review first; don't auto-create. When the coverage belongs in the feature body instead,
hand the groups to `quality-vector-tests` — it lays them out as the tracked scenario checkboxes. Do **not** reimplement issue mechanics here — compose with
`github-task`.

## Common axes worth probing (prompts, not a fixed checklist)

Use these to interrogate the feature — keep the ones the code makes real, drop the rest:
- **Source coverage** — per connector/source, does the thing participate correctly? Which sources
  *seed* vs *consume*? What join key does each use, and where does that key break?
- **Correctness of the core outcome** — the headline behavior, and its failure twins
  (under- vs over-doing it: split vs merge, missing vs duplicated).
- **Fail-safe** — unresolved/unknown/malformed input degrades safely, never corrupts good data.
- **Idempotency & lifecycle** — re-runs stay stable (no churn/dupes); create/update/delete and
  join/leave/rename behave; history retained.
- **Tenant isolation** — no cross-tenant leakage; same key across tenants stays separate.
- **Contract & consumer compatibility** — API/response shapes, status/error codes, downstream
  consumers and existing e2e stay green.
- **Migration & cutover** (esp. ports) — new impl boots on the old schema/data, no re-migration
  or loss; drop-in swap is invisible to consumers.
- **The differential gate** (ports, consolidations, migrations) — same seeded dataset → new output
  vs what it replaces. **It is rarely pure zero-diff.** Tag each item: `exact` (must match),
  `known-diff(direction)` (a deliberate semantic change — assert the direction, don't fail it), or
  `merge/scale-preserving` (siblings collapse into one — merged == Σ parts, source → breakdown).
  The tagged expectation table *is* the reusable artifact; a blanket "zero diff" gate is wrong and
  will fail intended changes.
- **Invariants a consolidation must hold** — e.g. a merged total equals the sum of its parts, and
  keeps holding after any roll-up (day → week/month). Absent-source honesty: a source not present
  stays NULL/absent, never a fake zero.
- **Reusable per-wave gate** (rolling migrations) — when a platform absorbs a catalog wave by wave,
  the differential + matrix must be **registry-driven** so each wave re-runs them by adding config,
  not code; the first wave proves the gate. Prove it in the scope: seed one out-of-first-wave item
  and confirm it's covered with no test edit.

## Output template

Keep it this tight. Drop any section that doesn't apply rather than padding it.

**Compact form (the default when filing).** Most of the time this is all you file — one framing
line, ~6 risk-ordered bullets, an out-of-scope line, a day estimate. Reach for the full template
below only when the reader wants the working plan.

**Always wrap the compact form in a collapsible spoiler** (`<details>`/`<summary>`) so it drops
into a GitHub issue or comment as one foldable block, with the 🧪 title line as the summary. Put
a blank line after `<summary>` and before `</details>` or GitHub won't render the markdown inside.
Estimate assuming the tests are **automated over seeded example data on the existing stand** —
that is the norm here, so day counts are small (often 1–3); only call out manual effort if some
check genuinely can't be automated.

```markdown
<details>
<summary>🧪 QA testing scope — <feature> (#<n>)</summary>

<one framing sentence: what we're proving + the strategy in a phrase — e.g. "new feature, no
baseline to compare, so we test correctness" or "port — must reproduce the old counter">.
<Optional: one clause naming the single fixture / dataset threaded through everything.>

1. **<Biggest risk, plain words>** — <suite> — <what breaks the feature if this is wrong>.
2. **<Next risk>** — <suite> — <one plain sentence>.
... (~6 bullets, worst first)

**Out:** <what's deferred / owned elsewhere>. **~<N> QA-days, automated.**

</details>
```

**Full template** — when the reader wants the working plan, not just the headline effort:

```markdown
Test scope for <feature> (parent #<n> — use the nearest parent; note both if the feature itself
has one). Goal: <one line. For a port/consolidation, name the parity bar and what it must
reproduce — e.g. "the new counter equals the retired sibling keys on the same data">.

**<Axis / source split>** — <the corrected, code-grounded framing; e.g. seed sources vs
resolve-only sources, or the key axis of variation>.

**Out of scope:** <not-built / other-layer / deferred surface>.

## The stand   <!-- only if the tests need new infra; drop for an implicit existing stand -->
<what to bring up — services, fake providers, compose vs kube, seed/hooks — and how you drive it>

## Plan

**1. <Verb> <the action>** (<suite> · the AC-<n> it proves) — <what to set up → what to assert;
matrix only where it adds signal>.
**2. <Verb> …** — <one line>.
... (~5–8 steps; reusable harnesses first, then the gate, then apply per case)

## Acceptance
- [ ] <the coverage/differential gate that actually proves it — e.g. per-source coverage measured;
      differential tagged exact/known-diff(direction)/merge, never blanket zero-diff>
- [ ] every criterion of the reviewed AC set maps to a test group (deferred ones carry a reason
      and an owner)
- [ ] <invariant / registry-driven / sequenced-UI checks as they apply>
```

## Worked examples

Five, one per shape — read the one closest to the feature in front of you:

- `references/example-identity-resolution.md` — a **port** (C#→Rust identity-resolution). Models
  the altitude, the assumption-correction move (git seeds no identity), and the source × join-key
  matrix, with the old-vs-new differential as the gate.
- `references/example-collab-consolidation.md` — a **refactor/consolidation** (collapse
  connector-scoped metric rows into modality counters with a breakdown). Models how a non-port
  still gets a differential — against the *retired sibling keys* it subsumes — plus the
  sum-equals-parts invariant and honest-NULL (not fake-zero) correction.
- `references/example-unified-metrics-migration.md` — a **migration platform / rolling migration**
  (a generic metrics engine that the whole catalog migrates onto, wave by wave). Models the
  provisional-target-set principle (test machinery over a fixed list), the three-way differential
  tag, registry-driven harnesses, `xfail`-as-gate for deferred tenant isolation, the action-plan
  altitude, and grounding that surfaced a real reconciler race.
- `references/example-nginx-auth-security-epic.md` — a **security architecture EPIC** (replace an
  API gateway with an nginx edge + authenticator gear), specs-first (no code yet). Models the
  no-differential security gate, grounding in PRD/DESIGN found on a merged branch, the fail-closed
  matrix as the headline, choke-point **minimization** (test once + a golden-file invariant), the
  **stand-first** action layout, and treating a fake provider's control hooks as the test oracle.
- `references/example-functional-role-cohorts.md` — a **new capability** (per-person role sets from
  HR), no baseline to match. Models the risk-ordered correctness gate, "set-ness" as the one
  concentrated risk, honest-gap / expected-fail handling — and the **deep-analysis vs lean-artifact
  split**: do the surface/state-machine/matrix work to find the coverage, then file only the ~6-line
  risk-ordered plan. The reference case for "shorter means fewer concepts, not smaller font."
