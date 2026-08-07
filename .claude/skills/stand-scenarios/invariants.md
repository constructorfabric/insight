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

**Verdict: No surface.** No cost endpoint. §S-1 "Not this" says the same thing
from the product side.

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

**Verdict: Partial, and the interesting part is *where*.**

The four-person threshold is `MIN_COHORT = 4` in
`src/frontend/src/lib/insight/within-team-peer.ts:17` — the **frontend**. Its
comment states the reason: peer stats are not computed by the backend yet, so
the grid synthesises them client-side. No equivalent constant was found under
`src/backend/`.

So the rule that protects an individual from being identifiable behind a group
figure is currently enforced by the screen. R5 says a boundary must hold by
construction; R10's threshold does not. **Record the divergence; do not design a
test that asserts the API enforces it, because it does not.**

The eight-per-side threshold belongs to comparative *conclusions* — S-2, no
surface.

**Seed limit:** every seeded team is five ICs plus a lead, so nothing on this
stand falls below four. The negative case needs a seed change before it can be
written at all.

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

**Verdict: Partial — and testable without asserting a value.**

`PeerValueDto` carries **`n`**, the cohort size, alongside `median`/`p25`/`min`/
`max`. That is the oracle: request the same metric and cohort at team scope and
at organization scope, and `n` must differ. No hand-authored number is needed —
the claim is that the two reads disagree, which is exactly what the rule says
must happen.

The complication is R10's: within-team peer statistics are synthesised
client-side today, so where the figure is *computed* determines which layer can
prove it. Read `withinCohortPeer` and the `peer` view handler before choosing.

---

## Summary

| Rule | Verdict | Layer for new claims |
|---|---|---|
| R1 evidence gaps shown | Built | API (the `null`-not-`0` half) + UI (covered) |
| R2 confidence travels | No surface | — |
| R3 lineage before attribution | No surface | — |
| R4 no default ranking | Partial | UI |
| R5 access by construction | Built | **API** |
| R6 cost movement preserved | No surface | — |
| R7 observes, does not act | Not enforceable here | — |
| R8 clean room | No surface | — |
| R9 role/activity separate axes | Partial | API (temporal half) |
| R10 group-size thresholds | Partial; enforced client-side | UI only; seed blocks the negative case |
| R11 defect stated on the metric | Built (the exclusion half is not) | API — `GET /v1/metric-definitions` |
| R12 group figure computed in view | Partial | API via `n`, pending where it is computed |
