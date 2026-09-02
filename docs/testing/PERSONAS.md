# Constructor Insight — Personas & Scenarios (quality baseline)

**Date:** 2026-09-02 · **Status:** draft, for QA review
**Product source of truth:** [`SCENARIOS.md`](../product/SCENARIOS.md) — the personas, the
scenarios, their per-persona boundaries and the rules that hold in every scenario. The individual
user questions (A1…G4) stay in the workspace's `quality/user-scenarios.md`. This document does not
restate either; where it adds a requirement of its own, the line is marked **(QA addition)**.

**What this document holds** — the QA half the product doc deliberately omits: the logins and what
a failure costs (§1), the mapping from test fixtures and markers to personas and scenarios (§2),
the order in which the scenarios are verified and what a check adds beyond the product tables (§3),
the invariant check registry (Appendix A), and what is blocked and by what (Appendix B).

**Vocabulary.** A *stand* is a test environment running a full copy of the product. The two
*visibility modes* (`org_chart`, `flat`) are defined in the product doc's Personas section.

> **Where this stands today.** The 17 invariants in Appendix A are the first thing worth automating,
> and most can start now: they are assertions against API responses and run on any stand that has
> data. The exceptions are governance (A11–A13), which needs every persona present at once on one
> stand with realistic reporting lines, and anything PEER — **and no test stand satisfies either
> today**: none runs flat, and no seeded org carries the full persona set. Those fixtures, not the
> test design, are the blocker on P0 coverage.

---

## 1. Personas — the QA view

Who each persona is, how far they see and what bounds them lives in the product doc. What QA adds
is the login and the failure cost.

The **mode is a property of the stand.** At test time it is read from `GET /v1/me`
(`visibility_policy`), once per session — never assumed, never taken from a manifest. Observed
2026-09-01: dev and qaclust run `org_chart`; cf-prod runs `flat`. No stand we control for testing
runs flat.

| Persona | Who we log in as, on a stand | What a failure costs them |
|---|---|---|
| **EXEC** | The seed's `ceo`. That account also holds the admin role, so there is no EXEC-without-ADMIN login today (Appendix B) | A board-level number that is wrong or unexplained |
| **LEAD** | A seed `*_lead`, picked with at least eight reports so eight-per-side comparisons are exercisable | Acting on a jam that is a data artefact |
| **IC** | A seed `*_ic` | A leak of what the product must withhold from them, or their own work credited away |
| **PEER** | No fixture — blocked on a flat stand (Appendix B) | An identifiable person behind a figure the floor should have withheld |
| **ADMIN** | The seed's `admin_operator`, which also carries access to the stand itself | Shipping a customer that silently proves nothing |
| **none** | An account from another tenant, or one no person resolves to | Anything at all: the only correct responses are a refusal or an empty scope |

Three consequences for the setup:

