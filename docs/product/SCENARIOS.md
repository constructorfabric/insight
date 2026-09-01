# Constructor Insight — Main Scenarios by Persona

**Status:** draft for review · Companion to [VISION.md](VISION.md)

The vision says what Insight is and what it can do. This document says **who is standing in front of
it, what they are there to do, and how far each of them may reach**. It adds no capability and changes
no commitment.

- **Personas** — the target user groups of VISION §6.1, as five *reach* personas across the two
  visibility modes the product ships, plus the administrator role, which is a role and not a reach.
- **Scenarios** — ten, in three tiers, ordered by what the product is for. Each one is a class of
  user scenarios: the concrete questions people bring, and what each persona does and must never meet
  while answering them.

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
means four different things:

- an **individual contributor** sees their own work against a department or cohort median, and no team
  metrics at all;
- a **team manager** sees their team, the people in it by name, and groups recalculated for that team
  rather than carried over from the organization;
- an **executive** sees the whole organization;
- a **peer** in a flat organization sees everyone, because there is no tree to bound them — the
  customer chose that mode, and the boundary is the organization itself.

The activity is shared. The boundary is not, and the boundary is the part that gets lost in
implementation. That is why it is written down as a statement rather than left implied.

**How to read this document**

- A **persona block** has fixed labels in a fixed order, so the same label answers the same question
  for every persona.
- A **scenario block** is a title, one sentence that holds for everyone, the user scenarios in the
  class with who asks each, a table of what each persona does here and what they must never meet, and
  *Not this* — what the scenario deliberately does not do, so it is not promised in a demo.

---

## 1. Personas

A persona here is a **reach** — how far someone may see — and reach comes from one of two sources,
chosen per installation by the identity service's `visibility_policy`.

| Mode | Where reach comes from | Reach personas |
|---|---|---|
| `org_chart` | Reporting lines. A viewer sees themselves, the people who report to them, and whatever they have been granted on top. | EXEC · LEAD · IC |
| `flat` | The organization. Every viewer sees everyone and the organization roll-up — the mode for a roster that carries no reporting lines. | PEER |

The mode belongs to the installation, not to a person, and the identity service reports it to every
client (`GET /v1/me`) — a leaf IC and a PEER are served the same empty `subordinates`, and the policy
is what tells them apart. Grants keep their meaning under either mode: underlying records, aggregates,
person-level data, cost and recommendations are still granted one by one (S-9).

**ADMIN is orthogonal to both.** Administration is a role, not a reach. An administrator is also an
IC, LEAD, EXEC or PEER underneath, and the role adds no visibility of its own — the visible-set
predicate in the identity service has no role term (ADR-0015).

**Function is the second axis.** Reach says how far a person may see; function says what they are
looking at. VISION §6.2 lists nine — Engineering / R&D, Product Management, Design / UX, DevOps / SRE,
QA, Support, Sales, Marketing, Finance / FinOps — and a function is applied per user scenario, never
per persona: a line reads `LEAD · Sales` or `EXEC · Finance`. Finance and product management are
therefore not personas; their questions are carried by a reach persona in that function.

#### EXEC · an executive or portfolio leader

**Who:** a manager whose subtree is the whole organization — the product tells an executive from a
manager only by how much of the organization reports to them, never by a title
**Mode:** org_chart
**Lands on:** the organization roll-up
**Asks:** "Did it get better, or did we just get busier?"
**Sees:** the whole organization; anyone by name where person-level access is granted
**May compare:** functions, teams and people with one another

#### LEAD · a functional leader or team manager

**Who:** a manager whose subtree is smaller than the organization
**Mode:** org_chart
**Lands on:** their team's roll-up
**Asks:** "Where exactly is work blocked, and what can I do?"
**Sees:** their own team at any depth, the people in it by name, and no further
**May compare:** groups inside their own team, and their own reports with one another

#### IC · an individual contributor in a hierarchy

**Who:** nobody reports to them
**Mode:** org_chart
**Lands on:** their own page
**Asks:** "How does my own work look, and what is in my way?"
**Sees:** themselves, with the department or cohort as a median; no cost figures, and no conclusions
or advice
**May compare:** nothing — they are placed against a department or cohort median, never against a
named colleague

#### PEER · a member of a flat organization

**Who:** any signed-in person when the visibility policy is `flat`
**Mode:** flat
**Lands on:** the organization roll-up, the same landing as a manager
**Asks:** "How are we doing, and where do I stand among my peers?"
**Sees:** the whole organization, everyone by name, themselves included
**May compare:** anyone with anyone; the organization is the only cohort

