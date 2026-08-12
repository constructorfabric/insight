# Claude Team — extraction audit

Findings for `cpt-insightspec-aicost-feature-claude-team-audit` (DECOMPOSITION 2.1).
Working document: it records which checks were run against a populated instance and the
decisions they support, and is not a spec.

Measured against `constructorfabric/insight@main` and `data_collector@54e6892`. Every check
below is stated as a query so it can be re-run; results are not reproduced here.

## 1. Stream coverage against the reference implementation

| Stream | Endpoint | Ours | `data_collector` | Reaches silver |
|---|---|---|---|---|
| `claude_team_members` | `/api/organizations/{org}/members` | ✅ | ✅ `claude_team_members` | ❌ no staging model |
| `claude_team_invites` | `/api/organizations/{org}/invites` | ✅ | ✅ `claude_invites` | ❌ no staging model |
| `claude_team_overage_spend` | `/api/organizations/{org}/overage_spend_limits` | ✅ **wider** | ✅ `claude_spending` | ✅ `class_ai_overage` |
| `claude_team_code_metrics` | `/api/claude_code/metrics_aggs/users` | ✅ **wider** | ✅ `claude_daily_usage` | ✅ `class_ai_dev_usage` |
| *invoices* | `/api/stripe/{org}/invoices` | ❌ **missing** | ✅ `vendor_invoices` | — |

**Two of four streams stop at bronze.** `members` and `invites` have no staging model and
feed nothing — the same as the reference implementation, which also only stores them. No
parity gap, but worth recording: `members` is the only place carrying `role` and
`full_name`, and it feeds neither `class_people` nor any cohort.

**Where we are wider.** `overage_spend` additionally persists `monthly_credit_limit`,
`currency`, `out_of_credits`, `used_credits_basis` and `limit_type`, none of which the
reference stores. The limit is the ceiling on extra usage, not the included allowance (§16 of
the research notes); it is what makes proximity-to-blocking computable.
`code_metrics` carries thirteen fields against the reference's eight.

**The single real gap is invoices** (DECOMPOSITION 2.5).

## 2. `status` — a filter that can drop rows silently

`claude_team__ai_dev_usage.sql:93` applies `WHERE status = 'active'`.

```sql
SELECT status, count() AS rows, uniqExact(email) AS people
FROM bronze_claude_team.claude_team_code_metrics FINAL
GROUP BY status
```

**Decision: keep the filter.** The vendor documents no value set for this field, so a value
other than `active` may appear at any time and would be dropped without a trace. Add a
build-time check that counts non-active rows rather than removing the filter.

## 3. `is_enabled` — resolves PRD OD-5

```sql
SELECT is_enabled, count() AS rows, uniqExact(account_email) AS people
FROM bronze_claude_team.claude_team_overage_spend FINAL
GROUP BY is_enabled
```

A disabled seat can be joined to `class_ai_dev_usage` for the same billing month to see
whether the state co-occurs with an unassigned tier, a null allowance, or absent activity.

**Reading:** the field admits two incompatible interpretations — "the seat is assigned and
active", and the `schema.yml` description, "overage spending is permitted for the seat".
Where a disabled seat also carries an unassigned tier, a null allowance and no consumption,
the signals are perfectly correlated and no observation of that shape can separate them.

Anthropic's own documentation favours the latter — extra usage is described as admin-gated
per person, and its published Spend Limits API models the allowance without any equivalent
boolean. See §16. The reference implementation stores the field and never reads it, so it
settles nothing either.

**Decision for the seat-state filter (2.6):** gate on `credit_limit_cents IS NOT NULL` alone.
Under the enablement reading a seat can be assigned, active and merely barred from
overspending, and excluding it would drop a real seat from utilisation. Carry `is_enabled` as
a dimension, not as a state.

An enabled seat with no activity is an under-utilisation candidate; a disabled one is not.

## 4. Vendor extras — all three are usable

```sql
SELECT limit_type, used_credits_basis, out_of_credits, count() AS rows
FROM bronze_claude_team.claude_team_overage_spend FINAL
GROUP BY limit_type, used_credits_basis, out_of_credits
```

- `limit_type` — distinguishes a seat inheriting the plan-tier default from one carrying a
  per-member override. Meaningful, and worth exposing as a dimension later if per-member
  overrides need review.
- `used_credits_basis` — states whether consumption is measured before or after discount.
  Comparing `used` against `limit` is only valid while both sides share one basis, so a
  `pre_discount` value must invalidate the comparison rather than pass silently. See D6.
- `out_of_credits` — the vendor's hard-exhaustion flag. Where it is not populated, the soft
  `used > limit` computation is the only over-limit signal available.

## 5. Field-level decisions for `code_metrics`

Populated-ness per field is checked with `countIf(<field> IS NOT NULL AND <field> != '')`
against `count()` over `bronze_claude_team.claude_team_code_metrics FINAL`.

