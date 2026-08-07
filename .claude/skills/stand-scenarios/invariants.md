# §5 — the twelve rules, as an assertion checklist

SCENARIOS.md §5 states twelve rules that hold in *every* scenario. They are the
highest-leverage claims in the document: one covers several scenarios at once,
and a violation is usually silent.

For each: what it asserts, **where it is enforced today**, and whether the
deployed stand can see it. Verdicts are `Built` · `Partial` · `No surface` ·
`Not enforceable here`.

Re-verify the "enforced at" column before leaning on it — it is a reading of the
code at a point in time, and the code wins over this page.

---

## R1 · Evidence gaps are shown, not hidden

> never a zero for missing data, never an approximate estimate in place of an
> answer

**Verdict: Built — and the strongest non-access claim on this stand.**

A zero looks like a measurement and raises a false alarm; the distinction
between *absent* and *zero* has to survive the whole chain.

Enforced at: the API returns `null` rather than `0` for an unmeasured value
(`PeerValueDto` and the period view both type the value as nullable); the SPA
renders `—` / "not recorded" / an explicit empty card.

Partially covered: `test_the_personal_dashboard_renders_every_metric_domain`
asserts the unseeded Wiki domain shows "No data" and
"No metrics with data for this period.", and that populated KPI tiles are
`not_to_have_text("—")`. `test_the_team_view_lists_every_report_the_roster_declares`
asserts an unrecorded cell renders as unrecorded rather than as a number.

Gap worth a claim: the **API** half — that an unmeasured metric answers `null`
and never `0`. The UI cannot prove it; a backend that returned `0` and a
frontend that rendered `—` for falsy would look identical on screen.

## R2 · Confidence and limitations travel with every conclusion

**Verdict: No surface.** Conclusions are S-2, which is not built. Revisit when
a diagnosis surface ships.

## R3 · Lineage before attribution

> untraceable work is a gap, not a quiet claim

**Verdict: No surface** on this stand. Lineage is Appendix B 8.3, which has no
scenario of its own; the repair path is ADMIN work in S-7 and needs
`ingestion`, which this stand declares `no`.

## R4 · No default ranking of named individuals, and no unexplained productivity scores

> people are named where person-level access has been granted; **the ranking is
> what is ruled out**

**Verdict: Partial.** The claim is about a *default surface*, so it is a UI
claim: the team view must not arrive sorted by a score, and must expose no
composite "productivity" figure. Assertable as an absence — no rank column, no
score field, a default ordering that is not by value.

Weak by nature: an absence claim passes for many wrong reasons. Pair it with a
positive claim that names are present where access is granted, so the two
together distinguish "correctly not ranking" from "rendered nothing".

## R5 · People-level access is role-based and policy-controlled

> five kinds of data, granted separately. The scope boundary holds **by
> construction, not as a filter a screen applies**: a viewer is handed their own
> part of the organization and cannot ask for more.

**Verdict: Built — the best-covered rule, and still the one with the most
remaining claims.**

"By construction, not a filter" is the operative phrase: every claim of this
family belongs at the **API** layer. A boundary that only the SPA enforces is
the defect this rule exists to rule out.

Enforced at: `require_admin` (an active `admin` row in
`identity.person_roles`); org-scope resolution on `/v1/subchart`,
`/v1/profiles`, `/v1/visible-persons`; the visible-set check on
`POST /v1/metric-results` and `/v1/metric-drilldown`.

Covered: `test_only_the_people_the_caller_may_see_come_back`,
`test_org_visibility_scope_differs_by_persona`,
`test_two_leads_of_different_teams_see_different_people`,
`test_subchart_of_someone_out_of_scope_is_404_not_403`,
`test_a_person_outside_the_callers_scope_is_404_not_403`,
`test_metric_results_403_for_a_person_out_of_scope`,
`test_one_hidden_person_refuses_the_whole_request`,
`test_a_visibility_grant_changes_what_the_grantee_can_see`,
`test_operator_sees_nobody_in_the_org_chart`.