#### ADMIN · a data steward or administrator

**Who:** holds the admin role, whatever their reach. Two jobs in one persona — the *steward* decides
who is who and who may see what; the *operator* installs, upgrades, migrates and wires sources. At
most customers these are different people; in the product they are one role
**Mode:** both
**Lands on:** Manage
**Asks:** "Which of these numbers can be trusted, and who may see them?"
**Sees:** settings, not people's data — the role adds no visibility of its own
**May compare:** nothing — the role sees settings, not people

---

# 2. Main — review, analysis, conclusions

The loop the product exists for: measure → diagnose → recommend → validate (VISION §1).

## S-1 · Metrics review · Main

Every number carries a governed definition, unit, granularity, confidence and stated limitations, and
is worked out the same way for every persona and every scope (VISION §8.4).

**User scenarios in this class**
- What is visible about me, and what is in my way? — IC
- What changed for my team over the period? — LEAD
- Where is work stalling? — LEAD
- What falls apart if a specific person drops out? — LEAD, EXEC
- Where are we improving, and where just getting busier? — EXEC
- How much does AI cost, who spends it, in what form? — EXEC · Finance
- How much goes into coordination instead of work? — LEAD
- How are we doing, and where do I stand among my peers? — PEER

| Who | Does here | Must never meet |
|---|---|---|
| **EXEC** | organization, function and team, with change over time; coverage counted from people who actually have data, never by treating missing data as zero | a default view that ranks people against one another · a number without its coverage |
| **LEAD** | their own team at any depth, the people in it by name; groups recalculated for the team in view; concentration read with the meaning of its domain — in code and documentation a high top-decile share is a bus-factor risk, in communication a load imbalance, and the surface says which | a team they do not manage · a group figure below four people · a group figure carried over from the organization |
| **IC** | their own activity, flow and AI usage, with the department or cohort as a median | another person's activity · any team metric beyond that median · their own rank |
| **PEER** | the whole organization as one scope, themselves included, every member named; their own page against the organization as the median | a peer comparison below five people |
| **ADMIN** | nothing by default | data visibility from the role alone |

**Not this:** no single "value of AI" number — seat-based and usage-based cost are never summed into
one figure, and unattributed cost stays its own line rather than being spread across the rest
(VISION §6.2.9, §11.4).

## S-2 · Analysis and diagnosis · Main

The product stops showing shape and starts asserting a relationship — bottlenecks, risks, anomalies,
cost drivers, quality issues, role and activity mismatches — and says which kind of claim it is making,
with confidence and limitations (VISION §8.5). It rests on lineage, and lineage comes before
attribution (VISION §7.3): what cannot be traced is shown as an evidence gap, never converted into a
confident claim. Looking forward is the same scenario in the other direction — deciding what to commit
to, from the organization's own delivery history rather than generic assumptions (VISION §1).

**User scenarios in this class**
- Did throughput rise where AI was adopted, and what did it cost? — LEAD, EXEC
- Is speed limited by writing code or by reviewing it? — LEAD
- Speed went up — did quality hold? — LEAD · Product
- Is this ticket spike caused by what we shipped? — LEAD · Support
- Activity rose — did the deals move? — LEAD, EXEC · Sales
- Development got cheaper — did the cost move somewhere else? — EXEC · Finance
- Is it feasible, what will it cost, how long, what risks? — EXEC, LEAD · Product
- Where is the effect larger for less effort? — EXEC · Product

| Who | Does here | Must never meet |
|---|---|---|
| **LEAD** | a conclusion for their team or cohort, with the evidence behind it; where the chain of evidence breaks in their own area, and what would repair it; for proposed work, feasibility, cost, duration and risk from the organization's own history | a verdict about a named individual · a comparative conclusion with fewer than eight people on each side — the surface does not render it, and says why · a conclusion built on a metric known to be defective; it is excluded by name, and the exclusion is stated |
| **EXEC** | the same at organization level; cost and outcome followed to the function, team, product or service as far as the trail goes; ranked opportunities where the expected effect is larger for less effort | attribution stronger than the lineage supports · a causal claim where the evidence supports a correlation · a forecast presented as a guarantee — it is an extrapolation that strengthens as history and lineage improve |
| **PEER** | the same as EXEC at organization level, with one cohort: the comparison is against the organization's own history, because there is no second group to compare with | the same as EXEC |
| **IC** | not an audience for diagnosis | a named example inside one |

What limits a conclusion — which sources are missing, which windows do not overlap, which metric is
under a known defect — is an administrator's view, and it lives in S-7.

