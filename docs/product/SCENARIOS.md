# Constructor Insight — Main Scenarios by Persona

**Status:** draft for review · Companion to [VISION.md](VISION.md)

The vision says what Insight is and what it can do. This document says **who is standing in front of
it, what they are there to do, and how far each of them may reach**. It adds no capability and changes
no commitment.

- **Personas** — the vision's target user groups, as four *reach* personas across the two visibility
  modes the product ships, plus the administrator role, which is a role and not a reach.
- **Scenarios** — ten, in three tiers, ordered by what the product is for. Each one is a class of
  user scenarios: per persona, the questions they bring, what they can see and do, and their
  boundaries.

---

## 1. Personas

Each persona block has the same labels in the same order, so one label answers one question for
every persona. The block is the one home of *how far they see*; the scenarios say what is done
with that reach.

A persona here is a **reach** — how far someone may see — and reach comes from one of two sources,
chosen per installation.

| Mode | Where reach comes from | Reach personas |
|---|---|---|
| **Reporting lines** (`org_chart`) | A viewer sees themselves, the people who report to them, and whatever they have been granted on top. | EXEC · LEAD · IC |
| **Flat** (`flat`) | Every viewer sees everyone and the organization roll-up — the mode for a roster that carries no reporting lines. | PEER |

The mode is set once per installation, never per person. Each kind of data is still granted
separately under either mode ([rule 5](#6-rules-that-hold-in-every-scenario)).

**ADMIN is orthogonal to both.** Administration is a role, not a reach. An administrator is also an
IC, LEAD, EXEC or PEER underneath, and the role adds no visibility of its own.

**Function is the second axis.** Reach says how far a person may see; function says what they are
looking at. The vision lists nine functions — Engineering / R&D, Product Management, Design / UX,
DevOps / SRE, QA, Support, Sales, Marketing, Finance / FinOps — and a function is applied per user
scenario, never per persona: a question is asked by *a sales lead*, or by *EXEC, from finance*.
Finance and product management are therefore not personas; their questions are carried by a reach
persona in that function.

### EXEC · an executive or portfolio leader

- **Who:** a manager whose subtree is the whole organization — the product tells an executive from a
  manager only by how much of the organization reports to them, never by a title
- **Mode:** reporting lines
- **Lands on:** the organization roll-up
- **Asks:** "Did it get better, or did we just get busier?"
- **Sees:** the whole organization; anyone by name where person-level access is granted
- **May compare:** functions, teams and people with one another

### LEAD · a functional leader or team manager

- **Who:** a manager whose subtree is smaller than the organization
- **Mode:** reporting lines
- **Lands on:** their team's roll-up
- **Asks:** "Where exactly is work blocked, and what can I do?"
- **Sees:** their own team at any depth, and the people in it by name
- **May compare:** groups inside their own team, and their own reports with one another

### IC · an individual contributor in a hierarchy

- **Who:** nobody reports to them
- **Mode:** reporting lines
- **Lands on:** their own page
- **Asks:** "How does my own work look, and what is in my way?"
- **Sees:** themselves, with the department or cohort as a median
- **May compare:** nothing — they are placed against a department or cohort median, never against a
  named colleague

### PEER · a member of a flat organization

- **Who:** any signed-in person when the installation runs flat
- **Mode:** flat
- **Lands on:** the organization roll-up
- **Asks:** "How are we doing, and where do I stand among my peers?"
- **Sees:** the whole organization, everyone by name, themselves included
- **May compare:** anyone with anyone; the organization is the only cohort

### ADMIN · a data steward or administrator

- **Who:** holds the admin role, whatever their reach. Two jobs in one persona — the *steward* decides
  who is who and who may see what; the *operator* installs, upgrades, migrates and wires sources. At
  most customers these are different people; in the product they are one role
- **Mode:** both
- **Lands on:** Manage
- **Asks:** "Which of these numbers can be trusted, and who may see them?"
- **Sees:** settings, not people's data
- **May compare:** nothing

---

## 2. Scenarios

Ten, in three tiers, ordered by what the product is for.

| Tier | What it is | Scenarios |
|---|---|---|
| **Main** | Review metrics, analyse them, reach conclusions — and, looking forward, estimate work not yet started. This is what the product is for. | [S-1](#s-1--metrics-review), [S-2](#s-2--analysis-and-diagnosis), [S-3](#s-3--conclusions-recommendation-and-validation) |
| **Secondary** | Build new views, explore, take the output elsewhere, compare with the outside world. | [S-4](#s-4--dashboards-views-and-exploration), [S-5](#s-5--sharing-and-reuse), [S-6](#s-6--external-comparison) |
| **Service** | Set the product up, keep it configured, keep it running. | [S-7](#s-7--sources-and-evidence-coverage), [S-8](#s-8--identity-roles-and-organization-model), [S-9](#s-9--configuration-and-access), [S-10](#s-10--deployment-upgrade-and-migration) |

The order is by importance to the reader, not by build order: the service tier has to work first, but
a customer does not arrive for it.

**How to read a scenario**

- A **scenario block** is a title, one sentence that holds for everyone, and a table with one row per
  persona: their *user scenarios* here, what they *can see and do*, and their *boundaries* — then
  *Not this*, what the scenario deliberately does not do, and where the current state differs from
  the promise, *Today*.
- A *boundary* is observable: the thing is absent from every response the persona can obtain, or it
  is withheld with a stated reason. A cell says *and says why* where the reason must be shown.
- A `—` reads by column. In *User scenarios*: the persona brings no question of their own here. In
  *Can see and do*: nothing. In *Boundaries*: nothing beyond
  [Configuration and access](#s-9--configuration-and-access) — that boundary holds in every
  scenario. In the grid below: the persona has no row in that scenario.
- In *User scenarios*, *a … lead* in any function is LEAD, and *everyone* is every persona.

**Who appears in which scenario**

| | EXEC | LEAD | IC | PEER | ADMIN |
|---|---|---|---|---|---|
| [S-1](#s-1--metrics-review) Metrics review | ✓ | ✓ | ✓ | ✓ | nothing by default |
| [S-2](#s-2--analysis-and-diagnosis) Analysis and diagnosis | ✓ | ✓ | not an audience | ✓ | see [Sources and evidence coverage](#s-7--sources-and-evidence-coverage) |
| [S-3](#s-3--conclusions-recommendation-and-validation) Conclusions | ✓ | ✓ | never the subject | undecided | ✓ |
| [S-4](#s-4--dashboards-views-and-exploration) Dashboards and exploration | ✓ | ✓ | ✓ | ✓ | ✓ |
| [S-5](#s-5--sharing-and-reuse) Sharing and reuse | ✓ | ✓ | — | ✓ | ✓ |
| [S-6](#s-6--external-comparison) External comparison | ✓ | — | — | ✓ | ✓ |
| [S-7](#s-7--sources-and-evidence-coverage) Sources and evidence coverage | ✓ | ✓ | ✓ | ✓ | ✓ |
| [S-8](#s-8--identity-roles-and-organization-model) Identity and organization model | ✓ | ✓ | ✓ | ✓ | ✓ |
| [S-9](#s-9--configuration-and-access) Configuration and access | ✓ | ✓ | ✓ | ✓ | ✓ |
| [S-10](#s-10--deployment-upgrade-and-migration) Deployment and migration | ✓ | ✓ | ✓ | ✓ | ✓ |

---

## 3. Main — review, analysis, conclusions

The loop the product exists for: measure → diagnose → recommend → validate.

### S-1 · Metrics review

Every number carries a governed definition, unit, granularity, confidence and stated limitations, is
worked out the same way for every persona and scope, and covers the period the viewer chooses — a
week by default.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **EXEC** | Where are we improving, and where just getting busier? · What falls apart if a specific person drops out? — with the person-level grant · How much does AI cost, who spends it, in what form? — from finance | organization, function and team over time · coverage counted from people who have data, never by treating missing data as zero | a default view that ranks people · a number without its coverage |
| **LEAD** | What changed for my team over the period? · Where is work stalling? · What falls apart if a specific person drops out? — with the person-level grant · How much goes into coordination instead of work? | their team at any depth, people by name · group figures recalculated for the team on screen · concentration read with its domain's meaning, and the surface says which | a team they do not manage · a group figure for fewer than four people · a team figure equal to the organization's when the team's own data would give another |
| **IC** | What is visible about me, and what is in my way? | their own activity, flow and AI usage against the department or cohort median — shown only when five or more people have data | another person's activity · any team metric beyond that median · their own rank |
| **PEER** | How are we doing, and where do I stand among my peers? | the whole organization as one scope, everyone named · their own page against the organization median | a peer comparison with fewer than five people who have data · their own rank — whether the band that places them against the median is one is undecided |
| **ADMIN** | — | nothing by default | — |

**Not this:** no single "value of AI" number — seat and usage cost are never summed, and unattributed
cost stays its own line.

**Today:** the four-person floor on a group figure is a promise, not yet enforced; the five-person
floor on a peer comparison is ([rule 10](#6-rules-that-hold-in-every-scenario)).

### S-2 · Analysis and diagnosis

The product stops showing shape and starts asserting a relationship — bottlenecks, risks, anomalies,
cost drivers, quality issues, role and activity mismatches — and says which kind of claim it is making,
with confidence and limitations. Looking forward is the same scenario in the other direction: deciding
what to commit to, from the organization's own delivery history.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **EXEC** | Did throughput rise where AI was adopted, and what did it cost? · Activity rose — did the deals move? · Development got cheaper — did the cost move somewhere else? — from finance · Is it feasible, what will it cost, how long, what risks? — from product · Where is the effect larger for less effort? — from product | a conclusion for the organization, a function or a team, with the evidence behind it · cost and outcome followed to the function, team, product or service as far as the trail goes · ranked opportunities where the effect is larger for less effort | attribution stronger than the lineage supports · a causal claim where the evidence supports a correlation · a forecast presented as a guarantee |
| **LEAD** | Did throughput rise where AI was adopted, and what did it cost? · Is speed limited by writing code or by reviewing it? · Speed went up — did quality hold? — an engineering or product lead · Is this ticket spike caused by what we shipped? — a support or product lead · Activity rose — did the deals move? — a sales lead · Is it feasible, what will it cost, how long, what risks? — a product lead | a conclusion for their team or cohort, with the evidence behind it · where the chain of evidence breaks in their area, and what would repair it · feasibility, cost, duration and risk of proposed work from the organization's own history | a verdict about a named individual · a comparative conclusion with fewer than eight people on each side — withheld, and says why · a conclusion built on a metric known to be defective — excluded by name, and says why |
| **IC** | — | not an audience for diagnosis | a named example inside one |
| **PEER** | Did throughput rise where AI was adopted, and what did it cost? · Is speed limited by writing code or by reviewing it? · Where is the effect larger for less effort? | a conclusion for the whole organization, with the evidence behind it · comparison against the organization's own history, since there is no second group to compare with | records, cost or recommendations without their own grant · a causal claim where the evidence supports a correlation |
| **ADMIN** | — | what limits a conclusion — a missing source, two windows that do not overlap, a metric under a known defect — shown as evidence coverage, not as a conclusion | — |

**Not this:** "AI sped up development by X%" is not a claim Insight makes; what it can say is that a
cohort with high usage differs from one with low usage in stated ways, correlationally — with the word
said on the surface. Attribution reaches person × day × tool and no further, so "this change was
written by AI" and "this change cost $N" are not claims Insight makes either. Work that cannot be
traced is shown as an evidence gap, never converted into a confident claim.

### S-3 · Conclusions: recommendation and validation

A recommendation is one lever from a fixed set, with a named owner, the evidence and confidence behind
it, what should move as a result, which guardrail must not slip, and a validation window — four weeks
by default, counted from the day it is issued — after which the product reads the outcome from the
measured system; its origin is declared, evidence-derived from the customer's own data or heuristic.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **EXEC** | Was it applied? Did it help? | one of four answers: the lever moved and the outcome moved · the lever moved and the outcome did not · the lever did not move · not enough data — when either side has fewer than eight people with data · read the window before against the window after, against a control: a group in the same size band that received no recommendation | a result assembled from metrics chosen after the fact — the lever's measure, the outcome and the guardrail are fixed when the recommendation is issued · a shift reported on any other metric — a detector hunting for one on a team of ten finds noise |
| **LEAD** | I can see the problem — what do I do? · Was it applied? Did it help? · We had a reorg that month — how do I say so? | one lever they can own, with how it is measured, what should move, the guardrail, and the check date · the four answers, for their own team · a note on the window — a reorg, a hiring wave, an outage — carried beside the outcome, never fed into it | a recommendation that judges a named individual rather than a process, team or cohort — an owner is named, a subject is not · a recommendation whose origin is unstated |
| **IC** | — | — | being the subject of a recommendation |
| **PEER** | — | a recommendation names an owner, and a flat organization has no lead — undecided | — |
| **ADMIN** | — | which levers are enabled, who owns each, and the validation window | — |

**Not this:** no surveys, and no self-reporting of any kind as an input — a validation that depends on
people filling in a form does not run. Insight recommends; it does not execute ([rule 7](#6-rules-that-hold-in-every-scenario)).

**Today:** three levers — reduce change size, spread review load, raise AI adoption where it is low at
comparable load.

---

## 4. Secondary — new views, exploration, reuse

Everything here extends the main loop. None of it is where a customer starts.

### S-4 · Dashboards, views and exploration

Someone builds a view rather than reading one: composing dashboards from the metric and recommendation
catalog, slicing by an attribute, defining a cohort, and following a figure back to how it was
calculated. Exploration moves the question, never the boundary.

Boundaries are those of [Configuration and access](#s-9--configuration-and-access); this table lists only what the scenario adds.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **EXEC** | How does group A differ from group B in the same scope? · How does the same figure split by one attribute? | compose from the catalog · slice by attribute · define cohorts and comparison groups across the organization and its functions · follow any figure back to how it was calculated | a follow-back from a figure that reaches underlying records without that grant |
| **LEAD** | How does group A differ from group B in the same scope? · How does the same figure split by one attribute? | compose from the catalog · slice by attribute · define cohorts and comparison groups · follow any figure back to how it was calculated · groups recalculated for what is on screen | an exploration path that reaches outside their own team · a group figure for fewer than four people · a follow-back that reaches underlying records without that grant |
| **IC** | — | their own context only | a follow-back that reaches underlying records without that grant |
| **PEER** | How does group A differ from group B? · How does the same figure split by one attribute? | compose from the catalog · slice by attribute · define cohorts across the whole organization · follow any figure back to how it was calculated | a follow-back that reaches underlying records without that grant |
| **ADMIN** | — | which metrics and thresholds exist, and which cohorts are valid | — |

**Not this:** a saved or shared view — undecided (see [Open points](#7-open-points)).

### S-5 · Sharing and reuse

A number keeps its meaning when it leaves the product — views, summaries, APIs and governed data access
carry its definition, coverage and confidence with it.

Boundaries are those of [Configuration and access](#s-9--configuration-and-access); this table lists only what the scenario adds.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **ADMIN** | How do we pull this into our BI, a report, or a bot? | API and governed data access, under the access rules that apply inside the product | a number stripped of its definition and confidence — outside the product it becomes a fact without caveats |
| **LEAD, EXEC** | How do we pull this into our BI, a report, or a bot? | a conclusion in their own report or review, upward or outward | — |
| **PEER** | How do we pull this into our BI, a report, or a bot? | a conclusion in their own report or review, over the whole organization | — |

**Not this:** none.

### S-6 · External comparison

Comparison against the organization's own history by default; opt-in comparison against peers and
public data where enabled. Every benchmark declares its source, cohort definition, coverage and
confidence.

Boundaries are those of [Configuration and access](#s-9--configuration-and-access); this table lists only what the scenario adds.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **EXEC, PEER** | Our three-day cycle — is that bad? | own history first, which requires sharing nothing · peer comparison only where the customer has opted in | — |
| **ADMIN** | — | participation on and off; it is revocable | — |

**Not this:** raw customer data leaving the customer boundary — only anonymized aggregates at cohort,
team or organization level are shared, never individual data, never stack ranking
([rule 8](#6-rules-that-hold-in-every-scenario)).

**Today:** no surface.

---

## 5. Service — setup, configuration, operation

None of this is why anyone buys the product, and all of it has to work before the rest does. The four
are also a sequence — connect the sources, resolve who is who, configure roles, metrics and access,
with the installation itself underneath — and only then does the main tier hold anything.

### S-7 · Sources and evidence coverage

Whatever the wiring state, the product says so plainly: what is connected, what that unlocks, and —
where an answer is not possible — the cause and the smallest set of fixes with the largest gain in
confidence.

Boundaries are those of [Configuration and access](#s-9--configuration-and-access); this table lists only what the scenario adds.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **ADMIN** | Which questions can I already ask, and which not? · Why is this empty, and what would make it not empty? · Is this an event in the business, or a break in the data? | all eight evidence categories as connected, partly connected or absent — people, work, communication, delivery, support, sales, cost, AI — and which metrics each unlocks · what each source declares about itself: fields, window, freshness, blind spots, and the level it supports · which links between systems are weak or broken, and what would repair them | being left to guess why a screen is empty |
| **LEAD, EXEC** | Why is this empty, and what would make it not empty? · Is this an event in the business, or a break in the data? | the cause named directly — which source is missing, which identities are unresolved, which link is broken — plus the minimal fix | a comparison across two periods whose source windows do not overlap, rendered as a plain number — it is withheld, and the source is named |
| **IC, PEER** | — | an empty section on their own page, or on the organization roll-up, names the missing source and the smallest fix | — |

**Not this:** a zero in place of missing data, or a "rough estimate for now" — the honest answer to a
missing source is what is missing ([rule 1](#6-rules-that-hold-in-every-scenario)).

### S-8 · Identity, roles and organization model

Someone who appears separately in code, tickets, chat and the HR system is recognised as one person,
with a stated confidence. The customer can correct the result, the correction survives the next sync,
and role and team history is preserved so past periods are not recalculated under a model that was
not valid at the time.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **ADMIN** | Which identity links did the system make, where is it unsure, where wrong? · How do I tell the product who is supposed to do what here? · Someone is listed in one role and does another — error or reality? | what was matched automatically, where the system is unsure, and where it got it wrong · merges and splits, reversibly · roles, several per person, and role history | losing a manual correction to the next sync |
| **LEAD, EXEC** | Someone is listed in one role and does another — error or reality? | the configured role beside the observed activity, for anyone in their reach · a tree they can trust to roll up into, with team membership kept per period | a subtree quietly reshaped by re-resolution, with past periods recalculated underneath it |
| **IC, PEER** | — | recognised as one person across code, tickets, chat and the HR system, with a stated confidence | their work attributed to a duplicate of themselves |

**Not this:** where observed work does not match the configured role model, Insight recommends
changing the configuration — not the person.

### S-9 · Configuration and access

The customer configures roles, sources, metrics, thresholds, cohorts, dashboards, localization and
access rules; each of the five kinds of data ([rule 5](#6-rules-that-hold-in-every-scenario)) is granted separately, and the boundary holds on
every surface.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **ADMIN** | How do I give a lead their team and nothing more? | grants the five kinds of data one by one · adapts roles, metrics and thresholds without engineering · sets language, date, number, currency and timezone rules | holding all five kinds implicitly · a refusal only hidden on screen rather than enforced by the system · data visibility from the admin role alone |
| **LEAD** | — | their own team, in full, at every depth | one level up, or sideways — the limit is structural, not a filter on a screen |
| **EXEC** | — | aggregates for the whole organization, and people where person-level access is granted | raw access as a privilege of rank — it is one of the five grants |
| **PEER** | — | everyone by name — person-level sight is what the mode grants | records, cost or recommendations without their own grant |
| **IC** | — | themselves — named to their own management chain and to anyone else granted person-level access for their part of the organization | being named to anyone outside it · cost figures · conclusions and advice |
| **everyone** | Can we work in our own language and timezone? | the language, date, number, currency and timezone rules the administrator set | — |

**Not this:** writing to a connected system — Insight writes its own configuration and annotations,
nothing else ([rule 7](#6-rules-that-hold-in-every-scenario)).

**Today:** timezone — dates are bucketed in UTC while the period is chosen in the viewer's own zone,
so an event near local midnight can land a day either side; the divergence is labelled on the surface
until sources carry people's timezones.

### S-10 · Deployment, upgrade and migration

The product is installed, updated, upgraded and — where it replaces something — migrated into, without
losing what already worked. Deployment models differ — Constructor-hosted, customer cloud, private
cloud, customer-operated — and in all of them customer data stays under customer control.

| Who | User scenarios | Can see and do | Boundaries |
|---|---|---|---|
| **ADMIN** | How do we replace what we have without losing history? | install, configure, update and upgrade a customer-operated deployment · inventory what exists first, then keep, rename, replace or retire · import history where retention allows, and check parity over an agreed period | losing a surface that worked before an upgrade — what was live before it is checked after it |
| **EXEC** | How do we replace what we have without losing history? | the previous system replaced with confidence — parity stated openly, including what could not be reproduced | a parity claim that quietly omits what could not be reproduced |
| **LEAD, IC, PEER** | — | their history kept across the change | a past period silently recalculated under a new model |

**Not this:** Constructor holding default access to customer data in order to operate.

---

## 6. Rules that hold in every scenario

From the vision, stated once here instead of repeated in each scenario.

1. **Evidence gaps are shown, not hidden** — never a zero for missing data, never an
   approximate estimate in place of an answer.
2. **Confidence and limitations travel with every conclusion** — a strong finding, a directional
   signal and an instrumentation problem stay distinguishable.
3. **Lineage before attribution** — untraceable work is a gap, not a quiet claim.
4. **No default ranking of named individuals, and no unexplained productivity scores** — people
   are named where person-level access has been granted; the ranking is what is ruled out.
5. **People-level access is role-based and policy-controlled** — five kinds of data, granted
   separately: underlying records, aggregates, person-level data, cost, and recommendations. The scope boundary holds by construction, not as a filter a screen applies: a viewer is
   handed their own part of the organization and cannot ask for more.
6. **Cost movement is preserved, not folded away** — a local saving that shifts cost, risk or
   effort downstream is shown as a shift; seat-based and usage-based cost are never one figure.
7. **Insight observes and advises; people act** — it writes its own configuration and
   annotations, nothing else.
8. **Clean room** — raw data stays inside the customer boundary; only anonymized aggregates
   are shared, opt-in and revocable.
9. **Role and activity are separate axes** — expected role model and observed activity are
   compared, never conflated; history is kept under the model valid at the time.
10. **Three group sizes, and they differ** — each counts people who have data in the period in
    view, never headcount. A group figure is not shown below **four**, and a comparative conclusion
    is not drawn below **eight** on each side; both are withheld, and say why. The first keeps an
    individual unidentifiable behind a number; the second keeps a claim from resting on a handful.
    The one threshold the product enforces today sits between them: a peer comparison — the median
    and quartiles behind a person's position — is not computed below **five**.
11. **A metric with a known defect says so on the metric itself** — and where a conclusion would rest
    on it, the metric is excluded by name rather than quietly included.
12. **A group figure is computed for the group in view** — never inherited from a wider scope. This is
    why the same cohort produces different numbers at team level and at organization level, and why
    that is correct rather than a discrepancy.

---

---

---

## 7. Open points

Product decisions this document depends on and nobody has made. Each is a question, why it is due now,
and what it blocks.

- **Ranking in flat mode.** *Question:* is the position band every PEER sees the default ranking the
  vision rules out, or a comparison the customer opted into by choosing the mode? *Why now:* before
  flat mode is sold as a feature. *Blocks:* PEER's boundary in [Metrics review](#s-1--metrics-review); whether flat mode suppresses
  the bands.
- **Recommendation ownership in a flat organization.** *Question:* who owns a lever when there is no
  lead? *Why now:* [Conclusions](#s-3--conclusions-recommendation-and-validation) expects a named owner for every recommendation. *Blocks:*
  PEER's row there; whether recommendations are offered under flat at all.
- **An executive is a manager with a larger subtree.** *Question:* does the product need a title to
  enforce an EXEC-only rule? *Why now:* whatever this document gives EXEC and withholds from LEAD holds
  by scope, not by rule. *Blocks:* any EXEC-only promise.
- **Saved and shared views.** *Question:* if a view can be shared, is what each viewer sees
  re-evaluated for that viewer? *Why now:* far cheaper to decide before the feature than after.
  *Blocks:* sharing in [Dashboards, views and exploration](#s-4--dashboards-views-and-exploration).
- **A manual correction against new evidence.** *Question:* when a later sync brings evidence that
  contradicts a manual merge or split, does the correction still win silently, or is the administrator
  shown the conflict and asked? *Why now:*
  [Identity, roles and organization model](#s-8--identity-roles-and-organization-model) promises
  the correction survives; it does not say what the administrator learns when the sources disagree.
  *Blocks:* ADMIN's row there.
- **The group-size thresholds.** *Question:* may a customer change four (a group figure) and eight per
  side (a comparative conclusion), or are they fixed? *Why now:* the scenarios already use both as
  boundaries, but only five (a peer comparison) is enforced today
  ([rule 10](#6-rules-that-hold-in-every-scenario)). *Blocks:* every cell that names the number.
