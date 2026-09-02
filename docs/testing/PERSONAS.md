# Constructor Insight — Personas & Scenarios (quality baseline)

**Date:** 2026-09-02 · **Status:** draft, for QA review
**Product source of truth:** [`SCENARIOS.md`](../product/SCENARIOS.md) —
five personas over two visibility modes, ten scenarios S-1…S-10 with per-persona boundary tables,
and twelve rules that hold everywhere. The individual user questions (A1…G4) stay in the
workspace's `quality/user-scenarios.md`. This document does not restate either; where it adds a
requirement of its own, the line is marked **(QA addition)**.

**What this document holds** — the QA half the product doc deliberately omits: who we log in as on
a stand and what that costs when it fails (§1), the mapping from test fixtures and markers to
personas and scenarios (§2), the order in which the scenarios are verified and what a check asserts
beyond the product tables (§3), the assertable invariants (Appendix A), and what is blocked and by
what (Appendix B).

**Vocabulary.** A *stand* is a test environment running a full copy of the product. A *typical
value* means one group-level figure — the median — never a list of colleagues. The two *visibility
modes* (`org_chart`, `flat`) are defined in the product doc's Personas section.

> **Where this stands today.** The 17 invariants in Appendix A are the first thing worth automating,
> and most can start now: they are assertions against API responses and run on any stand that has
> data. The exceptions are governance (A11–A13), which needs every persona present at once on one
> stand with realistic reporting lines, and anything PEER, which needs a stand running flat — **and
> no test stand satisfies either today.** Those fixtures, not the test design, are the blocker on
> P0 coverage.

---

## 1. Personas — the QA view

Reach, questions and boundaries per persona live in the product doc. What QA adds is the login and
the failure cost.

The **mode is a property of the stand**, set once per installation. At test time it is read from
`GET /v1/me` (`visibility_policy`), once per session — never assumed, never taken from a manifest.
Observed 2026-09-01: dev and qaclust run `org_chart`; cf-prod runs `flat`. No stand we control for
testing runs flat.

| Persona | Who we log in as, on a stand | What a failure costs them |
|---|---|---|
| **EXEC** | Someone whose subtree is the whole organization — the seed's `ceo`. That account also holds the admin role, so there is no EXEC-without-ADMIN login today (Appendix B) | A board-level number that is wrong or unexplained |
| **LEAD** | A manager with at least eight people under them — a seed `*_lead`; comparative conclusions need eight per side | Acting on a jam that is a data artefact |
| **IC** | Someone with nobody reporting to them — a seed `*_ic`; the tightest case in `org_chart` | Seeing colleagues' activity, or their own numbers misattributed |
| **PEER** | Any signed-in person on a stand running flat — no such stand and no such fixture exists today (Appendix B) | A rank shown that the mode does not promise; a comparison computed below the five-person floor |
| **ADMIN** | The seed's `admin_operator` — holds the admin role plus access to the stand itself | Shipping a customer that silently proves nothing |
| **none** | An account from another tenant, or one no person resolves to | Anything at all: the only correct responses are a refusal or an empty scope |

Three consequences for the setup:

- **A run covers one mode.** The personas a stand can host follow from its mode: `org_chart` gives
  EXEC, LEAD, IC; `flat` gives PEER; ADMIN and `none` exist under both. A pass as EXEC says nothing
  about IC — and nothing at all about PEER until a flat stand exists.
- **IC is the tightest case in `org_chart`; PEER inverts it.** IC is where the product must refuse
  the most, so it is the first place to check refusals. PEER has the broadest reach with one floor
  under it — the five-person minimum on a comparison — so it is the first place to check that the
  one remaining limit holds.
- **Logging in as every persona, plus `none`, is a prerequisite, not a detail.** The governance
  invariants (A11–A13) cannot be checked otherwise.

---

## 2. Harness mapping **(QA addition — deliberately absent from the product doc)**

None of this is built as of 2026-09-02; it is the agreed target.

**Persona comes from the session fixture** — who signs in — never from `requires_seed`, which names
data, not a viewer:

