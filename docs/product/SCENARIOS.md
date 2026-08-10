# Constructor Insight — Main Scenarios by Persona

**Status:** draft for review · Companion to [VISION.md](VISION.md)

The vision says what Insight is and what it can do. This document says **who is standing in front of
it, what they are there to do, and how far each of them may reach**. It adds no capability and changes
no commitment.

- **Personas** — the four target user groups of VISION §6.1.
- **Scenarios** — ten, in three tiers, ordered by what the product is for.
- **Underneath** — the detailed user scenarios (Appendix A): thirty concrete questions, one person
  asking one thing at one moment, each traced to the scenario it belongs to.

## The three tiers

| Tier | What it is | Scenarios |
|---|---|---|
| **Main** | Review metrics, analyse them, reach conclusions — and, looking forward, estimate work not yet started. This is what the product is for. | S-1, S-2, S-3 |
| **Secondary** | Build new views, explore, take the output elsewhere, compare with the outside world. | S-4, S-5, S-6 |
| **Service** | Set the product up, keep it configured, keep it running. | S-7, S-8, S-9, S-10 |

The order is by importance to the reader, not by build order. Technically the service tier comes
first — sources and identity have to be right before a metric can be — but a customer does not buy
setup, and a scenario list that opens with configuration describes an installation rather than a
product. **Secondary and service do not mean low priority on a roadmap**; they mean a customer does
not arrive for them.

**Why split by persona.** A capability is not one scenario. "Review metrics" is a single tier, but it
means three different things:

- an **individual contributor** sees their own work against a department or cohort median, and no team
  metrics at all;
- a **team manager** sees their team, the people in it by name, and groups recalculated for that team
  rather than carried over from the organization;
- an **executive** sees the whole organization.

The activity is shared. The boundary is not, and the boundary is the part that gets lost in
implementation. That is why it is written down as a statement rather than left implied.

**How to read a scenario block**

- **The scenario** — what holds for everyone, whoever is looking.
- **Per persona** — what each of them does, and what must never happen to them. Only the personas the
  scenario concerns are listed.
- **Not this** — what the scenario deliberately does not do, so it is not promised in a demo.
- **Detail** — the user-scenario IDs from Appendix A that belong here. It lists the detailed
  *questions*, not the source of every persona line: reach comes from VISION §6.1, what an
  administrator may configure from §9. So a persona can appear in a scenario with no question of its
  own beneath it — usually stating a limit rather than claiming a need.

---

## 1. Personas

The four target user groups of VISION §6.1, with the short codes used throughout.

| Code | Group (VISION §6.1) | Arrives asking |
|---|---|---|
| **EXEC** | Executives and portfolio leaders | "Did it get better, or did we just get busier?" |
| **LEAD** | Functional leaders and team managers | "Where exactly is work blocked, and what can I do?" |
| **IC** | Functional teams and individual contributors | "How does my own work look, and what is in my way?" |
| **ADMIN** | Data stewards and administrators | "Which of these numbers can be trusted, and who may see them?" |

Finance and product management are not separate personas here. Their questions are carried by these
four: cost by EXEC, forward-looking planning by EXEC and LEAD. VISION §6.2 lists nine functions —
engineering, product, design, DevOps, QA, support, sales, marketing, finance — and a functional lead
in any of them is a LEAD.

### 1.1 Reach

The boundary each persona carries into every scenario below.

