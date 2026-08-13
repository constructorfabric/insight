# Personas → the seeded roster, and the reach matrix as a decision table

SCENARIOS.md §1 names four personas. `src/ingestion/tools/seed` provisions a roster of 27
people. This file is the join, plus the traps in it.

Regenerate the roster view with `python3 -m insight_seed.render_profile`; the
authority is [`src/ingestion/tools/seed/PROFILE.md`](../../../src/ingestion/tools/seed/PROFILE.md) and
`manifest.json`, never this page.

## The mapping

| SCENARIOS.md | Fixture(s) | What makes it that persona |
|---|---|---|
| **EXEC** — "Did it get better, or did we just get busier?" | `ceo` | The only roster entry above a team: no `team`, realm roles `insight-admin` + `insight-lead`. The organization-wide viewer. |
| **LEAD** — "Where exactly is work blocked?" | `dev_lead`, `sales_lead`, `hr_lead`, `support_lead` | `role: lead`, realm `insight-lead`, each owning one team of five ICs. Four of them, which is what makes *sideways* refusal testable. |
| **IC** — "How does my own work look?" | `development_ic`, `sales_ic`, `hr_ic`, `support_ic` | `role: ic`, realm `insight-member` only. The narrowest surface and the tightest boundary. |
| **ADMIN** — "Which numbers can be trusted, and who may see them?" | `admin_operator` | Holds the active `admin` row in `identity.person_roles` — the only thing `require_admin` reads. |
| *(boundary, not a persona)* | `other_tenant_lead` | Exists **only** so cross-tenant refusal has a caller to refuse: no team, no org-chart edge, no activity, so they cannot appear in another persona's subtree or move a metric. |

## Three traps

### 1. EXEC is not ADMIN, in either direction

The `ceo` holds the **realm role** `insight-admin`. No identity endpoint reads
it. `require_admin` consults an active `admin` row in `identity.person_roles`,
and the CEO has none — so the most senior person in the org is **403** on every
admin-gated route.

That is SCENARIOS.md §1.1 running the other way: the doc states *administrative
rights do not carry the right to see data*; the seed also enforces that
*seniority does not carry administrative rights*. **Both directions are already
covered**, so neither is a gap:

| Direction | Proved by |
|---|---|
| admin rights ⇏ data visibility | `test_operator_sees_nobody_in_the_org_chart` |
| seniority ⇏ admin rights | `test_admin_listing_is_403_for_a_realm_admin_without_the_grant` — its docstring: *"Holding `insight-admin` in the realm is NOT administrative authority."* |

Read them the right way round. It is easy to cite the second for the first,
because both are about the word "admin".

Fixture consequence: use `realm_admin_session` for *a senior person's view of
the organisation*, `admin_operator_session` for *administrative authority*.
They are not interchangeable and the names are chosen to stop the confusion.

Do not reason "realm admin ⇒ the CEO" from the roster: `admin_operator` holds
`insight-admin` too, and sorts first. `realm_admin_session` reaches the CEO only
because `resolve_by_realm_role` explicitly skips operator accounts — being
outside the org chart they see nobody, which would make an admin-vs-lead
visibility comparison pass while proving nothing.

### 2. `lead_session` deliberately excludes admins

The CEO holds both realm roles, so without the exclusion `lead_session` and
`realm_admin_session` could resolve to the same person and **every
lead-vs-admin comparison would pass vacuously**. `resolve_by_realm_role(...,
LEAD_ROLE, excluding=ADMIN_ROLE)` is what prevents it.

When a claim contrasts two personas, check they cannot resolve to one person.

### 3. `admin_operator` is outside the org chart

