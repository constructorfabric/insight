---
status: proposed
date: 2026-08-06
---

# ADR-0003: Price card as the source of per-person token cost

**ID**: `cpt-insightspec-adr-claude-admin-price-card`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Effective rates and negotiated discounts](#effective-rates-and-negotiated-discounts)
  - [Reconciliation scope](#reconciliation-scope)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1 — proportional allocation from the cost report](#option-1--proportional-allocation-from-the-cost-report)
  - [Option 2 — price card](#option-2--price-card)
  - [Option 3 — no per-person token cost](#option-3--no-per-person-token-cost)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

> [!NOTE]
> **Deferred, not withdrawn.** `#2437` removed the `claude-admin` connector, and with it the
> `cost_report` stream this decision prices from and the `class_ai_api_usage` relation it
> extends. The decision stands as reasoning for whoever revives that branch; nothing in the
> current release builds it. Cursor, the remaining per-token source, needs no price card — its
> billed amount arrives in money.

## Context and Problem Statement

AI cost must be attributable to a person. For per-token billing there is no direct source:
`GET /v1/organizations/cost_report` groups only by `workspace_id` or `description`, never by
`api_key_id`, so no row in it names a person or a key. Token usage does reach the key —
`bronze_claude_admin.claude_admin_messages_usage` carries `api_key_id`, `model`,
`service_tier`, `context_window` and five separate token counters — and Identity Resolution
maps `api_key_id` to a person.

The question is how to turn per-key tokens into per-key money.

**Scope.** This decision governs the **per-token billing layer only**. Seat-based figures
need no pricing rule at all: `overage_spend_limits` returns `monthly_credit_limit` and
`used_credits` already denominated in money (minor units), so allowance, consumption and
overage are read, not computed. Nothing in this ADR applies to them.

## Decision Drivers

- Per-person token cost must be defensible as fact, not an estimate with an unstated error bar.
- `#1660` grades attribution; the cost work sits at L2. `#1674` requires every figure to
  declare `direct` / `derived` / `allocated`.
- `/v1/metric-results` accepts arbitrary historical windows, so the pricing rule must
  reproduce past periods correctly, not only the present.
- The current day should be priceable. `cost_report` lands only for completed days.
- Vendors add models continuously; a rule needing a code change per model release will drift.
- A tenant may hold negotiated rates that differ from published ones.

## Considered Options

1. **Proportional allocation from `cost_report`** — split a workspace's daily cost across its
   keys by each key's token share: `key_cost = (key_tokens / workspace_tokens) × cost_report_amount`.
2. **Price card** — multiply each key's tokens by a rate held as reference data:
   `key_cost = Σ tokens_i × rate_i`.
3. **Do not attribute token cost to people** — expose it at workspace/project grain only.

## Decision Outcome

**Option 2, the price card**, with `cost_report` retained as a reconciliation signal rather
than a source, the card **versioned by date**, and rates resolvable **per tenant**.

On comparable token rows the two options are arithmetically identical, not merely close.
Restricted to `cost_type = 'tokens'` under one effective rate,
`cost_report_amount = workspace_tokens × rate`, so substituting into option 1 gives
`(key_tokens / workspace_tokens) × workspace_tokens × rate = key_tokens × rate`. The
workspace total cancels. The card reaches the same number without depending on `cost_report`
at all. The card's exactness rests on one assumption — that the effective per-token rate
recovered from `cost_report` is constant across days and context tiers for a given
`(model, token_type, service_tier)` — which is stated as an assumption, not a measurement,
under [Effective rates and negotiated discounts](#effective-rates-and-negotiated-discounts),
and which reconciliation exists to falsify.

The identity holds only within those preconditions. It does not extend to non-token charges
(web search, code execution, session usage), nor to a tenant whose effective rate differs
from the rate in force on the card — there the two options price the same tokens
differently. Both bounds are stated below, in
[Effective rates and negotiated discounts](#effective-rates-and-negotiated-discounts) and
[Reconciliation scope](#reconciliation-scope).

Three supporting mechanisms are adopted with it:

- **The card is reference data, not code.** A table keyed
  `(tenant, provider, model, token_type, context_window, service_tier, valid_from)`. This keeps
  the computation a single parameterised JOIN instead of pulling token rows into an
  application, and makes a rate change a data change with an audit trail.
- **The card self-extends, approximately.** A `(model, service_tier, token_type)` seen in
  traffic with no rate in force gets one derived from recent `cost_report` rows **restricted to
  `cost_type = 'tokens'`** as `SUM(amount) / SUM(tokens)`, written with `origin = 'derived'`
  and with `context_window` NULL. Without that restriction the numerator would carry web
  search, code execution and session charges against a token-only denominator, and the derived
  rate would be wrong rather than approximate. The derivation does not group by context window, so where a model prices context
  tiers differently the derived figure is a traffic-weighted blend of them, not either tier's
  rate. That is why resolution prefers an exact match and the NULL row is the fallback — see
  [Consequences](#consequences) for the bound this carries. A newly released model still
  prices itself after a day of traffic instead of waiting for a release.
- **Divergence is a signal.** Per completed day and workspace, `Σ tokens × card` is compared
  against `Σ cost_report`, joined so that a day present on only one side still surfaces. A gap
  beyond tolerance raises a data-quality alert.

Attribution mode of the result is **`derived`** — computed from measured tokens by a
deterministic rule, with no proportionality assumption. Option 1 would have been `allocated`.

### Effective rates and negotiated discounts

The published card is the *list* rate. A tenant on negotiated terms pays something else, and
for that tenant `tokens × published_rate` is a usage-priced estimate, **not** the billed
amount — which would silently contradict the metric's own claim to be real cost.

The card therefore carries `insight_tenant_id`, resolved most-specific-first:

1. a row for this tenant, valid on the day being priced;
2. otherwise the global row (`insight_tenant_id IS NULL`) — the published rate.

The self-extension mechanism is what makes this practical rather than a manual per-tenant
configuration exercise: rates derived from a tenant's own `cost_report` are *that tenant's effective
rates*, discounts included, and are written tenant-scoped. Derivation is therefore never
written to the global row — one tenant's negotiated rate must not become another's default.

**Assumption, stated so it can be checked:** a tenant's effective per-token rate is constant
within a `(model, token_type, service_tier)` over the validity interval. The reconciliation
step is what detects a violation, and the alert it raises is the signal to add explicit rows.

### Reconciliation scope

Reconciliation compares only what both sides can express. The computed side prices **tokens**;
it does not compute a web-search charge, and `code_execution` and `session_usage` are not
priced at all. Comparing a tokens-only computed figure against a vendor total that also
contains search, execution and session charges would report divergence on perfectly correct
data.

Reconciliation is therefore scoped to **`cost_type = 'tokens'` on both sides**, with
`service_tier = 'priority'` excluded. Should a computed web-search charge be added later, the
scope widens to match — symmetrically, never on one side only.

### Consequences

**Enabling.**
- Per-person token cost becomes computable, and is `derived` rather than `allocated`.
- The current day is priceable; nothing waits on `cost_report`.
- `cost_report` gains a clearer role: reconciliation and drift detection, not attribution.
- A discounted tenant converges on its own effective rates through derivation.

**Costs and obligations.**
- `silver.class_ai_api_usage` must gain `model`, `service_tier` and `context_window` as
  columns. They already discriminate the row — they appear in its `unique_key` — but are not
  selectable, and rate depends on model.
- The `cache_creation_5m` / `cache_creation_1h` split must be preserved rather than summed.
  The two carry different multipliers (×1.25 and ×2 of base input), so merging them makes the
  cache-write line item unpriceable within roughly 60 %. Both fields are present in bronze;
  the loss happens in staging today.
- Rate correctness becomes an ongoing obligation discharged by reconciliation. A card that
  drifts without the check is worse than no card, because it is silently wrong.
- One documented approximation is inherited: rates are held per model with `context_window`
  NULL, exact only while a model charges one rate across context tiers. Resolution prefers an
  exact `(context_window, service_tier)` match and falls back to NULL, so a model that prices
  long context differently can be given explicit rows without changing the rule.

### Confirmation

- Reconciliation over recent completed days agrees with `cost_report` within tolerance per
  `(day, workspace)`, restricted to token line items.
- An e2e fixture seeds known token counts against a known card and asserts the metric value
  to the cent.
- A fixture spanning a `valid_from` boundary asserts each side is priced at its own rate.
- A fixture with a tenant-scoped rate asserts it wins over the global row, and that a second
  tenant is unaffected.

## Pros and Cons of the Options

### Option 1 — proportional allocation from the cost report

- Good: no reference data; the vendor's own total is the anchor, so a completed day cannot
  drift from the invoice.
- Bad: `allocated` by construction — the split is an assumption, and workspace traffic
  carrying no `api_key_id` distorts every key's share.
- Bad: cannot price the current day.
- Bad: correctness depends on `cost_report` freshness, so an ingestion failure becomes a
  cost-accuracy failure.

### Option 2 — price card

- Good: `derived`, deterministic, reproducible from stored inputs.
- Good: prices the current day; no dependency on `cost_report` latency.
- Good: prices any historical window correctly once versioned by date.
- Good: accommodates negotiated rates through tenant-scoped rows.
- Bad: introduces reference data that can go stale — mitigated by self-extension and
  reconciliation, not eliminated.
- Bad: requires the token-type split and the model dimension to be carried through silver.

### Option 3 — no per-person token cost

- Good: nothing to maintain; nothing to get wrong.
- Bad: fails the requirement — per-person and per-team questions stay unanswerable for every
  per-token provider.

## More Information

- `#1607` — AI development cost analytics; this ADR serves its per-token branch.
- `#1660` — attribution levels; `#1674` — attribution modes.
- `ADR-0002` — `cost_report` day-aligned exclusive `ending_at`; the same report is the
  reconciliation input here.
- `#1901`, `#1902` — Claude Admin ingestion defects; code fixes merged (`#1926`, `#1927`),
  issues open pending verification on data.
- Evidence for the constant-rate assumption and the token-multiplier comparison:
  `docs/components/backend/analytics/specs/ai-cost/research-notes.md` §6–§7.
- **Open, for whoever unblocks the per-token branch:** whether self-extension should group by
  `context_window` and write a row per context tier, falling back to the NULL row only where
  the window under derivation carries one tier. It would make derived rates exact per tier at
  the cost of sparser derivation — a tier with little traffic would get no rate at all — and
  it would diverge deliberately from the reference implementation, which groups by
  `(model, service_tier, token_type)`.
  Whatever the grouping, one constraint carries over from the reference: a rate re-derived from
  usage must not be written back over an existing one. A denominator summed from usage can
  include traffic the vendor does not bill, so re-derivation is a detector, not a source — the
  reference derives per full key for exactly that purpose and still refuses to overwrite. Any
  per-tier top-up therefore needs a filtered denominator before it may write anything.
  Not decided here: the mechanism cannot be built or checked against data while the connector
  it reads is absent from the repository.

## Traceability

| Decision | Realised by |
|---|---|
| Rates as tenant-aware, date-versioned reference data | `insight.ai_price_card` |
| Token-type split preserved | `silver.class_ai_api_usage` — `uncached_input_tokens`, `cache_read_tokens`, `cache_creation_5m_tokens`, `cache_creation_1h_tokens` |
| Model dimension carried | `silver.class_ai_api_usage` — `model`, `service_tier`, `context_window` |
| Derived per-person token cost | metric `ai.token_cost` |
| Divergence detection, token line items only | reconciliation check over `cost_report` vs card |
