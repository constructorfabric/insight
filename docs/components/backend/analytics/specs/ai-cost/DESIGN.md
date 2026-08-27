# Technical Design — AI Development Cost

Scoped addition to the [Analytics service design](../../DESIGN.md). Realises [PRD.md](PRD.md) and
[claude-admin ADR-0003](../ADR/0003-price-card-per-person-token-cost.md).
Everything the parent design already specifies — the evidence contract, the observation
contract, registry seeding, the result runtime — applies unchanged; this document adds only
what is specific to cost.

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
- [4. Additional context](#4-additional-context)
  - [4.1 Team and Role](#41-team-and-role)
  - [4.2 Validation](#42-validation)
  - [4.3 Migration Order](#43-migration-order)
  - [4.4 Out of Scope](#44-out-of-scope)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Cost becomes a first-class family of measures alongside activity, served by the same
registry, the same evidence contract and the same result runtime. Nothing bespoke is built
for cost: it is one more managed source with its own gold pair and its own metric keys.

Two properties make cost different from activity and drive every decision below. First,
**the same word covers three different quantities** — what consumption is worth at list
rates, what a seat fee already absorbed, and what was invoiced — and conflating them produces
figures wrong by two orders of magnitude. So each becomes its own metric key, and none is
ever summed with another. Second, **money must be reproducible**: the same window queried a
year later must return the same number, which forces rates to be stored, dated and
tenant-scoped rather than embedded in code.

### 1.2 Architecture Drivers

Pricing is governed by `cpt-insightspec-adr-claude-admin-price-card` (claude-admin ADR-0003):
rates as dated, tenant-scoped reference data; self-extension for unseen models; reconciliation
scoped to token line items. Its consequences are drivers here — the silver extensions in §3.7.3
exist because that decision requires them.

#### Functional Drivers

| Driver | Source | Consequence for the design |
|---|---|---|
| Per-person cost where the vendor bills per token | FR-3 | A pricing component, because no vendor endpoint reports cost at person grain |
| Billing models never summed | FR-5 | Distinct metric keys, plus `billing_model` carried on each cost row |
| Seat economics readable through the API | FR-4 | A gold pair over the existing `class_ai_overage`, at month grain |
| Coverage stated per layer, not per provider | FR-6 | A four-layer coverage projection derived from the connector registry |
| Absence never rendered as zero | FR-7 | Rows omitted rather than zero-filled where a measurement is missing |
| Price correctness observable | FR-13 | A reconciliation check against the vendor's own report |

#### NFR Allocation

| NFR | Allocated to |
|---|---|
| NFR-1 Reproducibility | `ai_price_card` validity intervals; day-resolved rate lookup in `ai_token_cost_daily` |
| NFR-2 Auditability | `ai_cost_metric_evidence` (drilldown population); `origin` on each price row |
| NFR-3 Minor units | `class_ai_cost.amount_value` + `amount_currency`; `class_ai_overage` already compliant |
| NFR-4 Extensibility by tag | `union_by_tag('silver:class_ai_cost')` |
| NFR-5 Failure is visible | Reconciliation signal; the 403-reads-as-empty defect is caught one layer up, by `assert_ai_overage_covers_active_seats` and `assert_ai_overage_stream_not_silent` |

### 1.3 Architecture Layers

```text
bronze_claude_admin.claude_admin_messages_usage ─┐
bronze_cursor.cursor_usage_events ───────────────┼─→ silver.class_ai_api_usage ─┐
                                                 │      (+model +tier +ctx,      │
                                                 │       split token types)      │
                                          insight.ai_price_card ─────────────────┤
                                            (tenant-aware, date-versioned)       │
                                                                                 ▼
bronze_claude_admin.claude_admin_cost_report ─┬─→ silver.class_ai_cost   insight.ai_token_cost_daily
bronze_openai.costs ──────────────────────────┘     (+billing_model)              │
                                                                                  ▼
bronze_claude_team.claude_team_overage_spend ──→ silver.class_ai_overage ─→ insight.ai_cost_metric_evidence
                                                                                  │
bronze_cursor.cursor_usage_events ──┐                                             ▼
bronze_claude_team.code_metrics ────┴─→ class_ai_dev_usage.cost_cents   insight.ai_cost_metric_observations
                                              └─→ ai_metric_* (existing, untouched)
                                                                                  │
                                              registry.yaml: source + measures + metrics
                                                                                  │
                                                                    POST /v1/metric-results
```

New: `silver.class_ai_cost`, `insight.ai_price_card`, `insight.ai_token_cost_daily`,
`insight.ai_cost_metric_evidence`, `insight.ai_cost_metric_observations`. Extended:
`silver.class_ai_api_usage`, `registry.yaml`. Untouched: everything feeding `ai.cost`.

## 2. Principles & Constraints

### 2.1 Design Principles

#### One measure, one billing model

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-principle-one-billing-model`

No measure blends billing models, and no row is classified by the connector that produced it.
Classification is a property of the row, because a single vendor report legitimately mixes
consumption charges with subscription charges.

#### Rates are data, dated and tenant-scoped

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-principle-rates-are-data`

Prices live in a table with validity intervals and an optional tenant, never in code. This is
what makes a historical window reproducible and a discounted tenant correct. A rate derived
from a tenant's own report is that tenant's, and is never written globally.

#### Absence is expressed, not filled

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-principle-absence-expressed`

Where a measurement does not exist, no row is emitted. A zero is reserved for a real reading
of zero. This distinction is load-bearing for cost specifically: a zero next to real usage
reads as "free".

#### Every number carries its provenance

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-principle-provenance`

Each metric states its billing model and attribution mode in its served definition, and each
measure is backed by an evidence population that explains it.

### 2.2 Constraints

#### Person is the only entity type

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-constraint-person-only`

The registry supports `entity_type: person`. Team and role figures come from the peer view
over `org_unit`; no per-team metric or storage is introduced.

#### One evidence relation per source

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-constraint-one-evidence-per-source`

The parent design binds a single evidence relation to a source, and a ratio's inputs must
share it. Seat and token measures therefore live in one `ai_cost` family rather than two.

#### Seat figures are monthly facts

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-constraint-monthly-facts`

A seat's month is a fact about that month, not a rate to be sliced. Nothing is pro-rated; a
window holding the month's first day returns it in full, and a window inside the month
returns nothing.

#### Silver cost rows require `_version`

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-constraint-version-required`

`ingestion-data-flow` ADR-0001 permits versionless RMT only for models rebuilt from scratch.
Cost is not one of those, and vendors revise it retroactively, so "latest read wins" must be
expressible.

#### Failure surfaces without source freshness

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-constraint-freshness-not-scheduled`

Two platform properties bound how NFR-5 can be met. Data-quality checks read silver and gold
only, never bronze, so a stream returning nothing cannot be observed at its source. And while
every connector declares `freshness` thresholds on its bronze sources — Claude Team's are
tuned per stream — no workflow, chart template or
script runs `dbt source freshness`, so a stalled stream raises nothing on that path either.
Visibility is therefore expressed as two silver invariants. The first: activity in
`class_ai_dev_usage` implies a `class_ai_overage` row for the same billing month, bounded to
the current month because `overage_spend_limits` snapshots the month in progress and is never
backfilled. That one only sees a seat which also produced activity, so a stream whose seats
are idle goes quiet unnoticed — the second closes it by comparing a source against its own
past: a source that reported seats last billing month still reports them in the current one.

Both are `severity: warn`, both name the 403 in their remediation, and neither separates an
unauthorised stream from a source decommissioned mid-month. Scheduling freshness is a
platform-wide gap, tracked separately.

## 3. Technical Architecture

### 3.1 Domain Model

| Entity | Meaning | Grain |
|---|---|---|
| **Priced token usage** | tokens consumed by an API key, valued at the rate in force that day | person × day × model × token type |
| **Billing line item** | one charge the vendor made | day × line item × workspace or project |
| **Seat extra-usage snapshot** | what a seat spent above its included usage in a billing month, and the ceiling set on that spend | seat × month |
| **Rate** | price per unit for a model and token type, valid over an interval, optionally tenant-specific | tenant × model × token type × tier × interval |
| **Cost coverage** | whether a layer of cost is available for a provider | provider × layer |

### 3.2 Component Model

#### Price Card

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-component-price-card`

##### Why this component exists

No vendor endpoint reports cost at person grain. Rates are the bridge from per-key tokens to
money, and they must be storable, datable and tenant-scoped to keep historical windows
reproducible.

##### Responsibility scope

Holds rates keyed
`(insight_tenant_id, provider, model, token_type, context_window, service_tier, valid_from)`
with `valid_to`, `price_per_mtok`, `currency`, `origin`. Resolves a rate for a given day.
Extends itself for unseen models by back-deriving from the vendor's cost report.

##### Responsibility boundaries

Does not compute cost, does not read usage, does not decide what is attributable. Derived
rows are always tenant-scoped; the global row is only ever seeded from published rates.

##### Related components (by ID)

`cpt-insightspec-aicost-component-token-pricer` consumes it;
`cpt-insightspec-aicost-component-reconciliation` validates it.

#### Token Cost Pricer

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-component-token-pricer`

##### Why this component exists

To turn per-key token counts into per-person money as a `derived` figure — one parameterised
JOIN rather than an application loop over token rows.

##### Responsibility scope

`insight.ai_token_cost_daily`: joins `class_ai_api_usage` to the resolved rate, maps
`api_key_id` to a person through Identity Resolution, and emits priced usage at
person × day × model × token type.

##### Responsibility boundaries

Attributes nothing whose key does not resolve to a person. Prices only token line items;
web search, code execution and session charges are not priced. Does not read
`class_ai_cost`.

##### Related components (by ID)

Consumes `cpt-insightspec-aicost-component-price-card`; feeds
`cpt-insightspec-aicost-component-metric-source`.

#### Cost Class

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-component-cost-class`

##### Why this component exists

Vendor billing line items arrive at org grain and have nowhere to live. Without a home they
are either discarded or, worse, pressed into person grain to fill a gap.

##### Responsibility scope

`silver.class_ai_cost`: one row per vendor charge, classified `metered` / `per_seat` /
`unclassified` from `line_item`, assembled by tag from per-connector staging models.

##### Responsibility boundaries

Never joined to a person. Unclassified rows stay visible in the class and are withheld from
every person-level figure.

##### Related components (by ID)

Read by `cpt-insightspec-aicost-component-reconciliation`.

#### AI Cost Metric Source

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-component-metric-source`

##### Why this component exists

A gold model alone does not create a metric. This component is the pair — evidence plus
observations — plus the registry entries that make cost readable through the API with
drilldown.

##### Responsibility scope

`insight.ai_cost_metric_evidence` and `insight.ai_cost_metric_observations`, carrying
`token_cost_usd`, `extra_usage_usd`, `extra_usage_limit_usd`, `seat_cost_usd`,
`daily_extra_usage_usd`; and the `ai_cost` source plus its metric keys in `registry.yaml`.

##### Responsibility boundaries

Owns no pricing and no classification. Emits no row where a measurement is absent.

##### Related components (by ID)

Consumes `cpt-insightspec-aicost-component-token-pricer` and `silver.class_ai_overage`;
described to clients by `cpt-insightspec-aicost-component-coverage`.

#### Reconciliation Check

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-component-reconciliation`

##### Why this component exists

A price card that drifts without a check is worse than no card, because it is silently
wrong. Drift is expected: vendors change rates and release models continuously.

##### Responsibility scope

Compares `Σ tokens × card` against the vendor's own token line items per completed day and
workspace, and raises a data-quality signal beyond tolerance.

##### Responsibility boundaries

Never corrects the card automatically. A human decides whether a divergence is a rate change,
an unpriced model, or an ingestion gap.

##### Related components (by ID)

Reads `cpt-insightspec-aicost-component-price-card` and
`cpt-insightspec-aicost-component-cost-class`.

#### Coverage Reporter

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-component-coverage`

##### Why this component exists

So a client can state what is and is not measured from data rather than from prose, and can
distinguish *the vendor does not expose this* from *we have not ingested it yet*.

##### Responsibility scope

Projects, per provider and per layer (`usage_priced`, `token_billed`, `seat`, `invoice`), one
of `available` / `not_exposed_by_vendor` / `not_ingested`, derived from the connector
registry.

##### Responsibility boundaries

Renders nothing; states availability only.

##### Related components (by ID)

Describes `cpt-insightspec-aicost-component-metric-source`.

### 3.3 API Contracts

No new endpoint. Five metric keys are added to the existing contract:

| `metric_key` | inputs | computation | format | direction | dimensions |
|---|---|---|---|---|---|
| `ai.token_cost` | `token_cost_usd` (value) | `sum` | `currency` | `lower_is_better` | `tool` |
| `ai.extra_usage_cost` | `extra_usage_usd` (value) | `sum` | `currency` | `lower_is_better` | `tool`, `seat_tier` |
| `ai.extra_usage_utilisation` | `extra_usage_usd` (numerator), `extra_usage_limit_usd` (denominator) | `ratio`, scale 100 | `percent` | — | `tool`, `seat_tier` |
| `ai.seat_cost` | `seat_cost_usd` (value) | `sum` | `currency` | `lower_is_better` | `tool`, `seat_tier` |
| `ai.daily_approximate_extra_usage_cost` | `daily_extra_usage_usd` (value) | `sum` | `currency` | `lower_is_better` | `tool`, `seat_tier` |

All carry `entity_type: person` and `peer_cohort_key: org_unit`. `ai.seat_cost` reads the
per-seat amount on an invoice's non-proration `subscriptions` lines, which is the only place
the vendor prices one seat; a billing month with a single priced tier prices every tiered seat
in it, and where several are priced the seat's own tier has to name one of them.
`ai.seat_underuse` carries that price against observed activity and is not defined yet.

`ai.extra_usage_cost` reads `extra_usage_usd`, which is `used_credits` as the vendor reports it —
the spend that begins once a seat exhausts the usage included in its fee. It is **not** the
excess over `extra_usage_limit_usd`: that field is an enforced ceiling on the same spend, and
subtracting it would report a rounding artefact instead of the money. `ai.extra_usage_utilisation`
therefore measures proximity to being blocked, not waste.

`explanation` is part of the contract: each key states its billing model, whether it is an
invoiced amount, and its attribution mode (`derived` for `ai.token_cost`). Until `#1674`
introduces a structured field, this text is the declaration (PRD FR-11).

`GET /v1/metric-definitions` additionally reports coverage per provider per layer (§3.2.6).

### 3.4 Internal Dependencies

| Depends on | For |
|---|---|
| Metrics registry and result runtime (parent design) | source registration, query compilation, peer view |
| Evidence contract and shared dbt macros | `metric_evidence_table`, `metric_observations_table`, monthly partitioning, storage keys |
| Identity Resolution | `api_key_id` → person |
| `silver.class_ai_overage` | seat measures (already populated) |
| `silver.class_ai_api_usage` | token counts, after the §3.6 extension |
| `union_by_tag` | assembling `class_ai_cost` from per-connector staging |

### 3.5 External Dependencies

#### Anthropic Admin API

`cost_report` (org-grain charges, `cost_type` per row) and `usage_report/messages` (tokens by
API key, model, tier, context window). The first is the reconciliation input and a source of
derived rates; the second is the priced quantity.

#### claude.ai web API via the customer-deployed proxy

`overage_spend_limits` — extra-usage spend and its ceiling in minor units, per seat per month.

#### OpenAI platform Admin API

`costs` — project-grain charges. Feeds `class_ai_cost` only; never a person metric.

#### Cursor API

Per-event usage with `chargedCents` and `isChargeable`, keyed by user email.

### 3.6 Interactions & Sequences

#### Pricing a day of token usage

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-seq-price-day`

1. For each `(tenant, day, api_key_id, model, service_tier, context_window, token_type)` row
   in `class_ai_api_usage`, resolve a rate: tenant-specific before global; exact
   `(service_tier, context_window)` → `context_window IS NULL` → both NULL; first row whose
   validity interval contains the day.
2. Multiply tokens by the rate.
3. Resolve `api_key_id` to a person. If it does not resolve, retain the row unattributed
   rather than distributing it.
4. Emit into `ai_token_cost_daily`, then into evidence and observations.
5. A `(model, service_tier, token_type)` with no rate in force triggers self-extension: derive
   `SUM(amount) / SUM(tokens)` from that tenant's recent cost report, write it
   `origin = 'derived'`, tenant-scoped, and re-resolve.

#### Reconciling a completed day

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-seq-reconcile-day`

1. Compute `Σ tokens × card` per `(completed day, workspace)`, excluding
   `service_tier = 'priority'`.
2. Compute `Σ amount` over `class_ai_cost` rows for the same key where
   `line_item` is a token charge.
3. Full-outer-join the two so a day present on only one side surfaces.
4. Raise a data-quality signal beyond tolerance, naming day, workspace and magnitude.

#### Serving a seat metric for an arbitrary window

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-seq-serve-seat-window`

1. Each `(seat, month)` snapshot is emitted once, dated at the first day of its billing
   month, so the date never moves with the sync schedule. A window inside a month therefore
   returns nothing, and the per-day distribution is what such a window reads.
2. A seat that spent nothing extra emits `0`; a seat with no ceiling emits no utilisation
   row, since the ratio has no denominator.
3. `Sum` over the window adds whole months exactly; a window holding a month's first day
   returns that month in full, never a fraction.
4. The billing month is derived from the read, never declared by the vendor: the endpoint
   reports period-to-date and resets at the boundary, so a month's value is as complete as
   its last read before midnight. A fact's month and the vendor's period can therefore
   disagree at a boundary, and nothing detects it: the money is right, the `covers_days` on
   such a reading is not, and a month whose only reading precedes the rollover keeps a
   figure belonging to the month before it.

### 3.7 Database schemas & tables

#### `insight.ai_price_card`

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-dbtable-price-card`

| Column | Type | Notes |
|---|---|---|
| `insight_tenant_id` | `Nullable(String)` | NULL = published/global rate; set = this tenant's effective rate |
| `provider`, `model` | `String` | |
| `token_type` | `String` | `uncached_input`, `cache_read`, `cache_creation_5m`, `cache_creation_1h`, `output` |
| `context_window`, `service_tier` | `Nullable(String)` | NULL = applies to all |
| `price_per_mtok` | `Decimal(18,6)` | minor units per million tokens |
| `currency` | `String` | ISO |
| `valid_from` | `Date` | |
| `valid_to` | `Nullable(Date)` | NULL = still in force |
| `origin` | `String` | `seeded` \| `derived` |

Seeded by migration. Three token vocabularies — the vendor's usage field names, the vendor's
cost-report type names, and this `token_type` — are reconciled in one documented mapping, not
scattered CASE expressions.

#### `silver.class_ai_cost`

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-dbtable-cost-class`

Grain `(tenant, source, day, line_item, workspace_id | project_id)`.

| Column | Type | Notes |
|---|---|---|
| `insight_tenant_id`, `source_id`, `unique_key` | | class base |
| `day` | `Date` | |
| `provider` | `String` | `anthropic`, `openai`, … |
| `line_item` | `String` | the vendor's charge type, verbatim |
| `billing_model` | `String` | `metered` \| `per_seat` \| `unclassified` — derived from `line_item` |
| `workspace_id`, `project_id` | `Nullable(String)` | whichever grain the vendor reports |
| `model`, `service_tier`, `context_window`, `token_type` | `Nullable(String)` | present on token rows |
| `amount_value` | `Decimal(18,4)` | |
| `amount_currency` | `String` | ISO |
| `source`, `data_source`, `collected_at`, `_version` | | class base |

`union_by_tag('silver:class_ai_cost')`, `materialized='incremental'`,
`incremental_strategy='delete+insert'` keyed on `unique_key`,
`ReplacingMergeTree(_version)`, `ORDER BY unique_key`.

**Both feeders must project `_version`.** `to_ai_cost.sql` does not today, so adding it is
part of its re-tagging; `claude_admin__ai_cost` projects it from the outset. This is not a
formality: `union_by_tag` resolves a duplicate `unique_key` by `ORDER BY _version DESC`,
falling back to an arbitrary winner when no version exists, and vendors revise cost
retroactively — "latest read wins" is the only correct resolution.

Feeders: `openai__ai_cost` (the existing `to_ai_cost.sql`, re-tagged
`['openai','silver:class_ai_cost']` with columns aligned — today the tag is missing, so no
class reads it) and `claude_admin__ai_cost` (new, from `claude_admin_cost_report`).

#### `silver.class_ai_api_usage` — extension

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-dbtable-api-usage-extension`

Additive; existing columns and `unique_key` unchanged, so no key recomputation and no
migration of existing rows.

| New column | Type | Why |
|---|---|---|
| `model` | `Nullable(String)` | rate depends on the model; today the value is buried in `unique_key` and unselectable |
| `service_tier` | `Nullable(String)` | priority traffic is priced separately and excluded from reconciliation |
| `context_window` | `Nullable(String)` | reserved for per-tier pricing |
| `uncached_input_tokens` | `UInt64` | explicit rather than recovered by subtraction |
| `cache_creation_5m_tokens` | `UInt64` | multiplier ×1.25 of base input |
| `cache_creation_1h_tokens` | `UInt64` | multiplier ×2 of base input |

`input_tokens`, `cache_read_tokens` and `cache_creation_tokens` keep their present
definitions. The staging model stops summing the two cache-creation fields and maps all six
straight from bronze, where both are already present.

#### `silver.class_ai_overage` — unchanged

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-dbtable-overage-unchanged`

Seat × month, minor units plus currency, vendor extras in a JSON blob, assembled by tag.
It needs gold models and registry entries, not a change to its shape.

Its `overage_cents` column, `max(0, used − limit)`, is **not** what `ai.extra_usage_cost` reads:
the limit is an enforced ceiling on extra usage rather than the allowance included in the
seat fee, so that difference is a rounding artefact and not the money. The measure reads
`used_amount_cents`. The column is left in place — it is the input to
`ai.extra_usage_utilisation` and to the over-ceiling signal — but nothing sums it as cost.

Monthly history lives in this class, not in bronze. The connector's `unique_key` carries no
month, so the promoted bronze table — `ReplacingMergeTree(_airbyte_extracted_at)`
`ORDER BY unique_key` — holds one row per seat, the current snapshot, and the staging
projection extends that key with the month, as the platform rule for a version axis
requires. A month therefore becomes durable the first time the pipeline runs inside it: a
month with no run keeps no row and cannot be reconstructed later, the vendor exposing
period-to-date only.

#### `insight.ai_cost_metric_evidence` and `_observations`

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-dbtable-metric-relations`

Standard evidence contract — `tenant_id`, `source_key`, `entity_type`, `entity_id`,
`metric_date`, `observed_at`, `measure_key`, `record_id`, `record_kind`, `granularity`,
`record_label`, `contribution`, `subject_key`, `dimensions`, `details` — via the shared
macros, which own materialisation, storage keys and monthly partitioning.

| measure | record | granularity |
|---|---|---|
| `token_cost_usd` | one row per `(person, day, model, token_type)` | `source_summary` |
| `extra_usage_usd` | one row per `(seat, month)` snapshot | `source_summary` |
| `extra_usage_limit_usd` | one row per `(seat, month)` snapshot carrying a ceiling | `source_summary` |
| `seat_cost_usd` | one row per `(seat, month)` snapshot the invoice priced | `source_summary` |
| `daily_extra_usage_usd` | one row per `(seat, read day)` step | `source_summary` |

`details` carries what a reader needs without leaving the drilldown: for token cost the
model, token type, token count and rate applied; for seat rows the seat tier, the ceiling,
and whether a ceiling was set.

A separate family from `ai_metric_*` because the grain differs — seat data is a monthly
snapshot, not a daily activity row — and because one evidence relation is bound per source.

Registry entry:

```yaml
  - source:
      key: ai_cost
      kind: managed_observation
      source_ref: ai_cost_metric_observations
      evidence_ref: ai_cost_metric_evidence
      revision_window_days: 31
    measures:
      - key: token_cost_usd
        evidence_granularity: source_summary
      - key: extra_usage_usd
        evidence_granularity: source_summary
      - key: extra_usage_limit_usd
        evidence_granularity: source_summary
      - key: seat_cost_usd
        evidence_granularity: source_summary
      - key: daily_extra_usage_usd
        evidence_granularity: source_summary
    dimensions:
      - tool
      - seat_tier
```

`token_cost_usd` is the deferred half — DECOMPOSITION 2.8 adds it and registers
`ai.token_cost`; §4.4 says why this release does not build it.

Metrics are authored in `registry.yaml`, embedded at compile time by `builtin.rs`
(`include_str!`); there are no Rust metric constants to edit. `passports.md` is generated by
`analytics passports` and marked *do not edit by hand* — adding a metric means regenerating
it. Registry invariants are enforced by `cargo test -p analytics`.

## 4. Additional context

### 4.1 Team and Role

No per-team metric, entity type or storage is introduced. Every cost metric declares
`peer_cohort_key: org_unit`, so the peer view returns the distribution across a person's org
unit — the same path `ai.cost` already serves. Role-based cohorts arrive with `#1455`.

### 4.2 Validation

| Layer | Check |
|---|---|
| Silver | `not_null` on money, currency and `_version`; `accepted_values` on `billing_model`; no `unclassified` row reaching a person-grain model; `/check-dbt-conventions` passes for engine, `order_by`, `unique_key`; activity in `class_ai_dev_usage` implies a `class_ai_overage` row for the same billing month (`assert_ai_overage_covers_active_seats`); a source that reported seats last billing month still reports them this one (`assert_ai_overage_stream_not_silent`) |
| Price card | no overlapping validity intervals for one resolution key; every `(model, token_type)` seen in usage has a rate in force; no `origin = 'derived'` row with `insight_tenant_id IS NULL` |
| Evidence | standard-column probe healthy; `evidence_granularity` declared for every measure; `measure_key` in `schema.yml` `accepted_values` |
| Gold | token cost is zero only when tokens are zero; a seat with no ceiling emits no utilisation row; every read of `class_ai_cost` and `class_ai_overage` keeps `FINAL`, and a retroactive feeder replacement resolves to the latest `_version` |
| Registry | `cargo test -p analytics`; `passports.md` regenerated and committed |
| Metric | one e2e per key: seed bronze → run pipeline → call `/v1/metric-results` → assert value, `null` on an empty window, dedup of a repeated bronze row |
| Cross-model | a response never aggregates two billing models |
| History | a fixture spanning a `valid_from` boundary prices each side at its own rate; a tenant-scoped rate wins over the global one and does not leak to a second tenant |

### 4.3 Migration Order

1. `class_ai_api_usage` extension — additive; no behaviour change on its own.
2. `ai_price_card` — table plus seed; inert until read.
3. `ai_token_cost_daily` → evidence + observations → `ai.token_cost` → `passports.md` → e2e.
4. `class_ai_cost` + both feeders; re-tagging the OpenAI model ends its orphan state.
5. Seat measures into the same evidence/observation pair → `ai.extra_usage_cost`,
   `ai.extra_usage_utilisation` → e2e.
6. Reconciliation check.
7. Coverage per layer — coordinate with `#1986`.

Step 5 depends on neither the price card nor the cost class, so it can ship first if seat
economics is the more urgent gap; the DECOMPOSITION takes that order.

### 4.4 Out of Scope

- Changing `ai.cost` semantics or its suppliers.
- Making `insight.ai_cost_person_period` part of the unified path. Nothing in the registry
  reads it; it is documented as unread and left alone.
- Team or role as a metric entity.
- Per-PR cost, and the vendor's own pull-request attribution counters — `#1660` territory.

- **Deferred out of this release:** `insight.ai_price_card`, `insight.ai_token_cost_daily`,
  `silver.class_ai_cost` and the `class_ai_api_usage` extension. `#2437` removed the
  `claude-admin` connector along with `class_ai_api_usage`, and the `openai` connector whose
  model was the other feeder named for `class_ai_cost`. The sections that specify them are kept
  as written — they are the input to whoever revives the branch — but nothing in this release
  builds them. Cursor's existing cost data remains served through `ai.cost`; a separate
  `ai.token_cost` path is future work.

One adjacent defect is recorded rather than fixed here: the 3-day incremental window against
~30-day vendor revisions.

## 5. Traceability

| PRD requirement | Realised by |
|---|---|
| FR-1 | `cpt-insightspec-aicost-dbtable-cost-class` (`billing_model`) |
| FR-3 | `cpt-insightspec-aicost-component-token-pricer`, `cpt-insightspec-aicost-seq-price-day` |
| FR-4 | `cpt-insightspec-aicost-seq-serve-seat-window`, `cpt-insightspec-aicost-constraint-monthly-facts` |
| FR-5 | `cpt-insightspec-aicost-principle-one-billing-model`, §3.3 |
| FR-6 | `cpt-insightspec-aicost-component-coverage` |
| FR-7 | `cpt-insightspec-aicost-principle-absence-expressed` |
| FR-8 | `cpt-insightspec-aicost-component-cost-class` boundaries |
| FR-10 | `cpt-insightspec-aicost-constraint-person-only`, §4.1 |
| FR-11 | §3.3 `explanation` contract |
| FR-13 | `cpt-insightspec-aicost-component-reconciliation`, `cpt-insightspec-aicost-seq-reconcile-day` |
| NFR-1, NFR-2 | `cpt-insightspec-aicost-dbtable-price-card`, `cpt-insightspec-aicost-dbtable-metric-relations` |
| NFR-3 | `cpt-insightspec-aicost-dbtable-cost-class` |
| NFR-4 | `union_by_tag` assembly, §3.7.2 |
