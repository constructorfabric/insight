---
name: stand-scenarios
description: "Turn docs/product/SCENARIOS.md into testable claims for the deployed-stand suite — QA scenario design for Insight. Pick a scenario (S-1…S-10), a persona (EXEC/LEAD/IC/ADMIN), an invariant (§5 rules 1-12) or an Appendix A question (A1…G4), ground it in the seeded roster, and emit claims with an oracle, a layer and a priority that stand-api-test / stand-ui-test can implement. Use when the task is deciding WHAT to test: 'what should we test for S-9', 'design e2e scenarios from the product doc', 'which product rules are unverified', 'is the IC boundary covered', 'plan coverage for the reach matrix', 'convert the scenarios doc into tests'. Not for writing test code — that is stand-api-test and stand-ui-test; for scoping tests from a GitHub issue or an unbuilt feature use scope-feature-tests instead."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Scenario design from SCENARIOS.md

[`docs/product/SCENARIOS.md`](../../../docs/product/SCENARIOS.md) is unusually
testable for a product document, and that is the whole opportunity here. It is
not prose about features — it is **boundaries**, written as a decision table
(§1.1 Reach: 4 personas × 7 dimensions, with explicit *Never* cells) plus
**twelve numbered invariants** (§5) that hold in every scenario. Those are
assertions someone already wrote down.

Your job is to convert them into claims a test can prove on the stand, and — at
least as importantly — to say plainly which ones **cannot be proved there
today** rather than designing a test that would pass for the wrong reason.

## Pattern files

- [persona-reach.md](./persona-reach.md) — SCENARIOS.md personas → seeded
  fixtures, the reach matrix as a decision table, and the traps in the mapping
- [invariants.md](./invariants.md) — §5's twelve rules as an assertion
  checklist, each with where it is enforced and whether the stand can see it

## The five questions, in order

Answer them in this order for every claim. Question 2 kills more candidate
tests than the other four together, and that is the point.

### 1. Which persona, and which seeded person is it?

SCENARIOS.md speaks in EXEC / LEAD / IC / ADMIN. The stand speaks in fixture
names. The mapping is in [persona-reach.md](./persona-reach.md) and it is not
one-to-one — **EXEC is not ADMIN**, and the seed enforces that distinction
deliberately.

Never write a claim against an email or a UUID. Fixture names are the contract.

### 2. Does the surface exist?

A scenario the product has not built yet produces **no claims**, and saying so
is the deliverable. Check the route tables
(`src/backend/services/{analytics,identity-resolution}/src/api/`), the
catalogue in `tests/stand/api/operations.py`, and the SPA
(`src/frontend/src/`) before designing anything.

Four verdicts, and only the first two produce claims:

- **Built** — a route or a screen answers this today.
- **Partial** — the mechanism exists but part of the guarantee does not. Design
  the claim for the part that does *and* record where the rest lives.
- **No surface** — nothing to test. Write one line saying so and stop.
  SCENARIOS.md Appendix C already concedes this for S-6.
- **Not enforceable here** — a real rule, wrong suite. Name the suite that owns
  it (the ingestion path, the chart install) rather than filing it as absent.

Beware the near-miss that looks like *No surface*: "there is no cost
**endpoint**" is true and irrelevant, because `ai.cost` is an ordinary metric on
the ordinary routes. Ask whether the *figure* is reachable, not whether a
dedicated route exists.

### 3. What is the oracle?

What do you compare against? On this stand there are exactly four legitimate
oracles, and one forbidden one.

| Oracle | Use for | Example |
|---|---|---|
| **The manifest** | identity facts — who exists, who reports to whom, which team | `stand_manifest.fixture("development_ic").uuid` |
| **The API response the UI just received** | anything on screen | the drilldown table matches the payload behind it |
| **The endpoint contract** | status codes, media types, problem documents | 415 on a wrong content type |
| **The code** | a rule whose only statement is an implementation | a suppression threshold that appears in no contract |
| ~~A number you read off a running stand~~ | **never** | `golden_metrics` is empty by design |

**No claim may assert an exact metric value.** Reading a number off the stand
and asserting it back only proves that the code which produced it produced it.
Assert *shape, reach, presence, honesty and refusal* instead — which is what
the boundaries in SCENARIOS.md are made of anyway.