Gaps: **upward** reach (a lead vs their own manager) is unnamed; the "five
kinds of data granted separately" enumeration has no claim distinguishing them.

Note the two-filter structure on `visible-persons`: ids resolve within the
caller's **tenant** first, then narrow to org-chart visibility. A regression in
either leaks a different thing, so name both.

## R6 · Cost movement is preserved, not folded away

> seat-based and usage-based cost are never one figure

**Verdict: Partial — there is no cost *endpoint*, but there is a cost
*metric*, and the rule is about figures.**

`registry.yaml:230-242` defines `ai.cost` ("AI usage cost", `format: currency`,
`entity_type: person`), served by `/v1/metric-definitions` and
`/v1/metric-results` like any other metric.

Its `explanation` states the separation this rule demands, in the product's own
words: *"Includes usage a seat or subscription already covered, and excludes
seat and subscription fees, so it is not the amount invoiced."* So the claim is
assertable at `GET /v1/metric-definitions` today — the definition must carry
that qualification rather than presenting a single blended figure.

The onward half — cost *movement* preserved as a shift across teams — has no
surface.

## R7 · Insight observes and advises; people act

> it writes its own configuration and annotations, nothing else

**Verdict: Not enforceable here.** Read-only-towards-sources is a property of
the ingestion path, and this stand declares `ingestion: no` — no connector runs.
The in-process rig and the connector suites are the right home.

## R8 · Clean room

> raw data stays inside the customer boundary; only anonymized aggregates are
> shared, opt-in and revocable

**Verdict: No surface.** Benchmarks are S-6, which Appendix C records as having
no surface yet.

## R9 · Role and activity are separate axes

> expected role model and observed activity are compared, never conflated;
> history is kept under the model valid at the time

**Verdict: Partial.** The role half is built —
`GET/POST /v1/roles`, `GET/POST/DELETE /v1/person-roles`, with an `active` flag
and a journal. The *temporal* half (past periods stay under the model valid at
the time) is the interesting claim and is untested: revoking a role must not
retroactively change a past period's figures.

Covered: `test_person_role_grant_and_revoke_round_trip`,
`test_person_roles_filtered_by_role_and_active_narrows_on_both`,
`test_revoking_the_last_active_admin_is_refused`.

Gap: the temporal guarantee. Needs a metric read before and after a role change
over the same past window — and note it does **not** need a metric *value*
oracle, only that the two reads agree.

## R10 · Two group-size thresholds, and they are different

> a group figure is not shown below **four** people; a comparative conclusion is
> not drawn below **eight** on each side

**Verdict: Built server-side — and the interesting part is that the two
enforcement points disagree on the number.**

There are two thresholds in the product, not one:

| Where | Constant | Value | Behaviour below it |
|---|---|---|---|
| Backend | `MIN_PEER_N` (`analytics/src/domain/metric_results/compiler.rs:25`) | **5** | the peer view still reports `n`, but `p25`/`median`/`p75`/`min`/`max` come back `NULL` |
| Frontend | `MIN_COHORT` (`src/frontend/src/lib/insight/within-team-peer.ts:17`) | **4** | the client-synthesised within-cohort peer view yields no stats (neutral cells) |

The backend one is compiled into the peer SQL as
`if({pool} >= {min_peer_n}, …, NULL)` over every percentile
(`analytics/src/domain/metric_results/compiler.rs:606-610`, `:1518`), where `{pool}` counts entities with a
non-null value. Its comment is explicit that this is deliberate placement:
*"Enforced here, server-side, so every consumer inherits it."*

So the strongest R10 claim is an **API** claim, not a UI one: a cohort below the
threshold answers with `n` present and every percentile `NULL`, never a
fabricated median. Design it.

**The finding to record is the three-way disagreement on the number.**
SCENARIOS.md §5 rule 10 says four; the frontend says four; the backend says
five. A cohort of exactly four measured members gets client-side statistics and
`NULL` API statistics — the same person sees a median on screen that the API
declines to compute. Which number is the product rule is a decision, and
Appendix C already lists the threshold as needing confirmation.