| | EXEC | LEAD | IC | ADMIN |
|---|---|---|---|---|
| **How far they see** | The whole organization | Their own team, and no further | Themselves | Settings, not people's data |
| **How far they zoom** | Organization, function, team, person | Team, sub-team, group, their own reports | Themselves, with the department or cohort as a median | n/a |
| **People by name** | Anyone, where person-level access is granted | The people reporting to them — that is what a team view is for | Themselves | Only while resolving who is who |
| **Comparison** | Between functions, teams and people | Between groups inside their own team, and between their own reports | Against a median, never against a named colleague | n/a |
| **Cost figures** | Where granted | Where granted | No | Where granted |
| **Conclusions and advice** | Reads conclusions | Reads conclusions, receives recommendations | Neither | Neither |
| **Never** | A default view that ranks people against one another · a number with no statement of coverage and confidence | Anything outside their own team · a default view that ranks their reports · group figures carried over from the organization instead of recalculated for the team | Any other person's activity · any team metric beyond the median they are placed against · their own place in a ranking | Administrative rights do **not** carry the right to see data — each kind is granted separately (VISION §9) |

**Naming and ranking are different things, and only one is restricted.** People are named wherever
person-level access has been granted: a manager's team view names their reports, and that is the point
of it. What VISION §3 rules out is a *default* view that ranks named individuals against one another,
or an unexplained productivity score. The line is the default surface and the granted scope — not the
name.

---

# 2. Main — review, analysis, conclusions

The loop the product exists for: measure → diagnose → recommend → validate (VISION §1).

## S-1 · Metrics review · Main

**The scenario.** Someone opens the product to see how things are going. Every number carries a
governed definition, unit, granularity, confidence and stated limitations, and is worked out the same
way for every persona and every scope (VISION §8.4).

**EXEC** — See how the organization as a whole is doing.
- organization, function and team, with change over time
- coverage counted from people who actually have data behind the figure, never by treating missing
  data as zero
- never a default view that ranks people against one another

**LEAD** — See how their team is doing and where work is stuck.
- their own team at any depth, and the people in it by name
- groups recalculated for the team in view, not carried over from the organization
- a group of fewer than four people is not shown at all — that is what keeps an individual
  unidentifiable behind a group figure
- concentration read with the meaning of its domain: in code and documentation a high top-decile
  share is a bus-factor risk, in communication it is a load imbalance. Same arithmetic, different
  conclusion, and the surface says which one it means
- never a team they do not manage

**IC** — See their own work, with a reference point.
- their own activity, flow and AI usage
- the department or cohort as a median to place themselves against
- never another person's activity, never a team metric beyond that median, never their own rank

**ADMIN** — Nothing by default.
- never gains data visibility implicitly from administrative rights

**Not this.** No single "value of AI" number: seat-based and usage-based cost are not summed into one
figure, and unattributed cost stays its own line rather than being spread across the rest
(VISION §6.2.9, §11.4).

**Detail.** B1 (own context), B2 (team over a period), B3 (where work is stuck), B4 (knowledge
concentration), B5 (organization roll-up), B6 (AI cost), B8 (cost of coordination).

## S-2 · Analysis and diagnosis · Main

**The scenario.** The product stops showing shape and starts asserting a relationship — bottlenecks,
risks, anomalies, cost drivers, quality issues, role and activity mismatches — and says which kind of
claim it is making, with confidence and limitations (VISION §8.5). It rests on lineage: work is
followed across the systems it passes through, and **lineage comes before attribution** (VISION §7.3)
— what cannot be traced is shown as an evidence gap, never converted into a confident claim. Where
observed work diverges from the configured role model, the divergence is surfaced here; the role model
it is measured against belongs to S-8.

**LEAD** — Understand why, not just what.
- a conclusion for their team or cohort, with the evidence behind it
- where the chain of evidence breaks in their own area, and what would repair it
- never a verdict about a named individual
- a comparative conclusion needs at least eight people on each side. Below that the surface does not
  render it, and says why — the threshold is higher than the four that protects a group figure,
  because a conclusion claims more than a number does
- never built on a metric known to be defective: it is excluded by name, and the exclusion is stated

**EXEC** — The same at organization level, plus a forecast for proposed work.
- cost and outcome followed to the function, team, product or service — as far as the trail goes
- never attribution stronger than the lineage supports
- never a causal claim where the evidence supports a correlation
- never a forecast presented as a guarantee