**Not this:** "AI sped up development by X%" is not a claim Insight makes; what it can say is that a
cohort with high usage differs from one with low usage in stated ways, correlationally — with the word
said on the surface, not in a footnote. Attribution reaches person × day × tool and no further, so
"this change was written by AI" and "this change cost $N" are not claims Insight makes either.

## S-3 · Conclusions: recommendation and validation · Main

A recommendation is a structured improvement object (VISION §1): observed problem, affected area,
evidence and confidence, recommended action, owner, expected metric movement, and the follow-up window
used to check it. Its origin is declared — evidence-derived from the customer's own data, or heuristic
— and afterwards the product reads the outcome from the measured system.

**User scenarios in this class**
- I can see the problem — what do I do? — LEAD
- Was it applied? Did it help? — LEAD, EXEC
- We had a reorg that month — how do I say so? — LEAD

| Who | Does here | Must never meet |
|---|---|---|
| **LEAD** | one lever they can own, from a fixed set rather than composed freely — three in the first version: reduce change size, spread review load, raise AI adoption where it is low at comparable load — with how the lever is measured, what should move, which guardrail must not slip, and when it is checked: four weeks, computed automatically | a recommendation that passes judgement on a named individual rather than a process, team or cohort — a named *owner* is expected, a named *subject* is not · a recommendation whose origin is unstated |
| **EXEC** | whether the lever moved, whether the outcome moved, and the honest fourth answer: not enough data; read over a fixed window, four weeks before against four weeks after, and against a control — a comparable group that received no recommendation | a result assembled from metrics chosen after the fact — they are fixed when the recommendation is issued · a detector hunting for a shift, which on a team of ten finds noise |
| **PEER** | undecided — a recommendation names an owner, and a flat organization has no lead to own a lever (Appendix C) | — |
| **ADMIN** | which recommendation families are enabled, who owns them, and how validation windows are defined (VISION §9) | — |
| **IC** | — | being the subject of a recommendation |

**Not this:** no surveys, and no self-reporting of any kind as an input — not because opinions do not
matter, but because a validation that depends on people filling in a form does not run. Validation is
read from the measured system. Insight recommends; it does not execute (VISION §13.3).

---

# 3. Secondary — new views, exploration, reuse

Everything here extends the main loop. None of it is where a customer starts.

## S-4 · Dashboards, views and exploration · Secondary

Someone builds a view rather than reading one: composing dashboards from the metric and recommendation
catalog (VISION §8.7, §9), slicing by an attribute, defining a cohort, and following a figure back to
how it was calculated (VISION §2). Exploration moves the question, never the boundary. Here a cohort is
something a person builds — the attribute, the comparison group, the scope — where in S-1 it is the
fixed backdrop a person is placed against; the machinery is shared, the activity is not.

**User scenarios in this class**
- How does group A differ from group B in the same scope? — LEAD, EXEC
- The slicing side of every S-1 question — who asks it there asks it here

| Who | Does here | Must never meet |
|---|---|---|
| **LEAD** | compose from the catalog; slice by attribute; define cohorts and comparison groups; follow any figure back to how it was calculated; groups recalculated for whatever is on screen at the time | an exploration path that reaches outside their own team · a group figure below four people |
| **EXEC** | the same, at organization and function level | — |
| **PEER** | views over the whole organization; the same as EXEC | — |
| **IC** | their own context only | — |
| **ADMIN** | which metrics and thresholds exist, which cohorts are valid, who may publish a shared view (VISION §9) | — |

**Not this:** a view carries the definitions and coverage of the metrics in it, never bare numbers.
Saving a composed view and sharing it are proposals rather than restatements (Appendix C): if a view
can be shared, what a viewer sees has to be re-evaluated for that viewer, so a shared dashboard cannot
become a way around access rules — which is why it needs deciding before view sharing is built.

## S-5 · Sharing and reuse · Secondary

A number keeps its meaning when it leaves the product — views, summaries, APIs and governed data access
carry the definition, coverage and confidence with the number (VISION §8.8).

**User scenarios in this class**
- How do we pull this into our BI, a report, or a bot? — ADMIN, LEAD

| Who | Does here | Must never meet |
|---|---|---|
| **ADMIN** | API and governed data access, under the same access rules that apply inside the product | a number stripped of its definition and confidence, so that outside the product it becomes a fact without caveats |
| **LEAD** | a conclusion in their own report or review | — |

## S-6 · External comparison · Secondary

Comparison against the organization's own history by default; opt-in comparison against peers and
public data where enabled. Every benchmark declares its source, cohort definition, coverage and
confidence (VISION §8.9, §12).

