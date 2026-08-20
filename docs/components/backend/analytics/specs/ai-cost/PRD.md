# PRD — AI Development Cost

Scoped addition to the [Analytics service](../../DESIGN.md): what AI cost means, which figures
exist, and what each may and may not claim. Realised by [DESIGN.md](DESIGN.md) and
[claude-admin ADR-0003](../ADR/0003-price-card-per-person-token-cost.md).

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Cost Semantics](#51-cost-semantics)
  - [5.2 Per-Token Cost](#52-per-token-cost)
  - [5.3 Seat Economics](#53-seat-economics)
  - [5.4 Honesty Guarantees](#54-honesty-guarantees)
  - [5.5 Aggregation](#55-aggregation)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-1 — A lead finds wasted seats](#uc-1--a-lead-finds-wasted-seats)
  - [UC-2 — A lead explains an unexpected bill](#uc-2--a-lead-explains-an-unexpected-bill)
  - [UC-3 — Finance compares consumption against spend](#uc-3--finance-compares-consumption-against-spend)
  - [UC-4 — An engineer checks their own consumption](#uc-4--an-engineer-checks-their-own-consumption)
  - [UC-5 — A tenant on negotiated rates](#uc-5--a-tenant-on-negotiated-rates)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Open Decisions](#12-open-decisions)
- [13. Risks](#13-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

Make the money spent on AI-assisted development answerable per person and per period —
separately for each billing model, and without ever presenting an estimate as a measurement.

### 1.2 Background / Problem Statement

`#1607` asks for "development cost analytics (AI)". The word *cost* covers three quantities
that are routinely conflated:

| Layer | The question it answers |
|---|---|
| **Usage-priced** | what would this consumption cost at the vendor's published rates, ignoring any subscription |
| **Seat / credit** | what the seat fee already covers, and what was spent above it at API rates |
| **Invoiced** | what the vendor actually billed |

Today one metric exists, `ai.cost`, which is the first layer only. Its own description says
so: consumption priced at vendor rates, including usage a seat already covered, excluding
seat fees — *"not the amount invoiced"*. A reader who assumes it is the bill will be wrong by
an order of magnitude: a person's usage-priced figure can exceed their consumed seat credits
by one to two orders, and the ratio is not constant.

The second layer exists in silver (`class_ai_overage`) but reaches no client, because it has
no metric key. The third does not exist.

### 1.3 Goals (Business Outcomes)

- Every cost figure states which billing model it belongs to; figures from different models
  are never summed.
- A person's real per-token cost is available where the provider bills per token.
- Seat economics — spend above the seat fee, and the ceiling set on it — is readable through
  the metrics API.
- A provider that does not expose cost renders as *not tracked*, never as zero.
- No per-PR cost is emitted as fact.

### 1.4 Glossary

| Term | Meaning |
|---|---|
| **Usage-priced cost** | consumption valued at the vendor's published per-unit rates, regardless of what was billed |
| **Per-token billing** | the vendor charges per unit consumed; consumption and invoice coincide |
| **Per-seat billing** | a fixed fee per person per month, with an included usage allowance |
| **Included allowance** | the consumption a seat fee already covers. The vendor publishes no per-tier figure and no ingested source carries it |
| **Extra usage** | consumption billed above the included allowance, at API rates — `used_credits`. This is the overage |
| **Extra-usage ceiling** | the admin-set cap on extra usage — `credit_limit_cents`. Reaching it blocks the seat |
| **Extra-usage utilisation** | `used_credits / credit_limit_cents` — how close a seat is to being blocked |
| **Price card** | reference data mapping `(tenant, model, token type, tier)` to a per-unit rate |
| **Attribution mode** | `direct` / `derived` / `allocated`, per `#1674` |
| **Honest-NULL** | absence of a measurement rendered as absent, never as `0` |

## 2. Actors

### 2.1 Human Actors

#### Engineering lead

Needs to know what a team costs in AI, and which seats are wasted or spending beyond the
seat fee.
Acts on two levers: revoking under-used seats and containing overage.

#### Finance / procurement

Needs what is actually billed, per provider, per period — and needs to be able to tell a
measured figure from a modelled one before putting it in a budget.

#### Individual contributor

Needs own consumption and how it compares to peers in the same org unit.

### 2.2 System Actors

#### AI connectors

Deliver usage, cost and seat data into bronze. Each provider exposes a different subset;
none exposes all three cost layers.

#### dbt (silver and gold)

Normalises bronze into class contracts, then derives evidence and observation relations.
Owns lineage, build ordering and data tests.

#### Analytics service

Serves `POST /v1/metric-results` and `GET /v1/metric-definitions` from the unified registry.
Owns query compilation and runtime schema validation.

#### Identity Resolution

Maps `email` and `api_key_id` to a person. Without it, per-token cost has no person grain.

#### Portal

Renders the AI & Cost zone; consumes coverage and attribution metadata to decide what it may
present as fact.

## 3. Operational Concept & Environment

Cost reaches a client through five stages: **bronze** → **silver** class contracts → **gold**
evidence and observations → **metric registry** (`registry.yaml`) → **`/v1/metric-results`**.

The fourth stage is the one most often forgotten: a gold model alone does not create a
metric. This is precisely why seat overage, which has had a populated silver class and a gold
view since June, still cannot be read through the API.

### 3.1 Module-Specific Environment Constraints

- **Provider coverage is uneven and permanent in places.** Two providers deliver per-person
  cost today (Cursor, Claude Team). Two deliver cost at org grain only (Claude Admin,
  OpenAI). Four deliver none (Claude Enterprise and ChatGPT by API design; Copilot;
  JetBrains). The first four can improve; the last four cannot without a vendor change.
- **The metric registry supports `entity_type: person` only.** Team and role figures must be
  produced by cohort aggregation, not by new entity types.
- **Arbitrary query windows.** `/v1/metric-results` accepts any period, including historical
  ones, so any pricing rule must reproduce past periods at the rates then in force.
- **Vendors revise cost data retroactively** for roughly 30 days, so a cost row for a given
  day legitimately changes after first ingestion.

## 4. Scope

### 4.1 In Scope

- A metric per billing model: usage-priced, per-token, extra usage; and how close a seat is
  to its extra-usage ceiling.
- `silver.class_ai_cost` for org-grain billing line items, each row classified by billing
  model.
- Per-person per-token cost derived from a tenant-aware, date-versioned price card
  (ADR-0003).
- Seat and extra-usage metrics over the existing `class_ai_overage`.
- Vendor invoices, as the only source of a seat's price and the only independent check on
  every computed figure.
- Per-provider cost coverage exposed as data rather than frontend copy.
- Honest-NULL wherever a provider has no ingested cost source.

### 4.2 Out of Scope

- Non-AI cost: people, infrastructure, support.
- Per-PR cost as fact — see FR-9.
- The impact / ROI conclusion, which consumes these figures (`#1609`).
- Team and role as *first-class metric entities* — see FR-10 for how they are served.
- **Deferred out of this release:** everything reading the Claude API. `#2437` removed the
  `claude-admin` connector and `silver.class_ai_api_usage`, and the `openai` connector with it,
  so FR-3's Claude Admin supplier, the price card and the reconciliation signal have no
  substrate. FR-3 is served by Cursor alone until a per-token source returns; the decomposition
  records what unblocks each shelved entry.

Deferred with an explicit trigger:

| Item | Resurrect when |
|---|---|
| **`$/PR` (allocated, cohort-ratio)** | after `#1674` defines the attribution-mode contract |

## 5. Functional Requirements

### 5.1 Cost Semantics

#### FR-1 — Billing model is carried on the data, not inferred from the connector

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-billing-model-is-carried-on-the-data-not-inferre`

Every cost row carries the billing model it belongs to. Classification is per row: a single
vendor report can mix token charges and subscription charges, so the connector that produced
a row does not determine its model.

#### FR-2 — `ai.cost` keeps its present meaning

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-aicost-keeps-its-present-meaning`

Consumption priced at vendor rates, including what a seat already covered, excluding seat
fees. Not an invoiced amount, and not to be relabelled as one. Verified suppliers: Cursor
(`chargedCents`, every event kind, seat-covered usage included) and Claude Team
(`total_cost`, confirmed against seat credits to be API-rate pricing, not credit
consumption).

#### FR-5 — Models are never summed

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-models-are-never-summed`

Usage-priced, per-token and per-seat figures are distinct metric keys. No key blends two
models, and no response places two models in one aggregate.

### 5.2 Per-Token Cost

#### FR-3 — Real per-token cost is available per person, from named suppliers

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-real-pertoken-cost-is-available-per-person-from`

Where a provider bills per token *and* the data reaches a person, that person's cost is
available. The supplier set is explicit, because it is narrower than "every per-token
provider":

| Supplier | Path to a person | In `ai.token_cost`? |
|---|---|---|
| **Claude Admin** | `messages_usage.api_key_id` → Identity Resolution → person; priced by the card | ✅ the primary supplier |
| **Cursor** | `usage_events.userEmail` → person; `chargedCents` filtered to `isChargeable` — the amount actually billed, as opposed to the unfiltered total that feeds `ai.cost` | ✅ |
| **OpenAI** | `costs` is `project × line_item × day`. There is no person, and no project→person mapping exists | ❌ **by nature, not by omission** — stays org-grain in `class_ai_cost` (FR-8) |
| Claude Team, Claude Enterprise, ChatGPT, Copilot, JetBrains | per-seat or no cost surface | ❌ |

The figure declares attribution mode `derived` (FR-11).

#### FR-13 — Price correctness is monitored, not assumed

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-price-correctness-is-monitored-not-assumed`

Computed cost is reconciled against the vendor's own report for completed days, over
comparable line items only, and divergence beyond tolerance raises a data-quality signal.

### 5.3 Seat Economics

#### FR-4 — Seat economics is readable through the metrics API, as a monthly fact

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-seat-economics-is-readable-through-the-metrics-a`

Extra usage and its ceiling are exposed as metric keys over `class_ai_overage`. **Extra usage
is `used_credits` itself** — the vendor bills it only once a seat has exhausted the usage
included in its fee — not the excess over `credit_limit_cents`, which is an enforced cap on
that spend rather than the included allowance. The two differ by orders of magnitude over the
same month.

Their grain is **person × month** and they are **not pro-rated**: a seat's month is a fact
about that month, not a rate to be sliced. A window covering whole months returns their sum;
a window covering part of a month returns that month's value in full, not a fraction. A seat
with no ceiling yields no utilisation figure rather than a zero; its extra usage is
unaffected, because the money does not read the ceiling.

A seat also costs its fee whether or not anyone uses it. That fee comes from the invoice, and
`ai.seat_underuse` reports it against the seat's observed activity — the only figure in this
work that expresses waste in money. Room left under the extra-usage ceiling is not waste and
is never presented as such.

### 5.4 Honesty Guarantees

#### FR-6 — Cost coverage is a fact from the API, per capability

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-cost-coverage-is-a-fact-from-the-api-per-capabil`

A single provider can be metered on one layer and per-seat on another — FR-1 says as much —
so a single enum per provider cannot describe it. Coverage is reported **per provider per
layer**: for each of `usage_priced`, `token_billed`, `seat`, `invoice`, one of
`available` / `not_exposed_by_vendor` / `not_ingested`. A client must not need a
hand-maintained list of which vendor bills how.

#### FR-7 — Honest-NULL

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-honestnull`

A provider with no ingested cost source yields absence, never `0`. An empty window yields
`null`.

#### FR-8 — Org-grain cost is not silently personalised

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-orggrain-cost-is-not-silently-personalised`

A cost row whose grain is workspace or project is not attributed to a person unless an
explicit mapping exists. Absent one, it stays at its own grain.

#### FR-9 — No `$/PR` as fact

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-no-pr-as-fact`

Per-PR cost may appear only as `allocated` (a person's period cost spread across their PRs,
which spreads all spend over all PRs whether or not AI was involved) or as `cohort-ratio`
(`Σ cohort cost / Σ cohort PRs`), each explicitly labelled. Neither is `direct`.

#### FR-11 — Attribution mode is declared, by the means available

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-attribution-mode-is-declared-by-the-means-availa`

Until `#1674` introduces a machine-readable mode on the metric contract, each cost metric
states its mode in its registry `explanation`, which is served with the definition and
rendered to the reader. When `#1674` lands, the declaration moves to the field it defines.
The requirement is that the mode is legible to whoever reads the number — not that it is
already structured.

#### FR-12 — Claude Code is counted once

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-claude-code-is-counted-once`

Claude Code activity is visible to more than one connector. Its cost enters the figures
through exactly one of them.

### 5.5 Aggregation

#### FR-10 — Team and role are served by aggregating person metrics, not by new entities

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-fr-team-and-role-are-served-by-aggregating-person-m`

The registry supports `entity_type: person` only. Team and role figures are produced by the
existing cohort mechanism: each cost metric declares `peer_cohort_key: org_unit`, and the
**peer view** returns the distribution across the person's org unit — the same path
`ai.cost` already uses. No per-team metric, entity type or storage is introduced.
Role-based cohorts arrive with `#1455` and require no change here.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### NFR-1 — Reproducibility

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-nfr-reproducibility`

The same window queried twice returns the same figure. Pricing a historical period uses the
rates in force on those days.

#### NFR-2 — Auditability

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-nfr-auditability`

Every figure is traceable to the rows and rates that produced it; rate changes leave a
record. Metrics are drilldown-capable through the evidence contract.

#### NFR-3 — Minor units

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-nfr-minor-units`

Money is carried in integer minor units with an explicit ISO currency. No floating-point
accumulation of money.

#### NFR-4 — Extensibility by tag

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-nfr-extensibility-by-tag`

A new provider joins a silver class by adding a staging model with the class tag, without
editing the class.

#### NFR-5 — Failure is visible

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-nfr-failure-is-visible`

A source that cannot be read is distinguishable from a source with nothing to report. An
existing counter-example is recorded in §11.

### 6.2 NFR Exclusions

- **Latency and freshness targets are not set here.** Cost figures inherit the pipeline's
  existing sync cadence; no cost-specific SLA is introduced.
- **High availability and failover** are properties of the analytics service and the
  warehouse, unchanged by this work.
- **Access control** is not extended. Cost metrics use the same authorisation path as every
  other metric; no cost-specific visibility rule is introduced.
- **Retention** of cost rows follows the existing silver and gold retention policy.
- **Scale targets** are not restated: cost volumes are daily and monthly aggregates, orders
  of magnitude below the activity measures already served.

## 7. Public Library Interfaces

### 7.1 Public API Surface

No new endpoint is introduced. The work adds metric keys to two existing endpoints:

#### `POST /v1/metric-results`

Gains `ai.token_cost`, `ai.extra_usage_cost`, `ai.extra_usage_utilisation` as requestable
`metric_key` values, each supporting the `period`, `peer`, `timeseries` and `breakdown`
views on the same terms as `ai.cost`.

#### `GET /v1/metric-definitions`

Gains the corresponding definitions, each carrying its `explanation` (which states the
billing model and attribution mode, per FR-11) and drilldown capability derived from the
evidence contract.

### 7.2 External Integration Contracts

#### Anthropic Admin API

`GET /v1/organizations/cost_report` — org-grain cost by day, workspace and description; no
`api_key_id` grouping. `GET /v1/organizations/usage_report/messages` — token counts per
API key, model, service tier and context window. Consumed under ADR-0003.

#### claude.ai web API (via the customer-deployed proxy)

`GET /api/organizations/{org}/overage_spend_limits` — per-seat extra-usage spend and its
admin-set ceiling in
minor units. `GET /api/claude_code/metrics_aggs/users` — per-person-per-day usage including
`total_cost`.

#### OpenAI platform Admin API

`GET /v1/organization/costs` — cost by project, line item and day. Org grain only; feeds
`class_ai_cost` and never a person metric.

#### Cursor API

Per-event usage including `chargedCents` and the `isChargeable` flag, keyed by user email.

## 8. Use Cases

### UC-1 — A lead finds wasted seats

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-usecase-a-lead-finds-wasted-seats`

A lead opens the AI & Cost zone for their org unit and asks which seats are paid for and
barely used. That question needs the seat's price, so it is answered by `ai.seat_underuse`
once invoices are ingested; the peer view places each person against their org unit. Seats
whose activity is unmeasured are absent rather than shown at zero, so the lead is not told a
seat is idle when it is simply not tracked.

`ai.extra_usage_utilisation` answers a different question on the same screen — how close a
seat is to the ceiling that would block it. A low ratio is not waste: the room under a
ceiling was never purchased.

### UC-2 — A lead explains an unexpected bill

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-usecase-a-lead-explains-an-unexpected-bill`

Extra usage appeared on the invoice. `ai.extra_usage_cost` per person per month shows who spent
beyond the usage included in their seat fee, and how much; `seat_tier` as a dimension shows
whether the tier itself is set correctly for that person.

### UC-3 — Finance compares consumption against spend

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-usecase-finance-compares-consumption-against-spend`

Finance compares `ai.cost` (what the consumption is worth at vendor rates) against
`ai.token_cost` (what per-token billing actually charged) and the seat figures. Because the
three are distinct keys, no view sums them; because each states its billing model, the
comparison is legible rather than misleading.

### UC-4 — An engineer checks their own consumption

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-usecase-an-engineer-checks-their-own-consumption`

An engineer requests their own person figures for the current period and sees where they sit
in their org unit's distribution — the same peer path `ai.cost` already provides.

### UC-5 — A tenant on negotiated rates

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-usecase-a-tenant-on-negotiated-rates`

A tenant holds discounted per-token rates. Rates derived from that tenant's own cost report
are written tenant-scoped, so its `ai.token_cost` reflects its effective rates rather than
list prices, and no other tenant is affected.

## 9. Acceptance Criteria

1. Cost is queryable per person and per period for each billing model, through
   `/v1/metric-results`.
2. Team and role figures are obtainable from the peer view over `org_unit` without any
   per-team metric or entity type.
3. No response sums or averages figures from different billing models.
4. Every per-token cost figure states mode `derived` in its served definition; no cost figure
   claims `direct` unless the vendor reported it at that grain.
5. Providers with no ingested cost source render as not tracked; an empty window returns
   `null`.
6. Coverage is reported per provider **per layer**, not as a single value per provider.
7. Extra usage and its ceiling are queryable per person and month; extra usage is
   `used_credits`, not the excess over the ceiling; a seat with no ceiling returns no
   utilisation row; no seat figure is pro-rated.
8. Claude Code cost appears once.
9. Org-grain cost is not attributed to a person without an explicit mapping; OpenAI cost does
   not appear in `ai.token_cost`.
10. No `$/PR` is emitted as fact; any per-PR figure is labelled `allocated` or
    `cohort-ratio`.
11. Reconciliation runs for completed days over comparable line items and reports divergence.
12. Each new metric ships with an e2e test that seeds bronze, runs the pipeline, calls the
    API, and asserts the returned value — the project's definition of done for a metric.

## 10. Dependencies

| Dependency | Nature |
|---|---|
| `#1561` unified metric system | the registry every metric must enter |
| `#1602` Identity Resolution | `api_key_id` → person, required by FR-3 |
| `#1674` attribution modes | the structured form FR-11 will move to |
| `#1660` attribution levels | the L2 grain and the L7 ceiling |
| `#1455` functional-role cohorts | role-based grouping for FR-10 |
| `#1986` catalog coverage keys | FR-6 — scope split to settle (OD-3) |
| `#1901`, `#1902` | Claude Admin ingestion; code merged, unverified on data |
| `#1517` honest-NULL | FR-7 |

## 11. Assumptions

1. A tenant's effective per-token rate is constant within a `(model, token type, tier)` over
   a validity interval. ADR-0003 states the conditions, the tenant-scoped fallback, and the
   reconciliation step that detects a violation.
2. Money fields from `overage_spend_limits` are already minor units — asserted in the
   connector schema and consistent with the vendor's documented model (`10000` ⇒ $100.00).
3. Identity Resolution resolves `api_key_id` to a person for keys carrying per-developer
   ownership; keys that do not remain unattributed rather than being spread.
4. The set of billing-model classifications is closed enough to enumerate; an unrecognised
   value is left unclassified and unattributed rather than guessed.

## 12. Open Decisions

Recorded rather than assumed. Each states what is blocked and what happens if it is never
answered.

**OD-1 — Seat base price: source.** No current source exposes what a seat costs. Options:
ingest invoices (accurate; new connector; out of scope here), or configure the price per
tenant (cheap; hand-maintained; diverges silently). *Resolution taken for now:
invoices are ingested in this issue (Phase 1), so `ai.seat_cost` is derived from them rather
than configured. Everything that needs no price — extra usage and its utilisation — ships
ahead of that and does not wait.*

**OD-2 — Seat metric aggregation.** *(resolved — recorded for traceability)* The registry
offers `Sum`, `Ratio`, `Median`, `DistinctCount` — no "latest value". Seat figures are
emitted once per person-month and summed. A multi-month window is exact; a partial window
returns the month in full (FR-4). No new computation type is required.

**OD-3 — Scope boundary with `#1986`.** Both target the same release. `#1986` asks for
catalog keys and the coverage flag; this PRD covers the data beneath them, and FR-6 widens
the flag from one enum per provider to one per layer. Either the seat keys and coverage land
here, or this work narrows to the data layer. *Blocks nothing technically; determines who
ships what.*

**OD-4 — Billing-model classification values.** The vendor cost report carries a type per
row. Two values are known to be token-related and attributable; the rest are not, and at
least one is undocumented. The adopted rule attributes only what is understood and leaves the
rest classified but unattributed. *Confirming the real value distribution on production data
would make the rule exhaustive rather than defensive.*

**OD-5 — Seat state filter.** *(resolved — recorded for traceability)* A deactivated person
keeps an overage row and, since the seat status became a carried column rather than a filter,
keeps their activity rows too. The filter must still live inside the overage branch, because a
seat with no usage has no activity row to inherit a state from. `is_enabled` was the candidate; its semantics
remain undocumented and one observation cannot separate "extra usage is disabled" from "the
seat is not assigned", so it is carried as a dimension and never used as a filter. Seat state
gates on `credit_limit_cents IS NOT NULL` instead.

## 13. Risks

| Risk | Consequence | Mitigation |
|---|---|---|
| Price card drifts from a tenant's effective rates | cost is wrong and nobody notices | reconciliation as a data-quality signal (FR-13); tenant-scoped derivation (ADR-0003) |
| A cost figure is read as the invoice | decisions taken on a number 13–198× off | distinct keys per model (FR-5); meaning stated on the metric itself |
| A zero is read as "free" | an unmeasured provider looks costless | honest-NULL (FR-7) and per-layer coverage (FR-6) |
| **An unauthorised source is indistinguishable from an empty one** | seat economics silently absent where a permission is missing | a permission failure must surface as a data-quality signal rather than an empty green sync — tracked as its own defect |
| Vendors revise cost data for ~30 days; the incremental window is 3 days | late revisions never reach silver | pre-existing, affects all incremental models; tracked separately, not solved here |
| Org-grain cost pressed into person grain to fill a gap | an `allocated` figure presented as fact | FR-8, FR-3 supplier table, and the mode declared on every figure |