**IC** — Not an audience for diagnosis, and never a named example inside one (VISION §6.1).

What limits a conclusion — which sources are missing, which windows do not overlap, which metric is
under a known defect — is an administrator's view, and it lives in S-7.

### Looking forward — the other direction of the same scenario

VISION §1 treats forward-looking work as a direction of its own, not an afterthought to diagnosis:
looking back, the product improves work that has already happened; looking forward, it helps decide
what to commit to. Both are analysis, which is why they sit in one scenario — but the questions differ.

**EXEC, LEAD** — Decide what to take on.
- is this feasible, what will it cost, how long will it take, what are the risks
- the answer built from the organization's own delivery history, not from generic assumptions
- ranked opportunities: where the expected effect is larger for less effort
- the same evidence model, confidence and stated limitations as everything else
- never a forecast presented as a guarantee — it is an extrapolation, and it strengthens as history
  and lineage improve

**Not this.** "AI sped up development by X%" is not a claim Insight makes; what it can say is that a
cohort with high usage differs from one with low usage in stated ways, correlationally — with the word
said on the surface, not in a footnote. Attribution also has a ceiling, and the ceiling is stated
rather than worked around: it reaches person × day × tool, and no further, so "this change was written
by AI" and "this change cost $N" are not claims Insight makes.

**Detail.** C1 (AI gain and price together), C2 (review as a bottleneck), C3 (did quality hold),
C4 (support load vs release), C5 (sales activity vs pipeline), C6 (cost moved rather than fell),
E1 (assess a feature before starting), E2 (where to invest next).

## S-3 · Conclusions: recommendation and validation · Main

**The scenario.** A recommendation is a structured improvement object (VISION §1): observed problem,
affected area, evidence and confidence, recommended action, owner, expected metric movement, and the
follow-up window used to check it. Its origin is declared — evidence-derived from the customer's own
data, or heuristic. Afterwards the product reads the outcome from the measured system.

**LEAD** — Get an action, not an observation.
- one lever they can own, drawn from a fixed set rather than composed freely — three in the first
  version: reduce change size, spread review load, raise AI adoption where it is low at comparable load
- with it: how the lever itself is measured, what should move as a result, which guardrail must not
  slip, and when it is checked — four weeks, computed automatically
- never a recommendation that passes judgement on a named individual rather than on a process, team or
  cohort — a named *owner* is expected, a named *subject* is not
- never a recommendation whose origin is unstated

**EXEC** — Know whether it worked.
- whether the lever moved, whether the outcome moved, and the honest fourth answer: not enough data
- never a result assembled from metrics chosen after the fact — they are fixed when the recommendation
  is issued
- read over a fixed window, four weeks before against four weeks after, rather than by a detector
  hunting for a shift — on a team of ten a detector finds noise
- read against a control: a comparable group that received no recommendation, so a company-wide trend
  is not mistaken for an effect

**ADMIN** — Configure which recommendation families are enabled, who owns them, and how validation
windows are defined (VISION §9).

**IC** — Never the subject of a recommendation.

**Not this.** No surveys, and no self-reporting of any kind as an input — not because opinions do not
matter, but because a validation that depends on people filling in a form does not run. Validation is
read from the measured system. Insight recommends; it does not execute (VISION §13.3).

**Detail.** D1 (a recommendation, not an observation), D2 (did it work, a month later), D3 (context
the system cannot see).

---

# 3. Secondary — new views, exploration, reuse

Everything here extends the main loop. None of it is where a customer starts.

## S-4 · Dashboards, views and exploration · Secondary

**The scenario.** Someone builds a view rather than reading one: composing dashboards from the metric
and recommendation catalog (VISION §8.7, §9), slicing by an attribute, defining a cohort, and following
a figure back to how it was calculated (VISION §2 — customers can see how metrics are calculated).
Exploration moves the question, never the boundary.

