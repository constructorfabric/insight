# SCENARIOS.md → the stand suite

What [`docs/product/SCENARIOS.md`](../../docs/product/SCENARIOS.md) promises,
which of it `tests/stand/` can prove today, and what is left. Produced with the
`stand-scenarios` skill; re-derive it rather than editing a cell — the code
wins over this page, and a stale row here is worse than no row.

**This is not a test-case database.** It carries no case bodies, no execution
status, no per-run results. It is a triage: surface verdicts, the claims worth
implementing, and the honest gaps. Claims become pytest cases under
`tests/stand/`, and the test is then the source of truth for what is asserted.

## Method

Five questions per claim, in order (`stand-scenarios`): which persona and which
seeded fixture · **does the surface exist** · what is the oracle · which layer ·
what already tests this. Question 2 kills more candidates than the other four
together.

Priority is **blast radius**, first trigger that fires — P0 a caller served
data they are not entitled to, P1 a whole view or contract broken for a class of
user, P2 one control with a workaround. Not screen prominence.

---

## 1. Surface verdict

| Scenario | Verdict | Basis |
|---|---|---|
| **S-1** Metrics review | **Partial** | `POST /v1/metric-results`, `/v1/metric-definitions`, `/v1/metric-drilldown` all answer. Reach and honesty are assertable; **values are not** — `golden_metrics` is empty by design |
| **S-2** Analysis and diagnosis | **No surface** | no conclusion, bottleneck, anomaly or forecast endpoint exists |
| **S-3** Recommendations and validation | **No surface** | no recommendation object, no validation window |
| **S-4** Dashboards and exploration | **Partial** | `/v1/queries` CRUD + run, `/v1/metric-drilldown` exist; saved/shared *views* are an Appendix C proposal, not a surface |
| **S-5** Sharing and reuse | **Partial** | `POST /v1/metric-drilldown/export` exists; "the definition travels with the number" is unverified in the export payload |
| **S-6** External comparison | **No surface** | Appendix C says so outright |
| **S-7** Sources and evidence coverage | **Partial** | the catalogue carries `schema_status` / `schema_error_code` / `last_observed_date`; readiness mode itself is unbuilt, and this stand declares `ingestion: no` |
| **S-8** Identity, roles, org model | **Built** | `/v1/profiles`, `/v1/subchart`, `/v1/visible-persons`, roles + person-roles + journals |
| **S-9** Configuration and access | **Built** | `require_admin`, `/v1/visibility`, org-scope resolution on every person-keyed route |
| **S-10** Deployment and migration | **Out of scope** | the suite tests a running stand; install/upgrade/parity is the chart and gitops suites' territory |

Five of ten produce claims. Two of those five — **S-8 and S-9** — are where the
product's own boundary language and the stand's capabilities line up exactly,
and they are already the best covered.

The §5 invariants are triaged separately in
[`.claude/skills/stand-scenarios/invariants.md`](../../.claude/skills/stand-scenarios/invariants.md):
`Built` for R1, R5, R11 · `Partial` for R4, R9, R10, R12 · `No surface` for R2,
R3, R6, R8 · not enforceable here for R7.

---

## 2. Already proved, without naming it

Shipped tests that prove a SCENARIOS.md clause today. Worth recording because
the next person to read the product doc will otherwise design them again.

| Clause | Proved by |
|---|---|
| §S-1 ADMIN "Nothing by default · never gains data visibility implicitly from administrative rights" | `test_operator_sees_nobody_in_the_org_chart` |
| §1.1 ADMIN "administrative rights do **not** carry the right to see data" | `test_admin_listing_is_403_for_a_realm_admin_without_the_grant` |
| §S-9 LEAD "never sideways" | `test_two_leads_of_different_teams_see_different_people`, `test_subchart_of_someone_out_of_scope_is_404_not_403` |
| §1.1 IC "How far they see: themselves" | `test_org_visibility_scope_differs_by_persona` (a plain member sees nobody in the org chart) |
| §5 R5 "the boundary holds by construction" | `test_metric_results_403_for_a_person_out_of_scope`, `test_a_person_outside_the_callers_scope_is_404_not_403` |
| §5 R5 tenant half | `test_only_the_people_the_caller_may_see_come_back`, `test_an_email_in_another_tenant_is_404` |
| §S-8 IC "Be one person, not four" | `test_person_id_and_email_are_two_spellings_of_one_identity` |
| §S-9 ADMIN "grants the five kinds of data one by one" (one of them) | `test_a_visibility_grant_changes_what_the_grantee_can_see` |
| §5 R1 "never a zero for missing data" (UI half) | `test_the_personal_dashboard_renders_every_metric_domain`, `test_the_team_view_lists_every_report_the_roster_declares` |
| §S-1 LEAD privacy of a mixed request | `test_one_hidden_person_refuses_the_whole_request` — a *partial* answer would disclose which ids are visible |