- **A run covers one mode.** Which personas a stand can host follows from its mode (the product
  doc's mode table); `none` is QA's own login, not a product persona, and exists under both. A pass
  as EXEC says nothing about IC — and nothing at all about PEER until a flat stand exists.
- **Refusals are checked first where the product bounds tightest.** Under `org_chart` that is IC;
  under `flat` it is the five-person floor (A17) — the one limit the mode leaves standing.
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
product doc. This section holds only the verification order and what a check adds beyond them.

**The rule that produced the order:** a service scenario is verified before a main one in two cases
— when the main tier cannot produce a correct answer without it, or when its failure cannot be
undone afterwards. Identity (S-8) is the first case: a wrong person makes every number wrong.
A governance breach (S-9) is the second: no patch un-leaks a name.

| Priority | Scenario | Why here |
|---|---|---|
| **P0** | S-1 Metrics review | Everything the product says stands on it, for every persona |
| **P0** | S-9 Configuration and access | Irreversible failure: one leak of a named individual is not fixable by a patch |
| **P0** | S-8 Identity and organization model | A wrong person makes every metric wrong |
| **P1** | S-2 Analysis and diagnosis | The first vertical ships here, and its acceptance suite must exist first (§4) |
| **P1** | S-3 Conclusions: recommendation and validation | Its pre-declaration check expires at issue time |
| **P1** | S-7 Sources and evidence coverage | Honest emptiness is what makes every other number trustworthy |
| **P2** | S-4 Dashboards, views and exploration | Verified by re-running S-9's checks through exploration paths |
| **P2** | S-5 Sharing and reuse | Checked at export moments, not continuously |
| **P2** | S-10 Deployment, upgrade and migration | Checked at upgrade and migration moments, not continuously |
| **P3** | S-6 External comparison | A stub, carried so the clean-room rule is not silently lost |

Read the three P0 rows as the answer to "what do we verify on every build": one main-tier scenario
and two service ones, because those two are the only failures that cannot be undone.

For readers of the previous revision, the old function IDs map as: PF-3 → S-1 + S-4 · PF-6 → S-9 ·
PF-2 → S-8 · PF-4 → S-2 · PF-5 → S-3 · PF-1 → S-7 · PF-7 → S-5 + S-10 · PF-8 → S-6 plus the
forward-looking half of S-2.

Each block below: **Checks** — only what QA adds on top of the product tables; a block with nothing
to add carries none. **Invariants** — the Appendix A checks that bind here. **Traces** — the
`quality/user-scenarios.md` questions feature tests are written against. **Covers** — the vectors
in the workspace's `quality/insight-quality-framework.md` this scenario feeds.

### S-1 · Metrics review · P0

**Checks.** Of [rule 10](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)'s three
floors only A17 is enforced — asserted as behavior; A9 and A10 are asserted as they land. Ranking
checks target the default screen, never impossibility
([rule 4](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)).
**Invariants.** A1–A4, A9, A14–A17. **Traces.** B1–B8 (B4 concentration, B6 cost and B7 cohorts are
the heterogeneous ones). **Covers.** Reliability.

### S-9 · Configuration and access · P0

**Checks.** The persona × grant negative matrix over the five kinds of data
([rule 5](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)), asserted with direct API
calls and evaluated once per mode — the expected values differ between `org_chart` and `flat`. The
admin-role check points at the identity service, not the UI.
**Invariants.** A11–A13. **Traces.** G1, B1 (IC isolation), B4 (anonymous cut), F1 (clean room).
**Covers.** Security.

### S-8 · Identity and organization model · P0

**Checks.** A manual correction must survive a product upgrade, not only the next sync
**(QA addition)**.
**Invariants.** — (none bind identity directly; it gates all of them instead.) **Traces.** A2; A3
and C7 blocked (Appendix B). **Covers.** Reliability.

### S-2 · Analysis and diagnosis · P1

**Checks.** The refusal cases are the acceptance suite. The concrete case: exactly seven people on
one side of a comparison — one below A10's floor — must produce a withheld-with-reason response.
**Invariants.** A5–A8, A10, A16. **Traces.** C1, C8; C2–C6 blocked, E1–E2 unbuilt (Appendix B).
**Covers.** Reliability.

### S-3 · Conclusions: recommendation and validation · P1

**Checks.** Pre-declaration is unrecoverable after a recommendation is issued, so its check must
land with the first recommendation, not after it (§4).
**Invariants.** A13. **Traces.** D1; D2 and D3 blocked (Appendix B). **Covers.** Reliability,
Security.

### S-7 · Sources and evidence coverage · P1

**Invariants.** A5–A8. **Traces.** A1, A4, C8 (the freshness and window part), G3 (the timezone
caveat). **Covers.** Versatility, Reliability.

### S-4 · Dashboards, views and exploration · P2

**Checks.** The S-9 matrix re-run through composed views, slices and follow-backs.
**Invariants.** A4, A9, A11, A12. **Traces.** B7; the other B-family questions are reached here
through composition. **Covers.** Reliability, Security.

### S-5 · Sharing and reuse · P2

**Invariants.** A12, A13. **Traces.** G2. **Covers.** Versatility.

### S-10 · Deployment, upgrade and migration · P2

**Checks.** A breakage after an upgrade must surface to whoever ran the upgrade before a user meets
it **(QA addition)**.
**Invariants.** — **Traces.** G4. **Covers.** Versatility, Reliability.

### S-6 · External comparison · P3

**Stub — deliberately.** Nothing to assert until a surface exists (Appendix B); carried so
[rule 8](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario) stays in the verification
order.
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
   seven-on-a-side refusal (its Checks line) and A8's exclusion check. By the scenario's own
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

- **Who owns E1 now that PM is folded away** — settled by the product doc's function axis: the
  question rides S-2.
- **Is this the missing persona catalog the framework asks for** — the product doc is that catalog;
  this document is the QA baseline over it, and both now link.

Still open:

- **Which stand hosts which scenario?** S-1 needs organization-scale data, S-2 needs eight people
  with data on each comparison side, S-9 needs every persona with realistic reporting lines, and
  PEER needs a flat stand — one chart value on a compose release would create one, but nobody owns
  it. No seeded stand satisfies any of the four today.
- **Are the five kinds of access enforced today, or only designed?** S-9's matrix and the ADMIN
  boundary both depend on the answer.
- **Does ADMIN get metric visibility by default on current stands?** Confirming the product doc's
  promise means asking the identity service, not the UI.
- **Is the e2e rig in scope for the persona markers?** It is the second strict-marker root; leaving
  it out halves the meta test's value, taking it in drags the markers into a slower suite.
- **When does `member_session` become `ic_session`?** The rename is agreed; it touches every test
  that names the fixture.
- **The product doc's [Open points](../product/SCENARIOS.md#7-open-points) that block expected
  values:** *Ranking in flat mode* blocks PEER's S-1 assertions; *Recommendation ownership in a
  flat organization* blocks the S-3 PEER row; *The group-size thresholds* blocks hardcoding four
  and eight — until it is decided, checks reference the rule, not a literal.

---

## Appendix A · Cross-scenario invariants

The check registry: each entry is a defect class already seen, asserted once and reused rather than
re-implemented inside each feature's tests. **This is the first automation target.** The product doc
owns what each rule means; an entry here names its owner and adds only what a check needs —
enforcement status, code anchors, defect references, run requirements. Three entries (A1–A3) have no
product owner: they are QA conventions carried from the workspace's `quality/user-scenarios.md`.

Numbering is local to this document (A1–A17) — cite these as "A4", never as "rule 4", which names
the product doc's own numbering.

**Metric grammar** (S-1, inherited by S-2 and S-4)

- **A1 · per-person counts.** QA convention: counts are shown per active person with the change
  since last period; a team total is a caption, never the headline number.
- **A2 · no mixed averages.** QA convention: averages and medians are never added together.
- **A3 · concentration additivity.** The S-1 LEAD row owns the domain framing; the QA half:
  a top-tenth share is computed only over figures that can be added up.
- **A4 · group-figure recalculation.** [Rule 12](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).

**Honest emptiness / data quality** (S-7, S-2)

- **A5 · no zero for missing data.** [Rule 1](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario) — observed as `#1517`.
- **A6 · no estimate in place of an answer.** [Rule 1](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).
- **A7 · overlapping windows only.** The [S-7](../product/SCENARIOS.md#s-7--sources-and-evidence-coverage) LEAD, EXEC row.
- **A8 · defect declared, and excluded from conclusions.** [Rule 11](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).

**Suppression thresholds** (S-1, S-2, S-4) — all three owned by [rule 10](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario)

- **A9 · the group-figure floor.** Not yet enforced.
- **A10 · the per-side conclusion floor.** Not yet enforced.
- **A17 · the peer-comparison floor.** Enforced: `MIN_PEER_N = 5` in the analytics metric-results
  compiler.

**Governance** (S-9) — *the three that need the full persona set*

- **A11 · no default ranking.** [Rule 4](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).
- **A12 · reach held at the API.** [Rule 5](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario) — the expected value depends on the stand's mode.
- **A13 · no write-back.** [Rule 7](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).

**Cost & AI** (S-1, S-2)

- **A14 · cost kinds never summed.** [Rule 6](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).
- **A15 · cost movement preserved.** [Rule 6](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario).
- **A16 · correlation declared.** [Rule 2](../product/SCENARIOS.md#6-rules-that-hold-in-every-scenario) and the [S-2](../product/SCENARIOS.md#s-2--analysis-and-diagnosis) *Not this*.

**Where they can run.** All but A11–A13 are assertions against API responses and need only a stand
with data — though A12's and A17's expected values depend on the stand's mode. A11–A13 need every
persona present at once, with realistic reporting lines — the fixture that does not exist yet.

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
| S-3 | The PEER row | The product doc's open point *Recommendation ownership in a flat organization* |
| S-6 | Everything | No surface exists yet |