### 4. Which layer?

**API unless the claim can only be true in a browser.** A browser is slow,
flakier than an HTTP call, and exercises more surface than most assertions
need. The suite would rather have one more API test than one more browser test
whenever the two would prove the same thing.

A UI claim must carry, in writing, why an API test cannot make it — backed by
something measured, not by "the UI should be covered too". The shipped
justifications are the model: *every SPA route answers 200 text/html to an
anonymous HTTP client; refusal exists only inside the browser.*

### 5. What already tests this?

Ask before designing, every time. `tests/stand/` carries well over a hundred tests and
several already prove SCENARIOS.md clauses without naming them — e.g. ADMIN
"never gains data visibility implicitly from administrative rights" is
`test_operator_sees_nobody_in_the_org_chart`.

```bash
grep -rn --include='*.py' "def test_" tests/stand
grep -rn "subchart\|visible-persons\|metric-results" tests/stand/api
```

§2 of `SCENARIO-COVERAGE.md` already attributes nine clauses to their tests —
faster than grep for anything recorded there.

Two blocking gates in the sibling rig also own territory — the metric×view gate
(`src/ingestion/tests/e2e/lib/metric_coverage.py`) and the per-operation API
gate (`lib/api_coverage.py`). Do not re-specify what a gate already fails on.

Mark the verdict on every claim: **new** · **covered by `<test>`** ·
**partially covered by `<test>`** (say what is missing).

## Where claims come from

Four sources, in descending value.

**1. The *Never* cells (§1.1) and the per-persona "never" bullets.** The
highest-value claims in the document. A reach bug is *silent* — nobody files a
ticket saying they can see too much — so these are the ones no other mechanism
will catch. Every "never" is a negative test.

**2. The §5 invariants.** Cross-cutting, so one claim can cover several
scenarios. See [invariants.md](./invariants.md).

**3. The "Not this" blocks.** Absence claims: what must *not* be promised. Often
testable as a shape assertion (a field that must not exist, a figure that must
not be summed).

**4. The positive persona bullets.** What each persona does. Necessary, but the
weakest — a passing positive test is compatible with a wide-open boundary.

## Claim form

One bold-led paragraph. The claim as a single indicative sentence, at most two
more sentences for the trap, then the metadata line. Facts the whole group
shares sit once above the group.

```markdown
## <group title — plain language>

**Source** SCENARIOS.md §S-9 LEAD. **Personas** `dev_lead`, `sales_lead`.

**S9-L-03 — a lead asking for their own manager's subchart is refused.**
The refusal must be 404 rather than 403: a 403 confirms the person exists, so
the boundary would leak the shape of the org while denying the data.
Oracle the manifest's org edges · API `GET /v1/subchart/{id}` · P0 · new
```

- **IDs** — `S<n>-<persona letter>-<seq>` (`S9-L-03`, `S1-I-02`, `S8-A-01`),
  letters `E`/`L`/`I`/`A`. Cross-cutting invariants take `R<rule>-<seq>`
  (`R10-01`). An ID never changes and is never reused, so **read the issued
  ones before minting a new one** — the register is §3 of
  [`SCENARIO-COVERAGE.md`](./SCENARIO-COVERAGE.md),
  and it is the only place they are recorded.
- **Confidence** — close the metadata line with `[VERIFIED]` (confirmed against
  the code *and* an existing test or `PROFILE.md`), `[SUPPORTED]` (one
  authoritative source), or `[INFERRED]` (deduced; say what the implementer
  must check first). Reach for `[INFERRED]` when you have read the endpoint but
  not the code behind it — that is where wrong claims come from.
  The tag rates the **claim's grounding, not the product's test coverage** —
  whether a test exists is the `new` / `covered by` field beside it. A gap can
  be `[VERIFIED] · new`, and usually is: that pairing is the whole point.