**Appendix C, first open point** — "Administrative rights and data visibility …
worth confirming against the access model rather than the interface" — is
already confirmed at the API, by the first two rows. That open point can be
closed.

---

## 3. Designed claims

### The lead's upward boundary

**Source** SCENARIOS.md §S-9 LEAD, §1.1 LEAD *Never*. **Personas** `dev_lead`,
`ceo`. **Oracle** the seeded org edges (`deploy/seed/profiles.py:269-314` — CEO
`parent_uuid=None`, each lead parented to the CEO, each IC to their lead).

**S9-L-03 — a lead asking for their own manager's subchart is refused, 404 not
403.** "Never one level up" is stated beside "never sideways" and only sideways
is tested. A 403 would confirm the CEO exists, so the refusal itself would leak
the shape of the org while denying the data. API `GET /v1/subchart/{id}` · P0 ·
new · `[VERIFIED]`

**S9-L-04 — the lead's own subchart forest contains nobody above them.**
Distinct from S9-L-03: that one is a point lookup, this is the listing.
`test_org_visibility_scope_differs_by_persona` asserts `lead_view ⊆ admin_view`,
which is satisfied by a lead who *can* see the CEO. API `GET /v1/subchart` ·
P0 · new · `[VERIFIED]`

### The IC boundary

**Source** §1.1 IC *Never*, §S-1 IC. **Personas** `development_ic`, `sales_ic`.
Appendix C names this the tightest boundary and "the combination that is easiest
to get wrong quietly".

**S1-I-01 — an IC asking for a colleague's metrics is refused.** The shipped
403 case (`test_metric_results_403_for_a_person_out_of_scope`) calls as a
**lead**; nothing exercises the gate with the narrowest principal, and an IC is
the caller the rule is written for. API `POST /v1/metric-results` with
`member_session` · P0 · new · `[VERIFIED]`

**S1-I-02 — an IC asking for a colleague's drilldown evidence is refused.** The
drilldown returns per-row evidence, so a leak here is worse than a leaked
aggregate. `test_git_commit_drilldown_refuses_a_person_out_of_scope` exists as a
lead; the IC variant does not. API `POST /v1/metric-drilldown` · P0 · new ·
`[VERIFIED]`

### EXEC reach

**Source** §1.1 EXEC, §S-9 EXEC. **Persona** `ceo`.

**S9-E-01 — the CEO's subchart contains every person the manifest places in the
tenant, and nobody from another tenant.** The existing assertion is relational
(`admin_view > lead_view`), which a CEO missing half the org would satisfy.
Oracle the manifest roster · API `GET /v1/subchart` · P1 · partially covered by
`test_org_visibility_scope_differs_by_persona` · `[VERIFIED]`

**S9-E-02 — raw access is a grant, not a privilege of rank.** §S-9 EXEC:
"the underlying records only where granted — raw access is one of the five
grants, not a privilege of rank." The CEO holds no `admin` row, so the admin
routes refuse them — covered. What is untested is the *converse* for each of the
remaining grant kinds. P1 · new · `[INFERRED]` — the implementer must first
establish which of the five kinds have a distinct enforcement point.

### A grant widens exactly one viewer

**Source** §S-9 ADMIN "grants the five kinds of data one by one".

**S9-A-03 — granting visibility to one viewer leaves every other viewer's reach
unchanged.** `test_a_visibility_grant_changes_what_the_grantee_can_see` proves
the grant works; nothing proves it is *scoped*. A grant that widened everyone
would pass the existing test. API `/v1/visibility` + two subchart reads · P0 ·
partially covered · `[VERIFIED]`

### Missing data is absent, not zero

**Source** §5 R1, §S-7 "Not this". **Oracle** the contract.

**R1-01 — an unmeasured metric answers `null`, never `0`.** The UI half is
covered; the API half is not, and the two are indistinguishable on screen — a
backend returning `0` and a frontend rendering `—` for falsy look identical.
API `POST /v1/metric-results` · P1 · new · `[VERIFIED]`