Two parts of this scenario go beyond what the vision states today and are proposals rather than
restatements: **saving a composed view and sharing it with someone else**, and the access rule that
follows from sharing. Both are marked in Appendix C.

**Cohorts appear in two roles, and only one of them is here.** In S-1 a cohort is a fixed backdrop —
the median an individual is placed against, computed for them. Here a cohort is something a person
builds: choosing the attribute, the comparison group and the scope. The machinery is shared; the
activity is not.

**LEAD** — Build the view their team actually needs.
- compose from the catalog; slice by attribute; define cohorts and comparison groups
- follow any figure back to how it was calculated
- groups recalculated for whatever is on screen at the time, and still suppressed below four people
- never an exploration path that reaches outside their own team

**EXEC** — Build the portfolio view.
- the same, at organization and function level

**IC** — Explore their own context only.

**ADMIN** — Curate what can be built (VISION §9).
- which metrics and thresholds exist, which cohorts are valid, who may publish a shared view

**Not this.** A view carries the definitions and coverage of the metrics in it, not bare numbers.

**Proposed, and the reason S-4 is worth arguing about.** If a view can be shared, it must not become a
way around access rules: what a viewer sees would have to be re-evaluated for that viewer, so the same
saved dashboard shows each person only what they may see. Neither the vision nor the scenario draft
says this today — which is exactly why it needs deciding before view sharing is built rather than
after.

**Detail.** B7 (compare cohorts), and the slicing side of B2–B5.

## S-5 · Sharing and reuse · Secondary

**The scenario.** A number keeps its meaning when it leaves the product — views, summaries, APIs and
governed data access carry the definition, coverage and confidence with the number (VISION §8.8).

**ADMIN** — Take Insight's output elsewhere.
- API and governed data access, under the same access rules that apply inside the product
- never a number stripped of its definition and confidence, so that outside the product it becomes a
  fact without caveats

**LEAD** — Use a conclusion in their own report or review.

**Detail.** G2 (use conclusions in another system).

## S-6 · External comparison · Secondary

**The scenario.** Comparison against the organization's own history by default; opt-in comparison
against peers and public data where enabled. Every benchmark declares its source, cohort definition,
coverage and confidence (VISION §8.9, §12).

**EXEC** — Know whether a number is bad or normal.
- own history first, which requires sharing nothing
- peer comparison only where the customer has opted in

**ADMIN** — Turn participation on and off; it is revocable (VISION §12).

**Not this.** Raw customer data never leaves the customer boundary. Only anonymized aggregates at
cohort, team or organization level are shared — never individual data, never stack ranking
(VISION §3, §12.2).

**Detail.** F1 (are we slow, or is this normal).

---

# 4. Service — setup, configuration, operation

None of this is why anyone buys the product, and all of it has to work before the rest does.

**These four are also a sequence.** A new customer walks them roughly in order — connect the sources
(S-7), resolve who is who (S-8), configure roles, metrics and access (S-9), with the installation
itself underneath (S-10) — and only then does the main tier hold anything. VISION §14.1 states the
same adoption path: connect, configure, run readiness, start with directional insight, improve
evidence, validate. Listed last by importance; met first in time.

## S-7 · Sources and evidence coverage · Service

**The scenario.** Whatever the wiring state, the product says so plainly: what is connected, what that
unlocks, and — where an answer is not possible — the cause and the smallest set of fixes with the
largest gain in confidence (readiness mode, VISION §7.5).

**ADMIN** — Know what can be proven.
- all eight evidence categories as connected, partly connected or absent — people, work,
  communication, delivery, support, sales, cost, AI — and which metrics each one unlocks
- what each source declares about itself: fields, window, freshness, blind spots, and which level it
  supports — measurement, diagnosis, recommendation or validation (VISION §10.10)
- which links between systems are weak or broken, and what would repair them
- never left to guess why a screen is empty

**LEAD, EXEC** — Meet a gap and know what to do about it.
- the cause named directly: which source is missing, which identities are unresolved, which link is
  broken — plus the minimal fix