| Session fixture | Persona | Notes |
|---|---|---|
| `realm_admin_session` | EXEC | logs in as the seed `ceo`, who also holds admin |
| `lead_session` | LEAD | |
| `member_session` | IC | to be renamed `ic_session` — "member" is retired vocabulary |
| `admin_operator_session` | ADMIN | |
| `other_tenant_session` | none | |
| — | PEER | no fixture exists; blocked on a flat stand |

Seed roles (the seed profile's eleven role × team fixtures) map the same way: `ceo` → EXEC,
`*_lead` → LEAD, `*_ic` → IC, `admin_operator` → ADMIN. The realm roles behind them are
`insight-admin`, `insight-lead`, `insight-member`. Tests that build a session by name
(`session_for(...)`) declare persona explicitly with a marker.

**Markers** — on the stand-facing suites only (`tests/stand` api + ui); unit and contract suites
never carry them. Whether the e2e rig is in scope is an open question (§5).

- `persona(*codes)` · `mode(policy)` · `scenario(id)` — declared in both strict-marker roots
  (`tests/pyproject.toml`, `src/ingestion/tests/e2e/pytest.ini`) with a meta test that the two
  declarations match.
- Selection: `--persona` (repeatable), `--mode`, `--scenario`, deselecting in
  `pytest_collection_modifyitems`. An empty selection fails the run; exit-5 is not tolerated.
- Collection gates, beside the existing vector gate in `tests/stand/conftest.py`: exactly one
  persona per test and it agrees with the session fixture; at most one mode, with `peer ⇒ flat` and
  `ic/lead/exec ⇒ org_chart`; exactly one scenario.
- `mode("flat")` on an `org_chart` stand skips with a stated reason — the `requires_ingestion`
  pattern. Today `test_me.py` fails the whole run on a non-`org_chart` stand; the marker turns that
  failure into a skip.

The purpose is **selection** — run a persona's, a mode's or a scenario's tests against a stand —
not a coverage matrix. Priority (§3) says what runs first; markers say what runs at all.

---

## 3. Scenario priorities

The scenario definitions — one sentence, the per-persona table, *Not this*, *Today* — live in the
product doc. This section holds only the verification order and what a check asserts beyond them.

**The rule that produced the order:** a service scenario is verified before a main one in two cases
— when the main tier cannot produce a correct answer without it, or when its failure cannot be
undone afterwards. Identity (S-8) is the first case: a wrong person makes every number wrong.
A governance breach (S-9) is the second: no patch un-leaks a name.

| Priority | Scenario | Tier | Why here |
|---|---|---|---|
| **P0** | S-1 Metrics review | Main | Everything the product says stands on it, for every persona |
| **P0** | S-9 Configuration and access | Service | Irreversible failure: one leak of a named individual is not fixable by a patch |
| **P0** | S-8 Identity and organization model | Service | A wrong person makes every metric wrong |
| **P1** | S-2 Analysis and diagnosis | Main | The differentiator vs a dashboard; the first vertical |
| **P1** | S-3 Conclusions: recommendation and validation | Main | Its pre-declaration requirement expires at issue time |
| **P1** | S-7 Sources and evidence coverage | Service | Saying plainly what is missing is what makes any number trustworthy |
| **P2** | S-4 Dashboards, views and exploration | Secondary | What it adds beyond S-1 is composition; its boundary risks are S-9's checks run through exploration paths |
| **P2** | S-5 Sharing and reuse | Secondary | Matters at specific moments — a number leaving the product — not continuously |
| **P2** | S-10 Deployment, upgrade and migration | Service | Matters intensely at upgrade and migration moments, not continuously |
| **P3** | S-6 External comparison | Secondary | No surface exists yet — carried so the clean-room rule is not silently lost |

Read the three P0 rows as the answer to "what do we verify on every build": one main scenario and
two service ones, because those two are the only failures that cannot be undone.

For readers of the previous revision, the old function IDs map as: PF-3 → S-1 + S-4 · PF-6 → S-9 ·
PF-2 → S-8 · PF-4 → S-2 · PF-5 → S-3 · PF-1 → S-7 · PF-7 → S-5 + S-10 · PF-8 → S-6 plus the
forward-looking half of S-2.

Each block below: **Asserts** — what a check verifies beyond the product tables; **Invariants** —
which Appendix A rules bind here (Appendix A is the assertable form and wins over prose);
**Traces** — the `user-scenarios.md` questions feature tests are written against; **Covers** — the
vectors in the workspace's `quality/insight-quality-framework.md` this scenario feeds.

### S-1 · Metrics review · P0

**Asserts.** Of the three group-size floors only one is enforced today — a peer comparison is not
computed below five people with data (`MIN_PEER_N = 5` in the analytics metric-results compiler);
the four-person floor on a group figure and the eight-per-side floor on a conclusion are promises.
Checks assert the enforced floor as behavior and the promised floors as they land, and always
against people-who-have-data, never headcount. Ranking checks assert the **default** screen — a
named ranking a granted manager explicitly requests is permitted, so asserting impossibility is
asserting the wrong thing.
**Invariants.** A1–A4, A9, A14–A17. **Traces.** B1–B8 (B4 concentration, B6 cost and B7 cohorts are
the heterogeneous ones). **Covers.** Reliability.

### S-9 · Configuration and access · P0

**Asserts.** The persona × grant negative matrix — five kinds of data, each granted one by one, each
refusal enforced where the data is produced rather than hidden by the screen, so it holds for a
direct API call too **(QA addition)**. The admin role alone grants no metric visibility — asserted
against the identity service, not the UI. Expected values differ by mode: under flat, "their own
part of the organization" is the whole organization, so the matrix is evaluated per mode, not once.
**Invariants.** A11–A13. **Traces.** G1, B1 (IC isolation), B4 (anonymous cut), F1 (clean room).
**Covers.** Security.

### S-8 · Identity and organization model · P0

**Asserts.** Every automatic match carries its grounds and confidence; every merge and split is
reversible; a manual correction survives the next data refresh **and the next product upgrade**
**(QA addition)**. Past periods keep the shape and numbers they were reported under.
**Invariants.** — (none bind identity directly; it gates all of them instead.) **Traces.** A2; A3
and C7 blocked (Appendix B). **Covers.** Reliability.

### S-2 · Analysis and diagnosis · P1

**Asserts.** The refusal cases are the acceptance suite: exactly seven people on one side of a
comparison — one below the floor — must yield a withheld conclusion that says why; a metric under a
known defect must be excluded by name, not quietly folded in. Correlation is declared on the answer
itself. The forward-looking half (feasibility and investment ranking) has no screen — carried, not
asserted.
**Invariants.** A5–A8, A10, A16. **Traces.** C1, C8; C2–C6 blocked, E1–E2 unbuilt (Appendix B).
**Covers.** Reliability.

### S-3 · Conclusions: recommendation and validation · P1

**Asserts.** The lever's measure, the outcome, the guardrail and the validation window are all fixed
at issue time — pre-declaration is unrecoverable afterwards, so it must land with the first
recommendation, not after it. "Not enough data" is available as an honest outcome. The product
writes nothing back into a connected system. An IC is never the subject; the PEER row is an open
product decision.
**Invariants.** A13. **Traces.** D1; D2 and D3 blocked (Appendix B). **Covers.** Reliability,
Security.

### S-7 · Sources and evidence coverage · P1

**Asserts.** Never a zero standing in for missing data, never an estimate in place of an answer;
a comparison across two periods whose source windows do not overlap is withheld with the source
named. Every empty screen states its cause, for every persona including on an IC's own page.
**Invariants.** A5–A8. **Traces.** A1, A4, C8 (the freshness and window part), G3 (the timezone
caveat). **Covers.** Versatility, Reliability.

### S-4 · Dashboards, views and exploration · P2

**Asserts.** Exploration moves the question, never the boundary: the S-9 matrix re-run through
composed views, slices and follow-backs — an exploration path never reaches outside the viewer's
reach, a follow-back never reaches underlying records without that grant, and a composed group
figure is recalculated for what is on screen.
**Invariants.** A4, A9, A11, A12. **Traces.** B7; the rest of the B family is reached here by
composition rather than by reading. **Covers.** Reliability, Security.

### S-5 · Sharing and reuse · P2

**Asserts.** A number leaves the product together with its definition, coverage and confidence, and
the access rules that apply inside apply on the API and governed-access surface.
**Invariants.** A12, A13. **Traces.** G2. **Covers.** Versatility.

### S-10 · Deployment, upgrade and migration · P2

**Asserts.** Every view that worked before an upgrade works after it, and a breakage surfaces to
whoever ran the upgrade before a user meets it **(QA addition)**. History — everyone's — is kept
across the change; a migration states parity openly, including what could not be reproduced.
**Invariants.** — **Traces.** G4. **Covers.** Versatility, Reliability.

### S-6 · External comparison · P3

**Stub — deliberately.** No surface exists, so there is nothing to assert yet. Carried so the
clean-room rule is not silently lost: only anonymized, opt-in, revocable aggregates ever leave the
customer boundary.
**Invariants.** — **Traces.** F1. **Covers.** — (assigned when a surface exists.)

---

## 4. What this changes for the quality setup

1. **The 17 invariants are the first automation target.** They hold across every scenario above and
   each maps to a defect class already observed. Most are stand-independent — see Appendix A.
2. **The fixtures, not the test design, are the blocker.** Governance (A11–A13) needs every persona
   at once with realistic reporting lines; PEER needs a stand running flat; EXEC-without-ADMIN needs
   a fixture that separates the two roles `ceo` currently conflates. Until those exist, P0 coverage
   is partial by construction, however many checks are written.
3. **Every persona-specific check runs once per persona the stand's mode admits.** That is the
   practical meaning of §1: a pass as EXEC says nothing about IC, and a whole mode is dark until a
   stand runs it.
4. **S-9 is the security vector's content** — a persona × grant × mode negative matrix, not a
   scanner.
5. **S-2's acceptance suite must exist before the first vertical ships** — specifically the
   seven-on-a-side refusal and the excluded-by-name defective metric. By the scenario's own
   definition, an incomplete answer there is not thin but wrong.
6. **S-3's pre-declaration requirement expires.** It costs nothing at issue time and is
   unrecoverable afterwards; it lands with the first recommendation or the validation half is
   unbuildable later.
7. **Service does not mean second.** Two of the three P0 scenarios are service, because theirs are
   the only failures that cannot be undone.
8. **Two vectors still have no home here.** Efficiency (compute footprint) and Performance
   (per-endpoint latency) are measured on the reference-organisation fixture
   ([`REFERENCE-ORGS.md`](REFERENCE-ORGS.md)) rather than per persona, so no scenario claims
   them. That remains a real gap, and it is the same fixture question as item 2.

## 5. Open questions

Settled since the last revision:

- **Who owns E1 now that PM is folded away** — settled by the product doc: function is an axis of
  the user scenario, not a persona, so feasibility is asked by EXEC or a LEAD *from product* in S-2.
- **Is this the missing persona catalog the framework asks for** — the product doc is that catalog;
  this document is the QA baseline over it, and both now link.

Still open:

- **Which stand hosts which scenario?** S-1 needs organization-scale data, S-2 needs eight per side,
  S-9 needs every persona with realistic reporting lines, and PEER needs a flat stand — one chart
  value on a compose release would create one, but nobody owns it. No seeded stand satisfies any of
  the four today.
- **Are the five kinds of access enforced today, or only designed?** S-9's matrix and the ADMIN
  boundary both depend on the answer.
- **Does ADMIN get metric visibility by default on current stands?** The product doc asserts it must
  not; confirming it means asking the identity service, not the UI.
- **Is the e2e rig in scope for the persona markers?** It is the second strict-marker root; leaving
  it out halves the meta test's value, taking it in drags the markers into a slower suite.
- **When does `member_session` become `ic_session`?** The rename is agreed; it touches every test
  that names the fixture.
- **The product doc's own open points that block QA expected values:** whether flat mode's position
  band is a ranking (blocks PEER's S-1 assertions), who owns a recommendation under flat (blocks the
  S-3 PEER row), and whether the four / eight thresholds are customer-configurable (blocks writing
  the numbers into assertions).

---

## Appendix A · Cross-scenario invariants

The product declares these rules itself, and each is a defect class already seen. They hold across
every scenario above, so they are asserted once and reused rather than re-implemented inside each
feature's tests. **This is the first automation target.**

Numbering is local to this document (A1–A17) — `user-scenarios.md` numbers its own rules
differently, so cite these as "A4", never as "rule 4".

**Metric grammar** (S-1, inherited by S-2 and S-4)

- **A1.** Counts are always shown per active person, with the change since last period; a team total
  may appear as a caption, never as the headline number.
- **A2.** Averages and medians are never added together.
- **A3.** "Concentration" figures — what share sits with the top tenth — are computed only over
  figures that can be added up, and framed per area: a risk in development and the wiki, a workload
  imbalance in communication.
- **A4.** Group figures are recalculated for the group on screen, never carried over from a wider
  scope.

**Honest emptiness / data quality** (S-7, S-2)

- **A5.** Never a zero in place of missing data (`#1517`).
- **A6.** Never a rough estimate in place of an answer.
- **A7.** Two periods are compared only over time both sources cover.
- **A8.** A metric with a known defect says so on the metric itself.

**Suppression thresholds** (S-1, S-2, S-4) — *each counts people who have data, never headcount*

- **A9.** A group of fewer than four people is not shown at all, in any chart. *A promise, not yet
  enforced.*
- **A10.** A conclusion needs at least eight people on each side; below that it does not render, and
  says why. *A promise, not yet enforced.*
- **A17.** A peer comparison — the median and quartiles behind a person's position — is not computed
  below five people with data. *The one threshold enforced today (`MIN_PEER_N = 5`, analytics
  metric-results compiler); it protects IC's median and every PEER comparison.*

**Governance** (S-9) — *the three that need the full persona set*

- **A11.** No default screen ranks named individuals against one another. Naming people is expected
  where person-level access has been granted — a manager's team view names their reports; what is
  forbidden is the ranking, and naming anyone outside the viewer's granted part of the organization.
- **A12.** A viewer cannot reach outside their own part of the organization — enforced where the
  data is produced, so it holds for a direct API call too. *Under flat, "their own part" is the
  whole organization: the assertion's expected value depends on the stand's mode.*
- **A13.** The product writes only its own settings and notes, never back into a connected system.

**Cost & AI** (S-1, S-2)

- **A14.** Usage-priced and seat-priced costs are never added into one figure; cost that cannot be
  attributed to anyone gets its own line.
- **A15.** No single "value of AI" number; cost that moves from one area to another is shown as
  moving, not folded away.
- **A16.** Where a claim is a correlation, the answer says so on itself, not in a footnote.

**Where they can run.** A1–A10, A14–A16 and A17 are assertions against API responses and need only
a stand with data — though A12's and A17's expected values depend on the stand's mode. A11–A13 need
every persona present at once, with realistic reporting lines — the fixture that does not exist yet.

---

## Appendix B · Blocked, and by what

Carried at scenario level so that "we don't test it" reads as a consequence of a known gap rather
than an omission.

| Scenario | What is blocked | Blocker |
|---|---|---|
| every scenario, as PEER | All PEER assertions | No test stand runs flat and no peer fixture exists; one chart value on a compose release would create the stand, but it has no owner |
| every scenario, EXEC vs ADMIN | Proving an EXEC promise is not an artefact of the admin role | The seed `ceo` holds both; no EXEC-without-ADMIN login exists |
| S-8 | The operator queue for working through matches (A2) | The matching engine alone is a table in a database; the queue UI has no owner |
| S-8 | Expected functions / role model (A3), role-vs-work divergence (C7) | `#1455` blocked by `#1873` (both open, 2026-09-02) |
| S-2 | Review as bottleneck (C2) | `silver.fct_git_review` empty; reviewer keys in another namespace (`#1985 §3`); on dev, review attribution is 0% (`#3013`) |
| S-2 | Quality held after a speed-up (C3) | Former blocker `#2027` closed — unblocked on paper; needs re-triage before tests are written |
| S-2 | Support load vs release (C4), sales activity vs pipeline (C5) | Support/CRM outside the semantic layer (`#1930`) |
| S-2 | Cost moved rather than fell (C6) | Not built — no downstream cost-movement model |
| S-2 | Feasibility and investment ranking (E1, E2) | No screen exists yet |
| S-3 | Outcome read-back (D2), period annotations (D3) | Not built — but pre-declaration must land with D1 (§4) |
| S-3 | The PEER row | Open product decision: recommendation ownership in a flat organization |
| S-6 | Everything | No surface exists yet |