**R11-01 — every metric definition carries a `schema_status`, and one in
`error` carries a `schema_error_code` rather than a bare flag.** §5 R11: a
metric with a known defect says so on the metric itself. The fields exist
(`MetricDefinitionView`) and no test reads them. API
`GET /v1/metric-definitions` · P1 · new · `[VERIFIED]`

**R11-02 — `last_observed_date` is absent, not a placeholder date, where
nothing was ever observed.** R1 applied to the catalogue: a placeholder here
would misreport freshness for every consumer downstream. API · P1 · new ·
`[VERIFIED]`

### Role history

**Source** §5 R9, §S-8 LEAD/EXEC "never a past period silently recalculated".

**R9-01 — revoking a role does not change a past period's figures.** Needs no
value oracle: read the same past window twice, either side of the revoke, and
assert the two reads **agree**. That is the whole claim, and it is the temporal
guarantee the doc leans on in three separate scenarios. API `/v1/person-roles` +
two `/v1/metric-results` reads · P1 · new · `[SUPPORTED]`

### The group figure is computed for the group in view

**Source** §5 R12.

**R12-01 — the same metric and cohort at team scope and at organization scope
return a different `n`.** `PeerValueDto` carries the cohort size, so the claim
needs no hand-authored number — the assertion is that the two reads *disagree*,
which is exactly what the rule says must happen. P1 · new · `[INFERRED]` —
**blocked**: establish first whether the backend `peer` view returns populated
statistics on this stand, or whether the SPA synthesises them (see §4).

### No default ranking

**Source** §5 R4.

**R4-01 — the team view does not arrive sorted by any metric value and exposes
no composite score.** An absence claim, so weak alone: pair it with the
existing positive that every report is named, so the two together distinguish
"correctly not ranking" from "rendered nothing". UI · P2 · new · `[SUPPORTED]`

---

## 4. Findings

Things the pass surfaced that are decisions or defects, not test work.

**1 · The four-person suppression rule is enforced by the screen.** §5 rule 10
says a group figure is not shown below four people; §5 rule 5 says a boundary
must hold *by construction, not as a filter a screen applies*. `MIN_COHORT = 4`
lives in `src/frontend/src/lib/insight/within-team-peer.ts:17`, and its own
comment records why: *"Peer stats aren't computed by the backend yet, so the
shared members grid paints every cell neutral … a single, honest, client-side
computation."* No equivalent constant was found under `src/backend/`. Any API
consumer — and §S-5 promises the API is one — is outside that protection. This
needs a product decision before it needs a test.

**2 · R11's exclusion half is unbuilt.** "Where a conclusion would rest on a
defective metric, the metric is excluded by name" depends on conclusions (S-2),
which do not exist. Narrower point: the metric caveats that *are* written down
live in migration comments
(`m20260601_000002_seed_claude_team_metrics_catalog.rs`,
`m20260606_000001_dept_metric_distributions.rs`) and reach no surface at all.

**3 · Appendix C's first open point is answerable now** — see §2. ADMIN gaining
no implicit data visibility is enforced at the API and tested.

**4 · Appendix C's identity-correction point cannot be settled here.** "A
correction surviving the next sync" needs a sync to run, and this stand declares
`ingestion: no`. It belongs to the ingestion path, not this suite.

---

## 5. Blockers and seed gaps

| Blocker | Blocks | Note |
|---|---|---|
| No cohort below four people | the negative case for §5 R10 | every seeded team is five ICs + a lead; needs a small team in `deploy/seed/profiles.py` |
| `other_tenant_lead` has no activity and no org edge | any cross-tenant claim about *data* rather than *identity* | deliberate — they exist only so refusal has a caller |
| `ingestion: no` | S-7 readiness, R3 lineage, R7 read-only-towards-sources, identity-correction survival | compose seeds silver/gold directly |
| `golden_metrics` empty | every value assertion, in every scenario | deliberate; admission criteria in `deploy/seed/golden_metrics.py` |
| Where peer statistics are computed | R12-01 | read the `peer` view handler and `withinCohortPeer` before implementing |
| Which of the five grant kinds have distinct enforcement points | S9-E-02 | read the access model |

---

## 6. Out of scope for this suite

S-10 (deployment, upgrade, migration) tests the act of installing, which this
suite deliberately cannot do — *a run that could bring its own stand up would
hide exactly the deployment failures this suite exists to catch*.

The metric×view matrix and per-metric value assertions belong to the blocking
gate in `src/ingestion/tests/e2e/` and are not re-specified here.