- **Priority is blast radius**, not screen prominence. Assign on the first
  trigger that fires:
  - **P0** — a caller is served data they are not entitled to, or another
    person's data · nobody can sign in · a headline figure is wrong for every
    viewer.
  - **P1** — a whole view, section or endpoint contract is broken for a class
    of user · missing data renders as a real-looking value.
  - **P2** — one tile, one control, one ordering detail, with a workaround.

  A rarely-hit authorization hole is P0; a broken button on the landing page is
  P2.
- **Cite the code** where the claim rests on an implementation detail. If you
  cannot cite the line, you do not know the behaviour — go read it.

Two markers ride inside the bold when they apply:

- `EXPECTED TO FAIL` — the claim is correct and the product does not honour it.
  Write it red and file the defect (`file-bug-insight`). **Never soften a claim
  to match the implementation.** In the suite this lands as
  `@pytest.mark.xfail(strict=True, reason=…)`, so the marker retires itself the
  day the product is fixed — see `stand-api-test`.
- `NO SURFACE` — the guarantee has nothing to test against yet. Carries no
  priority and no layer.

## Design discipline

**Assert the whole partition, not one membership.** A route that returned
everything it was given, or nothing, satisfies any one-sided check — and both
are plausible failures for a filter. The shipped `visible-persons` test names a
visible person, an out-of-scope person, an out-of-tenant person and an
unresolvable id in one request, for exactly this reason.

**Which code a refusal answers *is* the claim — and the two services differ.**
Get this from the code, never from instinct:

| Surface | Code | Reasoning it encodes |
|---|---|---|
| identity person-keyed routes (`/v1/subchart/{id}`, `/v1/profiles`) | **404** | a 403 would confirm the person exists, leaking the org's shape while denying the data |
| the analytics visible-set gate (`/v1/metric-results`, `/v1/metric-drilldown`) | **403** | the caller may not ask about this person, and saying so leaks nothing they could not already infer |
| identity admin routes, missing grant | **403** | a 404 would leak that the gate ran *after* the lookup |

Writing a 404 claim against `/v1/metric-results` is the most likely mistake in
this whole skill: it either fails, or gets mismarked `EXPECTED TO FAIL` and
filed against a deliberate contract.

**Would this pass on an empty stand?** If a view that rendered nothing would
still satisfy the assertion, the claim asserts nothing. Prefer "every report
the roster declares" over "somebody rendered".

**Distinguish the two failures a boundary can have.** Out-of-tenant and
out-of-scope leak different things — that an id exists somewhere in the
product, versus who reports to whom. Name both rather than treating "not
visible" as one bucket.

**Derive expectations at runtime.** Everything expected comes from the manifest
or from the response, never typed from a prior run's observed output. A
reshuffled seed should move the expectation with it.

**Synthetic only.** Per `AGENTS.md`, nothing production-derived reaches a claim,
a test or a commit. The seeded roster is synthetic by construction
(`@company.nonpresent`) — use it, and never a real name.

## Procedure

1. **Scope it.** One scenario, one persona, one invariant, or one Appendix A
   question. Do not sweep all ten at once.
2. **Read the source.** The scenario block *and* the §1.1 row for each persona
   it names, *and* the §5 rules it leans on.
3. **Ground the personas** — [persona-reach.md](./persona-reach.md).
4. **Check the surface exists** (question 2). Stop where it does not.
5. **Inventory existing coverage** (question 5).
6. **Draft claims**, negatives first.
7. **Assign oracle, layer, priority**; cite code where it rests on one.
8. **Self-check** against Design discipline above.
9. **Record it.** Claims land in
   [`SCENARIO-COVERAGE.md`](./SCENARIO-COVERAGE.md)
   — surface verdicts in §1, existing coverage in §2, claims in §3, and
   product-doc-versus-product divergences in §4 (**Findings**, which is where
   the most valuable output usually goes: a rule that is enforced in the wrong
   place, or two enforcement points disagreeing on a number).
10. **Hand off** — `stand-api-test` or `stand-ui-test` per claim;
    `file-bug-insight` for anything marked `EXPECTED TO FAIL`.

Its §2 also answers question 5 faster than grep does, for anything already
attributed.

For a full pass over several scenarios at once, dispatch the
`stand-scenario-designer` agent — it does the reading in its own context and
returns claims. Run `stand-test-auditor` after the tests exist, to check each
assertion actually proves the claim it names.