**User scenarios in this class**
- Our three-day cycle — is that bad? — EXEC

| Who | Does here | Must never meet |
|---|---|---|
| **EXEC** | own history first, which requires sharing nothing; peer comparison only where the customer has opted in | — |
| **ADMIN** | participation on and off; it is revocable (VISION §12) | — |

**Not this:** raw customer data never leaves the customer boundary. Only anonymized aggregates at
cohort, team or organization level are shared — never individual data, never stack ranking
(VISION §3, §12.2).

---

# 4. Service — setup, configuration, operation

None of this is why anyone buys the product, and all of it has to work before the rest does.

**These four are also a sequence.** A new customer walks them roughly in order — connect the sources
(S-7), resolve who is who (S-8), configure roles, metrics and access (S-9), with the installation
itself underneath (S-10) — and only then does the main tier hold anything. VISION §14.1 states the
same adoption path: connect, configure, run readiness, start with directional insight, improve
evidence, validate. Listed last by importance; met first in time.

## S-7 · Sources and evidence coverage · Service

Whatever the wiring state, the product says so plainly: what is connected, what that unlocks, and —
where an answer is not possible — the cause and the smallest set of fixes with the largest gain in
confidence (readiness mode, VISION §7.5).

**User scenarios in this class**
- Which questions can I already ask, and which not? — ADMIN
- Why is this empty, and what would make it not empty? — ADMIN, LEAD
- Is this an event in the business, or a break in the data? — ADMIN, LEAD, EXEC

| Who | Does here | Must never meet |
|---|---|---|
| **ADMIN** | all eight evidence categories as connected, partly connected or absent — people, work, communication, delivery, support, sales, cost, AI — and which metrics each unlocks; what each source declares about itself: fields, window, freshness, blind spots, and which level it supports — measurement, diagnosis, recommendation or validation (VISION §10.10); which links between systems are weak or broken, and what would repair them | being left to guess why a screen is empty |
| **LEAD, EXEC** | the cause named directly — which source is missing, which identities are unresolved, which link is broken — plus the minimal fix | a comparison across two periods whose source windows do not overlap |
| **IC** | the same guarantee on their own context | — |
| **PEER** | the same guarantee, on the whole organization at once | — |

**Not this:** no zero in place of missing data — a zero looks like a measurement and raises a false
alarm. And no "rough estimate for now": the honest answer to a missing source is what is missing.

## S-8 · Identity, roles and organization model · Service

Someone who appears separately in code, tickets, chat and the HR system is recognised as one person,
with a stated confidence. The customer can correct the result, the correction survives the next sync,
and role and team history is preserved so past periods are not recalculated under a model that was not
valid at the time (VISION §8.2, §7.2).

**User scenarios in this class**
- Which identity links did the system make, where is it unsure, where wrong? — ADMIN
- How do I tell the product who is supposed to do what here? — ADMIN
- Someone is listed in one role and does another — error or reality? — LEAD, ADMIN

| Who | Does here | Must never meet |
|---|---|---|
| **ADMIN** | what was matched automatically, where the system is unsure, and where it got it wrong; merges and splits, reversibly; roles, several per person, and role history | losing a manual correction to the next sync |
| **LEAD, EXEC** | a tree they can trust to roll up into (VISION §7.2 — temporal team membership) | a subtree quietly reshaped by re-resolution, with past periods recalculated underneath it |
| **IC, PEER** | being one person, not four | their work attributed to a duplicate of themselves |

**Not this:** where observed work does not match the configured role model, Insight recommends
changing the configuration — not the person (VISION §9).

## S-9 · Configuration and access · Service

The customer configures roles, activities, sources, metrics, thresholds, cohorts, dashboards,
localization and access rules (VISION §9). Access to raw, people-level, aggregate, cost and
recommendation data is role-based and policy-controlled, and the boundary holds on every surface. Where
a setting cannot yet be honoured, the gap is shown rather than implied — timezone is the live example:
dates are bucketed in UTC while a period is chosen in the viewer's own zone, so an event near local
midnight can land a day either side, and until sources carry people's timezones the divergence is
labelled on the surface.

**User scenarios in this class**
- How do I give a lead their team and nothing more? — ADMIN
- Can we work in our own language and timezone? — everyone
- The configuration half of "who is supposed to do what here" — ADMIN