- never a comparison across two periods whose source windows do not overlap

**IC** — The same guarantee on their own context.

**Not this.** No zero in place of missing data — a zero looks like a measurement and raises a false
alarm. And no "rough estimate for now": the honest answer to a missing source is what is missing.

**Detail.** A1 (what can be proven), A4 (no answer, but what to connect), C8 (event in the business or
break in the data).

## S-8 · Identity, roles and organization model · Service

**The scenario.** Someone who appears separately in code, tickets, chat and the HR system is
recognised as one person, with a stated confidence. The customer can correct the result, the correction
survives the next sync, and role and team history is preserved so past periods are not recalculated
under a model that was not valid at the time (VISION §8.2, §7.2).

**ADMIN** — Sort out who is who.
- sees what was matched automatically, where the system is unsure, and where it got it wrong
- merges and splits reversibly
- defines roles, several roles per person, and role history
- never loses a manual correction to the next sync

**LEAD, EXEC** — Trust the tree they roll up into (VISION §7.2 — temporal team membership).
- never a subtree quietly reshaped by re-resolution, with past periods recalculated underneath it

**IC** — Be one person, not four.
- never has their work attributed to a duplicate of themselves

**Not this.** Where observed work does not match the configured role model, Insight recommends
changing the configuration — not the person (VISION §9).

**Detail.** A2 (identity queue), A3 (roles and expected activities), C7 (role vs observed work).

## S-9 · Configuration and access · Service

**The scenario.** The customer configures roles, activities, sources, metrics, thresholds, cohorts,
dashboards, localization and access rules (VISION §9). Access to raw, people-level, aggregate, cost and
recommendation data is role-based and policy-controlled, and the boundary holds on every surface.

**ADMIN** — Decide who gets what.
- grants the five kinds of data one by one
- adapts roles, metrics and thresholds without engineering involvement
- sets language, date, number, currency and timezone rules
- never holds all five kinds implicitly; a refusal is enforced by the system, not only hidden on screen

**Where a setting cannot yet be honoured, the gap is shown rather than implied.** Timezone is the live
example: dates are bucketed in UTC while a period is chosen in the viewer's own zone, so an event near
local midnight can land a day either side. Until sources actually carry people's timezones, the
divergence is labelled on the surface instead of being quietly absorbed — the same rule as evidence
gaps in §5.

**LEAD** — Their own team, in full.
- every depth inside it
- never one level up, and never sideways — the limit is structural, not a filter on a screen

**EXEC** — The organization as a whole.
- aggregates, and people where person-level access is granted
- the underlying records only where granted — raw access is one of the five grants, not a privilege of rank

**IC** — Themselves.
- named to their own management chain and to anyone else granted person-level access for their part of
  the organization — and to nobody outside it

**Not this.** Insight is read-only towards connected systems: it writes its own configuration and
annotations, nothing else (VISION §13.3).

**Detail.** G1 (who sees what), G3 (language and timezone), and the configuration half of A3.

## S-10 · Deployment, upgrade and migration · Service

**The scenario.** The product is installed, updated, upgraded and — where it replaces something —
migrated into, without losing what already worked. Deployment models differ (Constructor-hosted,
customer cloud, private cloud, customer-operated), and in all of them customer data stays under
customer control (VISION §1, §14.1, §15.2).

**ADMIN** — Run it.
- install, configure, update and upgrade a customer-operated deployment
- inventory what exists first, then keep / rename / replace / retire
- import history where retention allows, and check parity over an agreed period
- never lose a surface that worked before an upgrade — what was live before it is checked after it

**EXEC** — Replace the previous system with confidence.
- parity stated openly, including what could not be reproduced
- never a parity claim that quietly omits it

**LEAD, IC** — Keep their history across the change (VISION §7.2 — past periods stay under the model valid at the time).
- never a past period silently recalculated under a new model