**On the seed:** the pool is *measured members of the cohort for that metric*,
not headcount, so a metric only some of a team records can fall below five with
no seed change at all. Check before concluding the negative case is unreachable.

The eight-per-side threshold belongs to comparative *conclusions* — S-2, no
surface.

## R11 · A metric with a known defect says so on the metric itself

> and where a conclusion would rest on it, the metric is excluded by name

**Verdict: Built — the first half. The exclusion half is unbuilt.**

`MetricDefinitionView` (`tests/stand/api/schemas/analytics.py`) carries exactly
the fields this rule asks for:

| Field | Values | What it states |
|---|---|---|
| `schema_status` | `ok` · `error` · `unchecked` | whether the definition resolves against the warehouse |
| `schema_error_code` | `table_not_found` · `column_not_found` · `dimension_not_covered` · `unknown` | *which* defect |
| `is_enabled` | bool | whether the operator disabled it |
| `last_observed_date` | date or absent | freshness — "absent when no observation has ever been seen", orthogonal to `schema_status` |

So "says so on the metric itself" is directly assertable at
`GET /v1/metric-definitions`, and no test names it today. Claims available:
every definition carries a `schema_status`; a definition in `error` carries a
`schema_error_code` rather than a bare flag; `last_observed_date` is **absent**
rather than a placeholder date when nothing was ever observed (which is R1
again, on the catalogue).

The **exclusion** half — "where a conclusion would rest on it, the metric is
excluded by name" — is about conclusions, which are S-2, `No surface`.

Note the caveats that live only in migration comments
(`m20260601_000002_seed_claude_team_metrics_catalog.rs`,
`m20260606_000001_dept_metric_distributions.rs`) reach no surface at all. That
is a narrower gap than "the rule is unimplemented", and worth stating as such.

## R12 · A group figure is computed for the group in view

> never inherited from a wider scope. This is why the same cohort produces
> different numbers at team level and at organization level, and why that is
> correct rather than a discrepancy.

**Verdict: No surface for the team-vs-organization contrast. Do not design the
obvious claim — it cannot be requested.**

The tempting oracle is "ask for the same metric and cohort at team scope and at
organization scope; `n` must differ". Three things block it:

- **There is no scope selector.** A peer request carries exactly one field
  besides `view`: `cohort_key` (`MetricViewRequest2`).
- **There is one cohort key.** All 60 definitions in `registry.yaml` declare
  `peer_cohort_key: org_unit`, and validation refuses any value the definition
  does not declare (`analytics/src/domain/metric_results/validation.rs:330`).
- **Cohort membership is independent of what you asked for.** The cohort CTE
  selects everyone sharing the *target's* `cohort_id`
  (`analytics/src/domain/metric_results/compiler.rs:540-546`), so `n` for a given person and metric is the same
  number however many entities the request names.

A test written to that oracle would fail for a reason unrelated to R12. What
*is* assertable is narrower: that `n` reflects the cohort rather than the
request — ask for one person and for five, and `n` must **not** change. That is
worth having, because "the group figure is computed for the group in view" is
exactly the property a naive implementation would get wrong by counting the
requested entities.

---

## Summary

| Rule | Verdict | Layer for new claims |
|---|---|---|
| R1 evidence gaps shown | Built | API (the `null`-not-`0` half) + UI (covered) |
| R2 confidence travels | No surface | — |
| R3 lineage before attribution | No surface | — |
| R4 no default ranking | Partial | UI |
| R5 access by construction | Built | **API** |
| R6 cost movement preserved | Partial — `ai.cost` exists; the movement half does not | API — `GET /v1/metric-definitions` |
| R7 observes, does not act | Not enforceable here | — |
| R8 clean room | No surface | — |
| R9 role/activity separate axes | Partial | API (temporal half) |
| R10 group-size thresholds | Built server-side (`MIN_PEER_N`), and the two enforcement points disagree on the number | **API** |
| R11 defect stated on the metric | Built (the exclusion half is not) | API — `GET /v1/metric-definitions` |
| R12 group figure computed in view | No surface for the team-vs-org contrast; a narrower claim survives | API — `n` must not track the request |