| Who | Does here | Must never meet |
|---|---|---|
| **ADMIN** | grants the five kinds of data one by one; adapts roles, metrics and thresholds without engineering involvement; sets language, date, number, currency and timezone rules | holding all five kinds implicitly · a refusal only hidden on screen rather than enforced by the system |
| **LEAD** | their own team, in full, at every depth | one level up, or sideways — the limit is structural, not a filter on a screen |
| **EXEC** | the organization as a whole: aggregates, and people where person-level access is granted; the underlying records only where granted | raw access as a privilege of rank — it is one of the five grants |
| **PEER** | the organization as a whole, everyone by name — person-level sight is what the mode grants | records, cost or recommendations without their own grant |
| **IC** | themselves — named to their own management chain and to anyone else granted person-level access for their part of the organization | being named to anyone outside it |

**Not this:** Insight is read-only towards connected systems: it writes its own configuration and
annotations, nothing else (VISION §13.3).

## S-10 · Deployment, upgrade and migration · Service

The product is installed, updated, upgraded and — where it replaces something — migrated into, without
losing what already worked. Deployment models differ (Constructor-hosted, customer cloud, private
cloud, customer-operated), and in all of them customer data stays under customer control (VISION §1,
§14.1, §15.2).

**User scenarios in this class**
- How do we replace what we have without losing history? — ADMIN, EXEC

| Who | Does here | Must never meet |
|---|---|---|
| **ADMIN** | install, configure, update and upgrade a customer-operated deployment; inventory what exists first, then keep, rename, replace or retire; import history where retention allows, and check parity over an agreed period | losing a surface that worked before an upgrade — what was live before it is checked after it |
| **EXEC** | the previous system replaced with confidence — parity stated openly, including what could not be reproduced | a parity claim that quietly omits what could not be reproduced |
| **LEAD, IC, PEER** | their history kept across the change (VISION §7.2) | a past period silently recalculated under a new model |

**Not this:** Insight does not require Constructor to have default access to customer data in order
to operate (VISION §1).

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
    individual unidentifiable behind a number; the second keeps a claim from resting on a handful. The
    one threshold the product enforces today sits between them: a peer comparison — the median and
    quartiles behind a person's position — is not computed below **five** observed people.
11. **A metric with a known defect says so on the metric itself** — and where a conclusion would rest
    on it, the metric is excluded by name rather than quietly included.
12. **A group figure is computed for the group in view** — never inherited from a wider scope. This is
    why the same cohort produces different numbers at team level and at organization level, and why
    that is correct rather than a discrepancy.

---

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

---

## Appendix C · Open points

Several claims in this document go beyond what VISION.md and the scenario draft state. They are written
as requirements because the alternative is to leave them undecided, but each needs a decision rather
than a nod.

- **Administrative rights and data visibility** — settled. §1 asserts that ADMIN gains no data
  visibility implicitly, and the access model agrees: the identity service's visible-set predicate
  carries no role term (ADR-0015), under either visibility mode.
- **An executive is a manager with a larger subtree.** The product tells EXEC from LEAD only by how
  much of the organization reports to them, so anything this document gives EXEC and withholds from
  LEAD holds by scope, not by rule. An EXEC-only rule cannot be enforced until the product knows a
  title.
- **Ranking in flat mode.** A PEER sees every other member's position band. VISION §3 rules out a
  default view that ranks named individuals; a flat organization has opted into seeing everyone.
  Which of the two this is has to be decided before the mode is sold as a feature — and if the bands
  are a ranking, whether flat mode suppresses them or the customer accepts them.
- **Saved and shared views** (S-4). Composing a view is in the vision; *saving* it and *sharing* it are
  not. If sharing exists, the access rule has to come with it — what each viewer sees, re-evaluated for
  them — and that is far cheaper to decide before the feature than after.
- **An identity correction surviving the next sync** (S-8). The draft says merges and splits are
  reversible; neither document says a correction is not undone by re-resolution. Stated here as a
  requirement, because a correction that does not survive is not a correction.
- **The group-size thresholds** (§5, rule 10). The four for a group figure and the eight per side for a
  comparative conclusion come from the scenario draft rather than from the vision, and are worth
  confirming as product rules — including whether a customer may configure them. The five for a peer
  comparison is the product's own, and is the only one of the three confirmed to be enforced.
- **Recommendation ownership in a flat organization.** S-3 expects a named owner for every lever. A
  flat organization has no lead, so either the owner is chosen some other way, or recommendations are
  not offered under `flat` — and the mode should say which.
- **IC has the narrowest surface and the tightest boundary.** Everything an IC sees is their own or a
  median — the combination that is easiest to get wrong quietly.
- **S-6 has no surface yet.** Listed so the capability stays planned for, not to imply it exists.