| Field | Decision |
|---|---|
| `last_active` | **Do not carry yet.** Underuse is a seat's price against its activity, and activity rows already carry their own dates. It would answer a different and arguably better question — "idle for N days" — but adding a column to a shared class for one connector needs that question to be asked first. Recorded as a candidate for 2.6. |
| `api_key_name` | **Do not carry.** Nothing to attribute while the vendor leaves it empty. |
| `total_cost` | Already carried. A zero-cost row is an activity day with no priced consumption — expected, not a defect. |
| `avg_cost_per_day` | **Do not carry.** Derivable from `total_cost` and active days. |
| `avg_lines_accepted_per_day` | **Do not carry.** Derivable. |
| `prs_with_cc_percentage` | **Do not carry.** Derivable from the two counts. |

## 6. PR attribution is unreportable without the vendor's GitHub app

```sql
SELECT countIf(toFloat64OrNull(toString(prs_with_cc)) > 0) AS with_cc,
       countIf(toFloat64OrNull(toString(total_prs)) > 0)  AS with_prs
FROM bronze_claude_team.claude_team_code_metrics FINAL
```

Anthropic populates both counters only for organisations that have connected its GitHub app.

**Consequences:**

1. **Honest-NULL would be mandatory, not a nicety.** Emitting `0` would assert "no pull
   requests were made with Claude Code", which is false when the vendor simply is not
   reporting.
2. **The feature cannot be demonstrated without a connected app.** It could be built and
   covered by e2e with seeded fixtures, but a live demonstration needs a tenant that has one.
3. Together with the absence of any requirement calling for these counters — they answer
   whether a pull request involved Claude Code, which is `#1660`'s subject, not cost — the
   entry was removed from the decomposition and recorded as a candidate for `#1660`.

## 7. Seat economics — what to show for 2.3

```sql
SELECT period_month,
       count()                                   AS seat_months,
       countIf(is_over_limit = 1)                AS over_limit,
       sum(used_amount_cents)                    AS extra_usage_cents,
       avg(used_amount_cents / credit_limit_cents) AS mean_utilisation
FROM silver.class_ai_overage FINAL
GROUP BY period_month
ORDER BY period_month
```

The properties the demonstration rests on, each visible in that query:

- **Extra usage is `Σ used_credits`** — the money the vendor billed once seats exhausted the
  usage included in their fees. See §9.
- **It concentrates.** A minority of seats accounts for nearly all of it, so the peer view is
  what makes the distribution legible; an average hides it.
- **Seat-months at or past the ceiling are people being blocked**, not people overspending an
  allowance.
- **Mean utilisation is of the ceiling on extra usage**, not of a purchased allowance. Room
  under a ceiling costs nothing, so this reads as proximity to being blocked, never as waste.
  The wasted-seat story needs the seat's price, which arrives with invoices.
- **A seat with no allowance carries a NULL overage**, not a fabricated zero — the honest-NULL
  path. Whether a null allowance means *unknown* or *unlimited* is not settled; see §16.

## 8. Summary of decisions

| # | Decision | Affects |
|---|---|---|
| D1 | Keep the `status = 'active'` filter; add a build-time count of non-active rows | 2.1 → build |
| D2 | Seat-state gate is `credit_limit_cents IS NOT NULL` alone; `is_enabled` is carried as a dimension, never as a filter | 2.6; closes PRD OD-5 |
| D3 | Do not carry `api_key_name`, `avg_cost_per_day`, `avg_lines_accepted_per_day`, `prs_with_cc_percentage` | 2.1 → closed |
| D4 | Do not carry `last_active` yet; record as a candidate for an "idle seat" signal | 2.6 |
| D5 | PR attribution leaves this decomposition; recorded as a candidate for `#1660` | — |
| D6 | Guard `used_credits_basis` — the used-vs-limit comparison is only valid while every row shares one basis | 2.3 |
| D7 | `members` and `invites` stop at bronze in both implementations; no parity gap, no action | — |
| D8 | Invoices are the only genuine extraction gap against the reference | 2.5 |

## 9. `monthly_credit_limit` is a ceiling, not an included allowance

Full working and the vendor citations are in `research-notes.md` §16–§17; the short form:

Over-limit seat-months exceed the limit by a few cents — spending piling up at an enforced
boundary rather than overrunning an allowance. Anthropic's documented model for the same
concept treats the equivalent field as a spend ceiling, with `0` meaning "no extra usage" and
null meaning "no ceiling". The money billed beyond the seat fee is `used_credits` itself.

Consequence: `max(0, used − limit)` reports a figure orders of magnitude smaller than the
extra usage actually billed. Every seat metric in 2.3 and 2.6 is defined from `used_credits`,
and the ceiling serves only proximity-to-blocking.

| # | Decision | Affects |
|---|---|---|
| D9 | `ai.extra_usage_cost` reads `used_credits`; the ceiling feeds only `ai.extra_usage_utilisation`; `ai.seat_underuse` is redefined as a seat's price against its activity | 2.3, 2.6; PRD FR-4 |