**Not this.** Insight does not require Constructor to have default access to customer data in order to
operate (VISION §1).

**Detail.** G4 (migrate off a legacy system).

---

## 5. Rules that hold in every scenario

From the vision, stated once here instead of repeated in each scenario.

1. **Evidence gaps are shown, not hidden** (§3, §7.5) — never a zero for missing data, never an
   approximate estimate in place of an answer.
2. **Confidence and limitations travel with every conclusion** (§3) — a strong finding, a directional
   signal and an instrumentation problem stay distinguishable.
3. **Lineage before attribution** (§7.3) — untraceable work is a gap, not a quiet claim.
4. **No default ranking of named individuals, and no unexplained productivity scores** (§3) — people
   are named where person-level access has been granted; the ranking is what is ruled out.
5. **People-level access is role-based and policy-controlled** (§3, §9) — five kinds of data, granted
   separately. The scope boundary holds by construction, not as a filter a screen applies: a viewer is
   handed their own part of the organization and cannot ask for more.
6. **Cost movement is preserved, not folded away** (§11.4) — a local saving that shifts cost, risk or
   effort downstream is shown as a shift; seat-based and usage-based cost are never one figure.
7. **Insight observes and advises; people act** (§13.3) — it writes its own configuration and
   annotations, nothing else.
8. **Clean room** (§12.2) — raw data stays inside the customer boundary; only anonymized aggregates
   are shared, opt-in and revocable.
9. **Role and activity are separate axes** (§7.2) — expected role model and observed activity are
   compared, never conflated; history is kept under the model valid at the time.
10. **Two group-size thresholds, and they are different** — a group figure is not shown below **four**
    people, and a comparative conclusion is not drawn below **eight** on each side. The first keeps an
    individual unidentifiable behind a number; the second keeps a claim from resting on a handful.
11. **A metric with a known defect says so on the metric itself** — and where a conclusion would rest
    on it, the metric is excluded by name rather than quietly included.
12. **A group figure is computed for the group in view** — never inherited from a wider scope. This is
    why the same cohort produces different numbers at team level and at organization level, and why
    that is correct rather than a discrepancy.

---

## Appendix A · The detailed user scenarios

Thirty concrete questions — one person, one question, one moment — from the product scenario draft of
2026-08-04. They are the level at which features are specified and tested; the scenarios in §2–§4 are
what all of them must obey. Availability is deliberately not recorded here: it changes per release and
per customer, since the connected source set differs.

| ID | The question behind it | Who asks | Scenario |
|---|---|---|---|
| B1 | What is visible about me, and what is in my way? | IC | S-1 |
| B2 | What changed for my team over the period? | LEAD | S-1 |
| B3 | Where is work stalling? | LEAD | S-1 |
| B4 | What falls apart if a specific person drops out? | LEAD, EXEC | S-1 |
| B5 | Where are we improving, and where just getting busier? | EXEC | S-1 |
| B6 | How much does AI cost, who spends it, in what form? | EXEC † | S-1 |
| B8 | How much goes into coordination instead of work? | LEAD | S-1 |
| C1 | Did throughput rise where AI was adopted, and what did it cost? | LEAD, EXEC | S-2 |
| C2 | Is speed limited by writing code or by reviewing it? | LEAD | S-2 |
| C3 | Speed went up — did quality hold? | LEAD † | S-2 |
| C4 | Is this ticket spike caused by what we shipped? | LEAD † | S-2 |
| C5 | Activity rose — did the deals move? | LEAD, EXEC | S-2 |
| C6 | Development got cheaper — did the cost move somewhere else? | EXEC † | S-2 |
| E1 | Is it feasible, what will it cost, how long, what risks? | EXEC, LEAD † | S-2 |
| E2 | Where is the effect larger for less effort? | EXEC † | S-2 |
| D1 | I can see the problem — what do I do? | LEAD | S-3 |
| D2 | Was it applied? Did it help? | LEAD, EXEC | S-3 |
| D3 | We had a reorg that month — how do I say so? | LEAD | S-3 |
| B7 | How does group A differ from group B in the same scope? | LEAD, EXEC | S-4 |
| G2 | How do we pull this into our BI, a report, or a bot? | ADMIN, LEAD | S-5 |
| F1 | Our three-day cycle — is that bad? | EXEC | S-6 |
| A1 | Which questions can I already ask, and which not? | ADMIN | S-7 |
| A4 | Why is this empty, and what would make it not empty? | ADMIN, LEAD | S-7 |
| C8 | Is this an event in the business, or a break in the data? | ADMIN, LEAD, EXEC | S-7 |
| A2 | Which identity links did the system make, where is it unsure, where wrong? | ADMIN | S-8 |
| A3 | How do I tell the product who is supposed to do what here? | ADMIN | S-8, S-9 |
| C7 | Someone is listed in one role and does another — error or reality? | LEAD, ADMIN | S-8 |
| G1 | How do I give a lead their team and nothing more? | ADMIN | S-9 |
| G3 | Can we work in our own language and timezone? | all | S-9 |
| G4 | How do we replace what we have without losing history? | ADMIN, EXEC | S-10 |