No team, no edge in either direction. It contributes no activity data, sees
nobody in `/v1/subchart`, and cannot perturb a visibility assertion. That
isolation is the reason it is a separate person rather than a grant bolted onto
the CEO — and it is also what makes SCENARIOS.md §S-1 ADMIN ("Nothing by
default") directly assertable.

## §1.1 Reach as a decision table

Seven dimensions × four personas. The **Never** column is where the claims are.

| Dimension | EXEC | LEAD | IC | ADMIN | Stand verdict |
|---|---|---|---|---|---|
| **How far they see** | whole org | own team, no further | themselves | settings, not people's data | **Built** — `/v1/subchart`, `/v1/visible-persons`, `/v1/profiles`, `POST /v1/metric-results` (403 outside the visible set) |
| **How far they zoom** | org → function → team → person | team → sub-team → group → own reports | themselves vs a median | n/a | **Built** — `GET /v1/subchart?depth=` |
| **People by name** | anyone, where granted | their own reports | themselves | only while resolving who is who | **Built** — subchart, team view |
| **Comparison** | between functions, teams, people | between groups in their team | against a median, never a named colleague | n/a | **Built** — a `peer` view with `cohort_key`, `PeerValueDto` carrying `n`/`median`/`p25`/`min`/`max`, and server-side suppression below `MIN_PEER_N`. Detail in [invariants.md](./invariants.md) R10 and R12 |
| **Cost figures** | where granted | where granted | **no** | where granted | **Built** — there is no cost *endpoint*, but `ai.cost` is a currency metric on the ordinary metric routes, so the IC "no" is directly testable |
| **Conclusions and advice** | reads conclusions | reads + receives recommendations | **neither** | neither | **No surface** — S-2/S-3 not built |
| **Never** | a default ranking of people · a number with no coverage/confidence | anything outside their team · a default ranking of their reports · group figures carried over from the org | any other person's activity · any team metric beyond the median · their own rank | admin rights ⇏ data visibility | mixed — the access "nevers" are the strongest claims available |

### Reading the LEAD row

"Never one level up, and never sideways — **the limit is structural, not a
filter on a screen**" (§S-9). Two separate claims, and they fail differently:

- **Sideways** — `dev_lead` must not reach `sales_ic`. Covered
  (`test_two_leads_of_different_teams_see_different_people`,
  `test_subchart_of_someone_out_of_scope_is_404_not_403`).
- **Upward** — `dev_lead` must not reach `ceo`. The seed supports it directly:
  `src/ingestion/tools/seed/insight_seed/profiles.py` gives the CEO `parent_uuid=None` and every lead
  `parent_uuid=CEO_UUID`, with each IC parented to their lead — a genuine
  three-level tree. No test names the upward case. That is a real gap.

"Structural, not a filter" is itself testable: the boundary must hold on the
*API*, not merely be hidden by the SPA. Any claim of this family belongs at the
API layer even where a user would meet it on a screen.

### The cohort note

Group-size suppression is enforced **server-side** (`MIN_PEER_N = 5`) as well as
client-side (`MIN_COHORT = 4`), and the two numbers disagree with each other and
with SCENARIOS.md's four. That is a live finding rather than a footnote, and it
lives once, in [invariants.md](./invariants.md) under **R10** — read it there
before designing any comparison claim.

## Persona availability per scenario

Which personas each scenario names, and whether the stand can put a session
behind them.

| Scenario | Personas named | Sessions available |
|---|---|---|
| S-1 Metrics review | EXEC, LEAD, IC, ADMIN | all four |
| S-2 Analysis and diagnosis | LEAD, EXEC, IC | all — but no surface answers them |
| S-3 Recommendations | LEAD, EXEC, ADMIN, IC | all — but no surface |
| S-4 Dashboards and exploration | LEAD, EXEC, IC, ADMIN | all |
| S-5 Sharing and reuse | ADMIN, LEAD | both |
| S-6 External comparison | EXEC, ADMIN | both — no surface (Appendix C) |
| S-7 Sources and evidence coverage | ADMIN, LEAD, EXEC, IC | all; `ingestion: no` limits what is observable |
| S-8 Identity, roles, org model | ADMIN, LEAD, EXEC, IC | all — the best-served scenario |
| S-9 Configuration and access | ADMIN, LEAD, EXEC, IC | all — the other best-served |
| S-10 Deployment and migration | ADMIN, EXEC, LEAD, IC | out of this suite's scope |
