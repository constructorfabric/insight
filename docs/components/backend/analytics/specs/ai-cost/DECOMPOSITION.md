# Decomposition: AI Development Cost

Decomposes [DESIGN.md](DESIGN.md) into implementable features. Requirements referenced as
`FR-n` / `AC-n` are from [PRD.md](PRD.md); pricing decisions from
[claude-admin ADR-0003](../ADR/0003-price-card-per-person-token-cost.md).

<!-- toc -->

- [1. Overview](#1-overview)
  - [Deferred with the claude-admin removal](#deferred-with-the-claude-admin-removal)
- [2. Entries](#2-entries)
  - [2.1 Claude Team — extraction audit — HIGH](#21-claude-team--extraction-audit--high)
  - [2.2 Claude Team — permission-failure visibility — HIGH](#22-claude-team--permission-failure-visibility--high)
  - [2.3 Claude Team — seat economics to the API — HIGH](#23-claude-team--seat-economics-to-the-api--high)
  - [2.4 Claude Team — confirm and cover the usage-priced metric — MEDIUM](#24-claude-team--confirm-and-cover-the-usage-priced-metric--medium)
  - [2.5 Claude Team — vendor invoices — HIGH](#25-claude-team--vendor-invoices--high)
  - [2.6 Claude Team — seat cost and underuse — MEDIUM](#26-claude-team--seat-cost-and-underuse--medium)
  - [2.7 Claude API — token usage contract — HIGH — DEFERRED](#27-claude-api--token-usage-contract--high--deferred)
  - [2.8 Claude API — price card and per-person token cost — HIGH — DEFERRED](#28-claude-api--price-card-and-per-person-token-cost--high--deferred)
  - [2.9 Claude API — org-grain cost class — MEDIUM — DEFERRED](#29-claude-api--org-grain-cost-class--medium--deferred)
  - [2.10 Claude API — price reconciliation — MEDIUM — DEFERRED](#210-claude-api--price-reconciliation--medium--deferred)
  - [2.11 Cost coverage contract — MEDIUM — DEFERRED](#211-cost-coverage-contract--medium--deferred)
- [3. Feature Dependencies](#3-feature-dependencies)
- [4. Conditional Scope](#4-conditional-scope)

<!-- /toc -->

## 1. Overview

Work is split **by connector**, not by layer: each phase takes one vendor from bronze all the
way to `/v1/metric-results`, so every phase is independently verifiable, testable and
demonstrable on real data.

- **Phase 1 — Claude Team.** Everything this connector can yield, at least matching the
  reference implementation in `data_collector` (`apps/claude`).
- **Phase 2 — per-token cost.** Reduced to Cursor. The Claude API branch (2.7–2.10) is
  deferred: the connector it reads was removed from the repository, and so were the two silver
  relations it fed. See "Deferred with the claude-admin removal" below.

**Two quantities, not one.** The phrase "total_cost / avg_cost_per_day (per-seat overage)"
conflates two figures that arrive from different endpoints and differ by orders of magnitude:

| Figure | Endpoint | Grain | What it is |
|---|---|---|---|
| `total_cost`, `avg_cost_per_day` | `GET /api/claude_code/metrics_aggs/users` | person × day | consumption valued at the vendor's API rates — **not** overage. Already served as `ai.cost` |
| `used_credits`, `monthly_credit_limit` | `GET /api/organizations/{org}/overage_spend_limits` | person × month | money billed above the usage included in the seat fee, and the admin-set ceiling on it |

`total_cost` exceeds the same person's `used_credits` for the same month by one to two orders
of magnitude, and the ratio is not constant — `total_cost` values *all* consumption at API
rates, while `used_credits` counts only what is billed once the seat's included usage is
exhausted. Phase 1 therefore delivers **both**, as separate metric keys that are never summed
(FR-5).

**Where Phase 1 actually stands against the reference implementation.** Bronze is already
wider than `data_collector` on two of four streams — our overage stream carries
`monthly_credit_limit`, `currency`, `out_of_credits`, `used_credits_basis` and `limit_type`,
which the reference does not persist, and our code-metrics stream carries thirteen fields
against its eight. The real gaps are elsewhere:

1. **Seat data reaches no client.** `class_ai_overage` is populated and has no metric key.
2. **Invoices are absent.** The only stream `data_collector` has and we do not — and the only
   source of a seat's base price.
3. **Two facts stall inside our own pipeline**: `last_active` never leaves bronze, and rows
   with a non-active `status` are dropped without anyone knowing what is lost.

**Deferred out of this issue.** `prs_with_cc_count` and `prs_total_count` also reach silver
unread, but they answer whether a pull request involved Claude Code — the subject of `#1660`,
not of cost. No requirement here calls for them, and FR-9 forbids the per-PR cost figure they
would invite. Recorded as a candidate for `#1660`; the vendor populates them only where
Anthropic's GitHub app is connected.

**Decomposition strategy**:

- **2.1 and 2.2 are unblocked** and run in parallel with everything else. 2.1 replaces
  assumptions with a measured field-by-field delta; 2.2 makes the phase's central data source
  fail loudly instead of silently.
- **2.3 is the fastest visible value** — the data is already in silver, so the work is the
  gold pair plus registry entries.
- **2.5 is the most expensive** and the only part needing a CDK connector rather than a
  declarative manifest — a shape five connectors in the repo already use.
- Every feature that ships a metric ships its e2e test in the same change. A metric key
  without one is not shipped (AC-12).

**Late-phase items (future scope)**:

- **Per-PR cost as `allocated` or `cohort-ratio`** — blocked on `#1674` (FR-9).
- **Invoices for vendors other than Claude Team** (ChatGPT, Cursor) — different mechanisms;
  Phase 1 covers Claude Team only.

One defect surfaced by this work is tracked separately and is **not** in scope: the 3-day
incremental window against roughly 30-day vendor revisions.

### Deferred with the claude-admin removal

`chore: remove seven unused connectors` (#2437) deleted the `claude-admin` connector, its dbt
models and its DDL snapshot, and with them `silver.class_ai_api_usage`. The same commit removed
the `openai` connector, whose `to_ai_cost.sql` was the other feeder named for
`silver.class_ai_cost`. Four entries stand on what it deleted and are therefore deferred out of
this release, kept in place rather than dropped because the analysis in them survives the
connector:

| Entry | Why it is blocked | What would unblock it |
|---|---|---|
| 2.7 Claude API — token usage contract | `class_ai_api_usage`, the relation it extends, no longer exists | the connector returns, or another per-token source lands with the same shape |
| 2.8 Claude API — price card and per-person token cost | reads `claude_admin_cost_report`, which has no producer | the connector returns |
| 2.9 Claude API — org-grain cost class | both named feeders were deleted — `claude_admin__ai_cost` was never built and `to_ai_cost.sql` went with `openai` | any vendor billing line item reaches silver |
| 2.10 Claude API — price reconciliation | depends on 2.8 | 2.8 |
| 2.11 Cost coverage contract | depends on a token-billed source reaching a metric | the Cursor branch below, or 2.8 |

[ADR-0003](../ADR/0003-price-card-per-person-token-cost.md) governs 2.8 and is deferred with
it. It stays in the tree: the decision and its reasoning are the input to whoever revives the
branch, and the ADR itself now records what is open in it.

**What Phase 2 still holds.** Cursor is the only per-token source left in the repository, and it
needs no price card at all — `chargedCents WHERE isChargeable` is the billed amount, already in
money (research notes §2). Its entry is deliberately not written here: writing scope for work
nobody has scheduled produced 2.7–2.10, which are now shelved. It will be written when the
branch is picked up.

---

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-status-overall`

### 2.1 Claude Team — extraction audit — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-claude-team-audit`

- **Purpose**: Replace assumption with measurement. Establish the exact field-by-field delta
  between what claude.ai exposes, what `data_collector` persists, what our bronze holds, and
  what reaches the API — so the rest of Phase 1 builds only what is genuinely missing.

- **Depends On**: None.

- **Scope**:
  - Compare our four streams (`claude_team_members`, `claude_team_invites`,
    `claude_team_overage_spend`, `claude_team_code_metrics`) against the four syncs of
    `apps/claude` and against live proxy responses, field by field.
  - Determine what the `status = 'active'` filter in `claude_team__ai_dev_usage.sql`
    discards, by counting the distinct values present in bronze.
  - Establish what `monthly_credit_limit` and `used_credits` actually mean, since every seat
    metric is computed from them.
  - Settle whether `is_enabled` can serve as the seat-state filter. Closes PRD OD-5.
  - Decide on the fields stalled in bronze: `last_active`, `api_key_name`, and the derivable
    `avg_cost_per_day`, `avg_lines_accepted_per_day`, `prs_with_cc_percentage`.
  - Carry `disabled_reason` and `disabled_until` through the stream schema into bronze. They
    are the only vendor signal that separates "extra usage is switched off" from "the seat is
    not assigned", and reading them is what will finally define `is_enabled` (PRD OD-5). They
    reach bronze only; no staging or silver column is added for them here.

- **Out of scope**: building anything; this feature produces findings and decisions.

- **Requirements Covered**: prerequisite for FR-4, FR-12; resolves PRD OD-5.

- **Data**: reads `bronze_claude_team` streams, `silver.class_ai_dev_usage`,
  `silver.class_ai_overage`.

---

### 2.2 Claude Team — permission-failure visibility — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-claude-team-403`

- **Purpose**: Make an unauthorised overage stream distinguishable from an empty one. Phase 1
  rests entirely on `claude_team_overage_spend`; today a missing permission produces zero rows
  and a green sync, so a demonstration would show emptiness indistinguishable from a tenant
  that genuinely has no data.

- **Depends On**: None.

- **Scope**:
  - The signal is expressed in silver, not in the connector. Data-quality checks read silver
    and gold and never bronze, so a stream that returns nothing cannot be observed at its
    source; and the declarative manifest can only ignore, retry or fail a 403, where failing
    would take unrelated streams down with it. `action: IGNORE` therefore stays, and its
    consequence is caught one layer up.
  - `assert_ai_overage_covers_active_seats` — activity in `class_ai_dev_usage` for a billing
    month implies a `class_ai_overage` row for the same month. Catches a stream that stops
    reporting seats which are demonstrably in use.
  - `assert_ai_overage_stream_not_silent` — a source that reported seats in the previous
    billing month still reports them in the current one. Catches what the first check
    structurally cannot see: a stream gone silent while its seats happen to be idle.
  - Both name the 403 in their `remediation`, so the signal carries its likely cause and the
    operator is pointed at the proxy session key rather than at the data.
  - Stated limits: both are `severity: warn` and scoped to the current billing month; neither
    separates an unauthorised stream from a source decommissioned mid-month, which is why the
    remediation says so.

- **Out of scope**: changing any other stream's error handling.

- **Requirements Covered**: NFR-5.

---

### 2.3 Claude Team — seat economics to the API — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-claude-team-seat-metrics`

- **Purpose**: Close the phase's central gap: the silver class is populated and correct, and
  no client can read it. This also creates the `ai_cost` managed source that Phase 2 extends.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-403` — without it a
  demonstration cannot distinguish absent overage from an absent permission.

- **Scope**:
  - `insight.ai_cost_metric_evidence` — new gold model over `silver.class_ai_overage`, via
    the shared `metric_evidence_table` macro. One record per person-month snapshot per
    measure; `granularity` is `source_summary`; `details` carries seat tier, the limit, and
    whether the limit was known.
  - `insight.ai_cost_metric_observations` — derived from that evidence, via
    `metric_observations_table`. Measures `extra_usage_usd` (`used_credits`, the money) and
    `extra_usage_limit_usd` (the ceiling); dimensions `tool`, `seat_tier`.
  - Date a snapshot at the day it was last read, not at the first of its billing month — the
    convention the retired gold view used, and the reason the current month stays inside short
    rolling windows.
  - Honest-NULL: a seat that spent nothing extra emits zero; a seat with no ceiling emits no
    utilisation row, the ratio having no denominator. The money is unaffected — it does not
    read the ceiling.
  - `registry.yaml` — new `source` block `ai_cost` with `evidence_ref`, three measures with
    `evidence_granularity`; metrics `ai.extra_usage_cost` and `ai.extra_usage_utilisation`, both
    `entity_type: person`, `peer_cohort_key: org_unit`.
  - `explanation` on each key states its billing model and that it is a monthly fact.
  - Regenerate `passports.md` with `analytics passports`.
  - `schema.yml` — `accepted_values` on `measure_key` for both new models.
  - `assert_ai_overage_covers_active_seats` — a dbt data test carrying the coverage
    invariant: activity in `class_ai_dev_usage` for a billing month implies a
    `class_ai_overage` row for the same month, evaluated at the current-month boundary.
  - e2e `ai_seat_extra_usage.test.yaml`: seed one bronze snapshot per seat — all bronze can
    hold, its key carrying no month — run the pipeline, call the API, assert the value, a null
    result on an empty window, no utilisation row where no ceiling is set, and dedup of a
    repeated bronze row. One case asserts that `ai.extra_usage_cost` reads `used_credits` and
    not the excess over the ceiling.
  - e2e `test_ai_seat_extra_usage_history.py`: drive the pipeline twice, replacing bronze
    between runs as a sync does, and assert both billing months survive in
    `class_ai_overage`. The declarative rig seeds once and cannot express a second sync.

- **Out of scope**: `ai.seat_cost` and `ai.seat_underuse` — both land in 2.6.

- **Requirements Covered**: FR-4, FR-5, FR-7, FR-10, FR-11; AC-1, AC-2, AC-3, AC-5, AC-7,
  AC-12.

- **Data**: reads `silver.class_ai_overage`; writes `insight.ai_cost_metric_evidence` and
  `insight.ai_cost_metric_observations`.

- **Interfaces**: `POST /v1/metric-results` gains `ai.extra_usage_cost` and
  `ai.extra_usage_utilisation`; `GET /v1/metric-definitions` gains both with drilldown
  capability.

---

### 2.4 Claude Team — confirm and cover the usage-priced metric — MEDIUM

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-feature-claude-team-cost-verify`

- **Purpose**: `ai.cost` from Claude Team already works and its semantics are confirmed by
  measurement. The task is to pin that meaning down so it cannot drift, and to make the
  contrast with the per-seat figures legible at the point of reading.

  The two figures **overlap**: `ai.cost` prices all consumption at vendor rates, including
  the part a seat fee already covered *and* the part billed on top of it, while
  `ai.extra_usage_cost` is only that second part. Adding them counts the extra usage twice,
  and the overlap is large and variable — measured at 13×–198× between the two for the same
  person-month. FR-5 and AC-3 forbid the sum, not the pair: serving both keys in one response
  is the intended use, and the contrast is the point.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-audit`.

- **Scope**:
  - State in the metric `explanation` that this is consumption at vendor rates, not an
    invoiced amount, that per-seat figures live in separate keys, and that the two must never
    be added. The passport drift test pins the wording.
  - Extend `ai_cost.test.yaml` with a case requesting `ai.cost` and `ai.extra_usage_cost`
    together, asserting each returns its own value from its own source — the pair is served,
    no figure blends them (AC-3).
  - Assert the containment invariant `ai.extra_usage_cost <= ai.cost` over the same person
    and window. It follows from one being a subset of the other, and it is what catches a
    measure wired to the wrong source, which an equality-free test would not.

- **Out of scope**: changing any supplier or any value.

- **Requirements Covered**: FR-2, FR-5, FR-11; AC-3, AC-4.

---

### 2.5 Claude Team — vendor invoices — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-claude-team-invoices`

- **Purpose**: The invoiced layer — what the vendor actually billed. This is the one stream
  the reference implementation has and we do not, and the only source of a seat's base price.
  It is not a separate vendor: the same claude.ai session, through the same proxy.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-audit`.

- **Scope**:
  - Invoice list through the proxy: `GET /api/stripe/{org_id}/invoices`. The response is a
    claude.ai wrapper carrying `total_excluding_tax`, `currency`, `status`, `created_ts`,
    `num_seats` and `hosted_invoice_url` — but **no invoice id and no line items**. `total`
    differs from `total_excluding_tax` on most invoices, so the net-amount rule is the rule
    rather than an edge case.
  - Enrichment via the Stripe hosted chain: derive the account and token identifiers from
    `hosted_invoice_url`, request the hosted invoice page for the real `invoice_id` and a
    short-lived ephemeral key, then request the full invoice with that key for the line
    items. The ephemeral key is never persisted.
  - `silver.class_ai_invoice`, grain tool by invoice key by category. Net
    `total_excluding_tax` — never the tax-inclusive total; the native-currency amount
    alongside a USD-normalised amount.
  - Category from the **structural** Stripe signal — a subscription-item parent means
    `subscriptions`, an invoice-item parent means `overusage` — never from description text.
    A mixed invoice splits into two rows.
  - **The seat price is the line's `hosted_invoice_unit_amount`** — a monthly invoice states
    it directly, so nothing is divided. `quantity` is carried alongside as the seat count for
    that tier. The wrapper's `num_seats` is read by nothing: it is sparsely populated, and
    where it is populated it reports one line's quantity while the invoice covers several
    tiers. Stored for provenance only. This is what makes the hosted chain
    mandatory rather than an enrichment: without line items there is no seat price at all,
    and 2.6 has no input.
  - **A tier is part of the grain.** One invoice prices several tiers at once — Standard at
    $25.00 and Premium at $125.00 in the same period, measured. `hosted_invoice_tier_label`
    and the line's `period` are carried so a price binds to a tier and a billing window
    rather than to an organisation.
  - **Proration lines are excluded from the price, not from the ledger.** A mid-period seat
    change emits two `subscriptions` lines with `hosted_invoice_unit_amount: null` and
    amounts that are partial-period, one of them negative. They are real money and stay as
    invoice rows; they are excluded from seat pricing by the structural flag
    `parent.subscription_item_details.proration`, never by reading the description.
  - Idempotency: line items of a finalised invoice are immutable, so the chain does not
    re-run for an already-enriched invoice.
  - **A `hosted_invoice_url` is followed inside the run that fetched it, and is never stored
    to be followed later.** Stripe expires these URLs 30 days after the due date and never
    later than 120 days; what makes historical invoices reachable at all is that the
    claude.ai wrapper re-issues a fresh URL on every list call. Persisting one and
    re-requesting it on a later run would work in testing and fail quietly in production,
    starting with the oldest invoices.
  - Degradation: a chain failure on one invoice logs and continues; mass failure to derive
    tokens fails the run, because that indicates a URL-format change requiring a human.

- **Out of scope**: invoices from any other vendor.

- **Requirements Covered**: prerequisite for `ai.seat_cost`; independent check for FR-13.

- **Risk**: the multi-step ephemeral-key flow **cannot be expressed declaratively** — this
  needs a CDK connector and a pinned Stripe API version. The bootstrap host is undocumented:
  it appears in no Stripe documentation and no public discussion, carries no version and no
  contract, and can change without notice. There is no supported alternative — the official
  Invoice API authenticates as the *merchant*, and we are the customer — so the exposure is
  accepted and covered by the degradation rules rather than avoided. Four of the five things that could
  have blocked it were measured open (research notes §19): the proxy already forwards
  `/api/stripe/*` through a wildcard route, the installed session key already returns 200 on
  the invoice list, both Stripe hosts are reachable from the ingestion namespace with no
  `NetworkPolicy` in the way, and the CDK connector shape is an existing pattern rather than
  new infrastructure. What remains genuinely risky is the hosted chain itself — an
  undocumented three-hop flow across two hosts whose URL format can drift — and it now sits
  on the critical path, because the seat count it yields has no substitute.

---

### 2.6 Claude Team — seat cost and underuse — MEDIUM

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-feature-claude-team-seat-cost`

- **Purpose**: Add the only figure in this work that expresses waste in money — what a seat
  costs against how little it was used. Closes PRD OD-1.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-invoices` for the price,
  `cpt-insightspec-aicost-feature-claude-team-seat-metrics` for the metric source, and
  `cpt-insightspec-aicost-feature-claude-team-audit` for the seat-state filter.

- **Scope**:
  - `ai.seat_cost` from the `hosted_invoice_unit_amount` of non-proration `subscriptions`
    lines, joined to a person through their `class_ai_overage.seat_tier` — a tenant runs
    several tiers at once, so an organisation-wide seat price would be wrong for most people.
    A tier with no priced line in the window yields no seat cost rather than a fabricated one.
  - `ai.seat_underuse` — the seat's price carried against its observed activity, so that a
    paid seat nobody used is visible as money. Room left under the extra-usage ceiling is
    **not** underuse and is never counted as such: it was never purchased.
  - The seat-state filter lives inside the overage branch rather than being inherited from
    the activity stream. A deactivated person keeps an overage row but loses activity rows,
    and would otherwise read as an idle seat. The gate is `credit_limit_cents IS NOT NULL`;
    `is_enabled` is carried as a dimension and never filtered on (2.1).
  - e2e for both.

- **Requirements Covered**: FR-4, FR-5; AC-7; closes PRD OD-1.

---

### 2.7 Claude API — token usage contract — HIGH — DEFERRED

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-token-usage-contract`

- **Purpose**: Make `silver.class_ai_api_usage` capable of being priced. The model a row
  belongs to is buried inside `unique_key` and unselectable, and the two cache-creation token
  types are summed — merging two different multipliers and making that line item unpriceable
  within roughly 60 percent. Both facts are present in bronze; the loss is ours.

- **Depends On**: None.

- **Scope**:
  - Extend `silver.class_ai_api_usage` with `model`, `service_tier`, `context_window`,
    `uncached_input_tokens`, `cache_creation_5m_tokens`, `cache_creation_1h_tokens`.
    Additive; `unique_key` unchanged, so no key recomputation and no row migration.
  - `claude_admin__ai_api_usage.sql` — map all six straight from bronze; stop summing the two
    cache-creation fields. Existing columns keep their definitions.
  - `src/ingestion/silver/ai/schema.yml` — document the six columns.
  - dbt tests: new token columns non-negative; the four components sum to `input_tokens`
    where all are populated.

- **Out of scope**: any use of the new columns — pricing arrives in 2.8.

- **Requirements Covered**: prerequisite for FR-3; ADR-0003 obligations.

---

### 2.8 Claude API — price card and per-person token cost — HIGH — DEFERRED

- [ ] `p1` - **ID**: `cpt-insightspec-aicost-feature-token-cost`

- **Purpose**: Deliver what a person's AI consumption actually costs where the vendor bills
  per token, as a `derived` figure rather than an `allocated` one.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-seat-metrics` for the `ai_cost`
  source and its evidence relation, and
  `cpt-insightspec-aicost-feature-token-usage-contract` for the columns.

- **Scope**:
  - `insight.ai_price_card` — table plus seed, keyed by tenant, provider, model, token type,
    context window, service tier and validity start, with a nullable validity end, the rate,
    the currency and the row's origin.
  - Resolution: tenant-specific before global; an exact tier and context match, then a
    context-agnostic row, then a fully generic one; the first row whose interval contains the
    day wins.
  - Self-extension: an unpriced combination gets a rate derived from that tenant's recent
    cost report, written as derived and always tenant-scoped.
  - `insight.ai_token_cost_daily` — tokens valued at the resolved rate in one parameterised
    join; API keys resolved to people; unresolvable keys stay unattributed. Cursor
    contributes through the same shape filtered to chargeable events. OpenAI does not
    contribute — its cost is project-grain.
  - Add `token_cost_usd` to the `ai_cost` evidence and observation models; register
    `ai.token_cost`; regenerate `passports.md`.
  - e2e including a validity-boundary case and tenant-scoped rate isolation.

- **Requirements Covered**: FR-3, FR-5, FR-11, FR-12; AC-1, AC-4, AC-9, AC-12; NFR-1, NFR-2.

---

### 2.9 Claude API — org-grain cost class — MEDIUM — DEFERRED

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-feature-cost-class`

- **Purpose**: Give vendor billing line items a home, classified by billing model on the row.
  Ends the orphan state of `to_ai_cost.sql`, which has named `class_ai_cost` as its target
  since it was written while carrying no silver class tag.

- **Depends On**: None.

- **Scope**:
  - `silver.class_ai_cost` with `billing_model` derived from `line_item`; assembled by tag,
    `delete+insert` on `unique_key`, replacing engine keyed on `_version`.
  - Both feeders project `_version` — `to_ai_cost.sql` does not today. Required by
    `ingestion-data-flow` ADR-0001 and load-bearing: vendors revise cost retroactively and
    the tag union resolves duplicates by descending version.
  - `claude_admin__ai_cost` — new staging from `claude_admin_cost_report`.
  - Update the OpenAI connector README and dbt schema, and the claude-admin DESIGN.

- **Requirements Covered**: FR-1, FR-8; AC-9.

---

### 2.10 Claude API — price reconciliation — MEDIUM — DEFERRED

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-feature-reconciliation`

- **Purpose**: Make the price card's correctness observable. A card that drifts without a
  check is worse than no card.

- **Depends On**: `cpt-insightspec-aicost-feature-token-cost` and
  `cpt-insightspec-aicost-feature-cost-class`.

- **Scope**: compare computed token cost against vendor token line items per completed day
  and workspace, full-outer-joined so a day present on only one side surfaces; scope both
  sides to token charges with the priority tier excluded; raise a data-quality signal beyond
  tolerance.

- **Requirements Covered**: FR-13; AC-11.

---

### 2.11 Cost coverage contract — MEDIUM — DEFERRED

- [ ] `p2` - **ID**: `cpt-insightspec-aicost-feature-cost-coverage`

- **Purpose**: Let a client state what is and is not measured from data rather than prose,
  and distinguish a vendor that does not expose a layer from one we have not ingested yet.

- **Depends On**: `cpt-insightspec-aicost-feature-claude-team-seat-metrics` and
  `cpt-insightspec-aicost-feature-token-cost`.

- **Scope**: per provider per layer — usage-priced, token-billed, seat, invoice — one of
  available, not exposed by the vendor, or not ingested, derived from the connector registry.

- **Requirements Covered**: FR-6; AC-6.

- **Coordination**: overlaps `#1986` ask 1; PRD OD-3 settles ownership.

---

## 3. Feature Dependencies

```text
PHASE 1 - Claude Team

cpt-insightspec-aicost-feature-claude-team-audit (HIGH, p1)
    |
    +---> cpt-insightspec-aicost-feature-claude-team-cost-verify (MEDIUM, p2)
    +---> cpt-insightspec-aicost-feature-claude-team-invoices (HIGH, p1)
                                            |
cpt-insightspec-aicost-feature-claude-team-403 (HIGH, p1)
    |                                       |
    +---> cpt-insightspec-aicost-feature-claude-team-seat-metrics (HIGH, p1)
                        |                   |
                        +-------------------+
                                            |
                cpt-insightspec-aicost-feature-claude-team-seat-cost (MEDIUM, p2)

PHASE 2 - Claude API

cpt-insightspec-aicost-feature-token-usage-contract (HIGH, p1)
    |
    +---> cpt-insightspec-aicost-feature-token-cost (HIGH, p1)
              |
              +---> cpt-insightspec-aicost-feature-reconciliation (MEDIUM, p2)
              |            ^
              |            |
              |     cpt-insightspec-aicost-feature-cost-class (MEDIUM, p2)
              |
              +---> cpt-insightspec-aicost-feature-cost-coverage (MEDIUM, p2)

cpt-insightspec-aicost-feature-claude-team-seat-metrics (Phase 1)
              |
              +---> cpt-insightspec-aicost-feature-token-cost
```

**Dependency rationale**:

- `claude-team-seat-metrics` requires `claude-team-403` because a demonstration on an install
  without the billing permission would show emptiness indistinguishable from absent data.
- `claude-team-seat-cost` requires `claude-team-invoices` for the price, `seat-metrics` for
  the metric source, and `audit` for the seat-state filter.
- `token-cost` requires `claude-team-seat-metrics` because the `ai_cost` managed source, its
  evidence relation and its registry source block are created there. The parent design binds
  one evidence relation per source, so the second consumer extends the first's relation
  rather than creating another. This is the one cross-phase dependency; Phase 1 precedes
  Phase 2 in any case.
- `cost-coverage` requires at least one metric per layer before it can report on them.
- `audit`, `403`, `token-usage-contract` and `cost-class` have no dependencies and may
  proceed in parallel.

**Minimum demonstrable slice**: `claude-team-403` plus `claude-team-seat-metrics`. Everything
else in Phase 1 extends it.

---

## 4. Conditional Scope

Items whose shape depends on answers not yet in hand. Each states the default taken if the
answer never arrives, so no feature is blocked waiting.

| Open item | Affects | Default if unanswered |
|---|---|---|
| `is_enabled` semantics (PRD OD-5) | seat-state filter in 2.1 and 2.6 | Closed by not depending on it: 2.1 found one `false` row, consistent with two readings, and gates seat state on `credit_limit_cents IS NOT NULL` instead |
| Whether a null `monthly_credit_limit` means no ceiling or an unknown one | utilisation honest-NULL in 2.3 | Emit no utilisation row either way — the ratio has no denominator. The money is unaffected, so nothing downstream waits on this |
| The usage a seat fee includes | nothing currently built | Not published per tier and carried by no ingested stream. Not needed: the vendor already reports the spend above it as `used_credits` |
| Non-active `status` values | what 2.1 recommends carrying | Keep the current filter; record what it discards |
| Stripe hosted-chain stability | 2.5 | Per-invoice failures degrade to a warning; mass token-derivation failure fails the run, since that signals a format change |
| Cost-type distribution on production data (PRD OD-4) | 2.9 billing-model mapping | Attribute token and search charges; classify the rest as unclassified and attribute them to nobody |
| Scope boundary with `#1986` (PRD OD-3) | 2.11 ownership | Contract as specified; which issue ships it is a planning decision |
| `#1674` attribution-mode field | FR-11 form | Mode declared in the metric `explanation`; moves to the structured field when it exists |