All thirty map to a scenario. S-6 is the only scenario with no concrete question behind it yet.

**†** The source draft tags this question with a persona this document does not keep separate —
product management or finance. The row shows the persona carrying it here (§1). Six such rows: B6 and
C6 were finance, C3, C4, E1 and E2 were product management.

---

## Appendix B · Scenarios against VISION §8

The vision lists nine product capabilities. The scenarios above are organised by what a person is
doing rather than by capability, so the mapping is not one-to-one: configuration splits in two, and
deployment comes from elsewhere in the vision.

| VISION §8 capability | Scenario |
|---|---|
| 8.1 Source connection and evidence coverage | S-7 |
| 8.2 Identity, role and organization model | S-8 |
| 8.3 Work, outcome and cost lineage | No scenario of its own — nobody opens lineage. It is the rule S-2 and S-3 rest on (§5, rule 3), and repairing it is ADMIN work in S-7 |
| 8.4 Measurement and metric definitions | S-1 |
| 8.5 Analysis, diagnosis and forecasting | S-2 |
| 8.6 Recommendation and validation | S-3 |
| 8.7 Customer configuration | S-9 governs it — which metrics, cohorts and views may exist, and who may publish them; S-4 is where people exercise it |
| 8.8 Exposure and consumption | S-5 |
| 8.9 Benchmarks and shared intelligence | S-6 |
| §1, §14.1, §15.2 — deployment models, adoption, migration | S-10 |

---

## Appendix C · Open points

Four claims in this document go beyond what VISION.md and the scenario draft state. They are written
as requirements because the alternative is to leave them undecided, but each needs a decision rather
than a nod.

- **Administrative rights and data visibility.** §1.1 asserts that ADMIN gains no data visibility
  implicitly. Worth confirming against the access model rather than the interface.
- **Saved and shared views** (S-4). Composing a view is in the vision; *saving* it and *sharing* it are
  not. If sharing exists, the access rule has to come with it — what each viewer sees, re-evaluated for
  them — and that is far cheaper to decide before the feature than after.
- **An identity correction surviving the next sync** (S-8). The draft says merges and splits are
  reversible; neither document says a correction is not undone by re-resolution. Stated here as a
  requirement, because a correction that does not survive is not a correction.
- **The two group-size thresholds** (§5, rule 10) are four for a group figure and eight per side for a
  comparative conclusion. Both come from the scenario draft rather than from the vision, and they are
  worth confirming as product rules — including whether a customer may configure them.
- **IC has the narrowest surface and the tightest boundary.** Everything an IC sees is their own or a
  median — the combination that is easiest to get wrong quietly.
- **S-6 has no surface yet.** Listed so the capability stays planned for, not to imply it exists.
