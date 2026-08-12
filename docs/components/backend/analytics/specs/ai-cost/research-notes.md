# AI cost analytics (#1607) — verified research notes

Working notes for the AI development-cost work. Every claim here was checked against code or
against a populated instance; the source of each check is named inline so it can be re-run.
Results of the instance checks are deliberately not reproduced here.

Checked against:
- `constructorfabric/insight@main` = `6e354f34` (2026-08-06)
- a populated ClickHouse instance, read through a read-only account
- `data_collector` @ `54e6892` (gitlab.constr.dev/frontend/data_collector)
- Anthropic public pricing, 2026-06-24 snapshot

---

## 1. The three cost layers

"Cost" in #1607 covers three different quantities. They are not interchangeable and
each needs its own metric.

| Layer | Question it answers | `data_collector` | Insight |
|---|---|---|---|
| **Usage-priced** | what would this consumption cost at the vendor's published rates, ignoring any subscription | `claude_platform_key_costs`, `openai_platform_key_costs`, `cursor_usage_events` | `class_ai_dev_usage.cost_cents` → metric `ai.cost` |
| **Seat / credit** | how much of the seat's included monthly allowance was consumed, and how much was spent above it | `claude_spending`, `openai_subscription_usage`, `openai_subscription_balance` | `silver.class_ai_overage` (no metric) |
| **Invoiced** | what the vendor actually billed | `vendor_invoices` (+ `apps/invoices`, `packages/vendor-invoices`) | nothing |

`#1607`'s title says "real costs in money", which is the invoiced layer. Its body
describes the first two. The gap is deliberate to flag, not an oversight to paper over.

---

## 2. Metric target state

| Metric | Meaning | Billing model | Source |
|---|---|---|---|
| `ai.cost` *(exists)* | consumption priced at vendor rates; includes usage a seat already covered; excludes seat fees; **not an invoiced amount** | none (hypothetical) | `class_ai_dev_usage.cost_cents` |
| `ai.token_cost` *(new)* | real cost under per-token billing | per-token | Claude Admin `tokens × price card`; Cursor `chargedCents WHERE isChargeable`. OpenAI cost is project-grain and stays in `class_ai_cost` — no project-to-person mapping exists |
| `ai.seat_cost` *(new)* | base price of a seat, per tier | per-seat | invoices — `hosted_invoice_unit_amount` on non-proration `subscriptions` lines; **not** `num_seats`, **not** amount/quantity (§19, §20) |
| `ai.extra_usage_cost` *(new)* | spend above the usage included in the seat fee | per-seat | `class_ai_overage.used_amount_cents` — see §17, this was originally recorded as `overage_cents` |
| `ai.extra_usage_utilisation` *(new)* | how close a seat is to the ceiling that blocks it | per-seat | `class_ai_overage` |
| `ai.seat_underuse` *(new)* | a seat's price against its observed activity — waste in money | per-seat | invoices + `class_ai_dev_usage` |

`ai.seat_cost` cannot use `Sum` over a date window — a seat costs the same whether the
window is 3 days or 30. Confirm the metric registry supports a non-additive computation
before committing to it.

---

## 3. What `ai.cost` supplies today — verified

Definition: `src/backend/services/analytics/src/domain/metric_definitions/builtin.rs:282-305`.
Source `ai_usage` → `source_ref` `ai_metric_observations`, measure `cost_usd`,
dimension `tool`, computation `Sum`, format `Currency`, entity `Person`.

Chain:

```
bronze_cursor.cursor_usage_events ──→ cursor__event_cost_daily.sql ──┐
bronze_claude_team.claude_team_code_metrics ─────────────────────────┼─→ class_ai_dev_usage.cost_cents
                                    claude_team__ai_dev_usage.sql ───┘
        ↓ gold/ai_metric_observations.sql: sum_measure('cost_usd', …, 'cost_cents / 100', 'tool_dimensions')
   insight.ai_metric_observations
        ↓ builtin.rs BuiltinSource ai_usage + MetricSeed ai.cost
   POST /v1/metric-results
```

### Cursor — matches the declared semantics

`connectors/ai/cursor/dbt/cursor__event_cost_daily.sql` header:

> `chargedCents` is an event's full priced value and ALREADY INCLUDES `cursorTokenFee` —
> summing both double-counts the fee. Every event kind is included: `Included in Business`
> events are priced usage a seat covered, so this total is usage at Cursor's rates, not an
> invoiced amount.

`isChargeable` / `isFreeBugbot` / `isTokenBasedCall` are deliberately not applied as
filters. Applying `isChargeable` would produce the amount actually billed — that is the
input for `ai.token_cost`, not `ai.cost`.

### Claude Team — matches, confirmed by data

`claude_team__ai_dev_usage.sql` maps `total_cost` (from
`GET /api/claude_code/metrics_aggs/users`, decimal-as-string) to `cost_cents`.
Whether that field is API-rate pricing or consumed subscription credits was undocumented
on both sides. Settled by comparing the two meters for the same person and billing month:
`Σ cost_cents` from `class_ai_dev_usage` against `used_amount_cents` and
`credit_limit_cents` from `class_ai_overage`.

The two meters diverge by a large factor, and the factor is **not constant across people** —
so it is not a unit conversion. The magnitudes are consistent with `total_cost` pricing
consumed tokens at Anthropic's published API rates, while `used_credits` is bounded by the
seat's allowance and measures a different meter.

**Conclusion: `total_cost` is usage priced at API rates.** Claude Team belongs in
`ai.cost` as it stands. No supplier needs redirecting.

---

## 4. Which relations to check before believing a report of emptiness

```sql
SELECT count() FROM bronze_claude_team.claude_team_overage_spend FINAL;
SELECT count() FROM bronze_claude_team.claude_team_code_metrics FINAL;
SELECT count() FROM silver.class_ai_overage FINAL;
SELECT count(), countIf(cost_cents IS NOT NULL) FROM silver.class_ai_dev_usage FINAL;
```

What the answers settle:

- **Whether `class_ai_overage` is populated at all.** Reports of "0 rows" in the issue history
  refer to particular installs and cannot be generalised; each install has to be counted.
- **Whether the 403 `billing:view` hypothesis applies.** A non-zero
  `claude_team_overage_spend` means the stream is authorized and returning data, so emptiness
  elsewhere has another cause. Independently of the answer, the handler in
  `connectors/ai/claude-team/connector.yaml:215-224` swallows a 403 into an empty, green sync
  — an unauthorized stream is indistinguishable from an empty one, which is a real
  observability gap.
- **Whether the over-limit and honest-NULL paths are exercised** — a seat above its ceiling
  gives a non-zero `overage_cents`, a seat without a ceiling gives NULL rather than zero.
- **Whether cost is attached to activity rows**, and whether a second tool (Cursor)
  contributes cost on the same install.

Access to a populated instance is an operator matter and is not recorded here.

---

## 5. What is missing

| Gap | Evidence |
|---|---|
| `silver.class_ai_cost` does not exist | `connectors-ddl/silver.sql` defines `class_ai_api_usage`, `class_ai_assistant_usage`, `class_ai_dev_usage`, `class_ai_overage` only |
| OpenAI cost model is an orphan | `connectors/ai/openai/dbt/to_ai_cost.sql:6` — header says "→ class_ai_cost", `tags=['openai']` carries no `silver:` tag, so `union_by_tag` never picks it up |
| Claude Admin has no cost staging | `connectors/ai/claude-admin/dbt/` contains only `claude_admin__ai_api_usage.sql` and `claude_admin__ai_dev_usage.sql`; bronze `claude_admin_cost_report` exists and is unused |
| No seat/overage metric keys | `builtin.rs` `ai.*` namespace: `accepted_lines`, `removed_lines`, `active_days`, `cost`, `accepted_edit_actions`, `tool_acceptance_rate`, `assistant_messages`, `assistant_actions`, `dev_conversations`, `chat_assistant_conversations` |
| No per-provider cost-coverage flag | ask 1 of #1986 |
| Team/role is not an entity | `enum EntityType { Person }` — `builtin.rs:6-8`; all 59 metric seeds are `EntityType::Person` |
| `$/PR` has no attribution-mode field | blocked on #1674 |
| `insight.ai_cost_person_period` unused by the registry | migration `20260623000000_ai_personal_gold_views.sql` defines it; the registry reads `ai_metric_observations`. **Decision: leave it alone, document that it is unread.** |

---

## 6. Price card — the mechanism for per-person token cost

The blocker was believed to be that `GET /v1/organizations/cost_report` cannot group by
`api_key_id` (only `workspace_id` or `description`), so token cost could not reach a person.
`data_collector` shows that is not a blocker.

From `openspec/changes/same-day-cost/design.md`:

> The effective per-token price `computeKeyCosts` back-derives from `cost_report` is a
> **static constant** equal to Anthropic's published price card, constant across days *and*
> context tiers … Stored per-key costs reproduce the Anthropic Console figures to the cent.

So `per_key_cost = tokens × price_card`, and `cost_report` becomes a reconciliation
signal rather than the source. The arithmetic identity: the old allocation was
`(key_tokens / workspace_tokens) × cost_report_amount`, and
`cost_report_amount = workspace_tokens × price`, so the fraction cancels to
`key_tokens × price`.

Two algorithms exist in `data_collector`; only the second should be ported:

| | Old — `openspec/specs/claude-key-costs` | New — `openspec/changes/same-day-cost` |
|---|---|---|
| Method | proportional split of workspace cost by token share | tokens × price-card rate |
| Attribution mode (#1674) | `allocated` | `derived` |
| Current day priced | no (waits for `cost_report`) | yes |
| Depends on `cost_report` freshness | yes | no |

### Supporting mechanisms to port with it

- **Price-card table.** `data/migrations-pg/039-claude-platform-price-card.sql`, keyed
  `(model, token_type, context_window, service_tier) → price_per_mtok_cents`. Stored as
  data, not code, so the cost computation stays a single parameterised JOIN
  (`apps/claude-platform/src/price-card-sql.ts`).
- **Auto top-up.** `PRICE_CARD_TOPUP_SQL`, `apps/claude-platform/src/cost-report-sync.ts:67+`.
  Back-derives missing prices as `SUM(amount_cents) / SUM(tokens)` per
  `(model, service_tier, token_type)` over a recent window, so a newly released model prices
  itself after one day of traffic.
- **Drift reconciliation.** `RECONCILE_SQL`, same file line 172+. Compares
  `tokens × card` against `cost_report` per `(completed date, workspace)` via
  `FULL OUTER JOIN`, so a day present on only one side still surfaces. Scoped to
  `cost_type IN ('tokens','web_search')` and excludes `service_tier = 'priority'`.
  The reference is symmetric — its card side prices web search too, and
  `code_execution` is excluded from both sides. Not ported as-is: Insight computes no
  web-search charge, so its own reconciliation narrows to `cost_type = 'tokens'` on both
  sides until one exists (ADR-0003, Reconciliation scope).
- **Where this lives now.** The file references above hold at the pinned revision. The
  reference has since moved top-up, reconciliation and the price-drift work into
  `apps/claude-platform/src/cost-accuracy.ts` (with migrations 041/042), leaving
  `cost-report-sync.ts` to fetch and delegate. Grep the newer file when reading their
  current main.
- **A separate price-drift detector exists there too**, and its design carries a warning worth
  inheriting: it re-derives the price of every `(model, token_type, context_window,
  service_tier)` over a recent window and reports a deviation beyond tolerance, but it never
  writes a derived price back. A denominator summed from usage can include traffic the vendor
  does not bill, which makes a naive re-derivation confidently wrong rather than merely
  imprecise. Top-up may add a missing combination; nothing may silently replace an existing
  rate.

### Documented limitations to inherit

`apps/claude-platform/src/cost-report-sync.ts:61-66`:

> Known limitation: prices are aggregated across context_window tiers and stored with
> context_window NULL, which is exact only while a model charges one per-token rate for
> every context tier (true for every model in traffic today — verified against the
> 1M-context line items). A model that prices long context differently would land here as
> a blended average and needs explicit per-tier rows instead; the prefer-exact-then-NULL
> ordering in price-card-sql.ts already resolves those.

**Addition for Insight: the price card must be date-versioned** (`valid_from` / `valid_to`).
`data_collector` stores one current price per key. `/v1/metric-results` accepts arbitrary
historical windows, so recomputing a past period against today's prices would silently
misprice history.

---

## 7. Token-type split — why merging 5m and 1h is a pricing error

Anthropic prices token types relative to the model's base input rate. Published rates
(2026-06-24 snapshot): Opus 5 / 4.8 / 4.7 / 4.6 $5 / $25 per MTok; Sonnet 5 / 4.6 $3 / $15;
Haiku 4.5 $1 / $5; Fable 5 $10 / $50. Cache multipliers: **read ×0.1**, **write ×1.25 for
the 5-minute TTL and ×2 for the 1-hour TTL**.

`connectors/ai/claude-admin/dbt/claude_admin__ai_api_usage.sql:88-93`:

```sql
toUInt64(coalesce(uncached_input_tokens, 0)
       + coalesce(cache_read_tokens, 0)
       + coalesce(cache_creation_5m_tokens, 0)
       + coalesce(cache_creation_1h_tokens, 0)) AS input_tokens,
toUInt64(coalesce(cache_read_tokens, 0))        AS cache_read_tokens,
toUInt64(coalesce(cache_creation_5m_tokens, 0)
       + coalesce(cache_creation_1h_tokens, 0)) AS cache_creation_tokens,
```

`uncached_input_tokens` is recoverable by subtraction. The 5m/1h split is not — and those
two carry **different multipliers (×1.25 vs ×2)**, a 60 % price difference on that line
item. Both fields are present in bronze, so nothing is lost upstream; the merge is a
staging decision to reverse.

`model`, `service_tier`, and `context_window` appear in the row's `unique_key`
(same file, lines 72-79) but are **not columns** of `silver.class_ai_api_usage`. Price
depends on model, so they must become columns.

---

## 8. Attribution modes (#1674) and the L-ladder (#1660)

`#1674` requires every figure to carry how it was obtained:

| Mode | Definition | Example here |
|---|---|---|
| `direct` | measured at the grain it is reported at | Cursor `chargedCents` for one event |
| `derived` | computed from measurements by a deterministic rule | `tokens × price_card` |
| `allocated` | split by an assumption of proportionality | `(key_tokens / workspace_tokens) × workspace_cost` |

`#1660` grades attribution L1–L7; `#1607` sits at **L2** (cost at person / team / role ×
period, ground truth) and inherits the **L7** ceiling: no honest per-PR cost. Per-PR figures
may appear only as `allocated` (a person's period cost spread over their PRs) or
`cohort-ratio` (`Σ cohort cost / Σ cohort PRs`), each explicitly labelled.

Choosing the price-card algorithm moves per-person token cost from `allocated` to `derived`
— a strictly better position, not a constraint.

---

## 9. Vendor invoices — what porting costs

`data_collector` retrieves Claude invoices in three hops (`openspec/specs/claude-invoices-sync`):

1. `GET https://claude.ai/api/stripe/{ORG_ID}/invoices?limit=12&page=` — browser-authenticated;
   returns a claude.ai wrapper (`invoices[]`, `has_more`, `next_page`) with
   `total_excluding_tax`, `currency`, `status`, `created_ts`, `num_seats`,
   `hosted_invoice_url` — but **no invoice id and no line items**.
2. `GET https://invoicedata.stripe.com/hosted_invoice_page/{acct}/{token}` → `invoice_id`
   plus a short-lived `ephemeral_key`. `{acct}` and `{token}` are base64-decoded out of
   `hosted_invoice_url`.
3. `GET https://api.stripe.com/v1/invoices/{invoice_id}/hosted` with
   `Authorization: Bearer {ephemeral_key}`, a pinned `Stripe-Version`, and
   `Stripe-Account: {acct}` → full invoice with `lines.data[]`.

Storage rules (`openspec/specs/vendor-invoices`): net `total_excluding_tax` (never the
tax-inclusive `total`); `amount_cents_native` plus `amount_cents` normalised to USD;
category from the **structural** Stripe signal — `parent.subscription_item_details` →
`subscriptions`, `parent.invoice_item_details` → `overusage` — never from description
strings; a mixed invoice splits into two rows; `num_seats` stored but read by nothing (§19).

For Insight this is a new CDK connector, not a stream on an existing one: it needs the
claude.ai browser session (the existing `claude-team-proxy` provides it), two new external
hosts, multi-step ephemeral-key logic that a declarative connector cannot express, and a
pinned third-party API version. Separate PR, separate task.

---

## 10. Connector correspondence

| `data_collector` app | Surface | Insight connector |
|---|---|---|
| `apps/claude` (+ `claude-non-rnd`) | `claude.ai` web API via Playwright: `/api/organizations/{org}/members`, `/overage_spend_limits`, `/invites`, `/api/claude_code/metrics_aggs/users`, `/api/stripe/{org}/invoices` | `claude-team` (same endpoints via `claude-team-proxy`) |
| `apps/claude-platform` | `api.anthropic.com` Admin API: `cost_report`, `usage_report/messages`, `usage_report/claude_code`, `users`, `api_keys`, `workspaces`, `invites` | `claude-admin` |
| `apps/openai-platform` | `platform.openai.com` Admin: `costs`, `completions-usage`, `key-costs`, `projects`, `audit-logs` | `openai-api` |
| `apps/openai` | ChatGPT web via Playwright: `subscription-usage`, `subscription-balance`, `usage`, `export` | `chatgpt-team` (partial) |
| `apps/cursor` | Cursor API: `members`, `audit-logs`, `usage-events`, `daily-usage` | `cursor` |
| `apps/invoices` | Stripe hosted-invoice chain | — |
| — | Enterprise analytics (wire format undocumented) | `claude-enterprise` |
| — | — | `github-copilot`, `jetbrains`, `windsurf` |

`claude.ai` serves both Team and Enterprise plans, so "the Enterprise analogue" and "the
Team analogue" are the same app on that side. Insight's separate `claude-enterprise`
connector targets a different, undocumented analytics surface with no counterpart.

Insight's `claude_team__ai_dev_usage.sql` header cites
`/api/organizations/{org_id}/claude_code/metrics`, but `connector.yaml:325` uses
`/api/claude_code/metrics_aggs/users` — the same path `data_collector` uses. The SQL
comment is stale.

---

## 11. `cost_type` — classify by row, not by connector

`cost_type` is a column of `bronze_claude_admin.claude_admin_cost_report`
(`connectors-ddl/claude-admin.sql:72`), populated from the field of the same name in
`GET /v1/organizations/cost_report`. Values: `tokens`, `web_search`, `code_execution`,
`session_usage`.

`data_collector` uses only two of them:

- `apps/claude-platform/src/key-cost-sync.ts:213` — `AND cost_type = 'tokens'`
- `apps/claude-platform/src/key-cost-sync.ts:304` — `cost_type = 'web_search'`, allocated by
  web-search request count because those rows carry no `model`
- `apps/claude-platform/src/cost-report-sync.ts:186` — reconciliation over
  `cost_type IN ('tokens','web_search')`, with the comment
  *"(code_execution excluded from both sides)"*

`session_usage` and `code_execution` are attributed to nobody. What `session_usage`
represents is undocumented; the working hypothesis is the subscription/seat portion arriving
through the same report. Either way the rule to adopt is the same: **classify each cost row
by `cost_type`, not by which connector produced it.** A `billing_model` column on
`class_ai_cost` is the natural place for that classification.

---

## 12. `is_enabled` vs `status` — different fields, different streams

| | `is_enabled` | `status` |
|---|---|---|
| Endpoint | `GET /api/organizations/{org}/overage_spend_limits` | `GET /api/claude_code/metrics_aggs/users` |
| Stream (`connector.yaml`) | `claude_team_overage_spend` (line 168) | `claude_team_code_metrics` (line 288) |
| Bronze table | `bronze_claude_team.claude_team_overage_spend` | `bronze_claude_team.claude_team_code_metrics` |
| Grain | one row per seat, monthly snapshot | one row per (`metric_date`, `email`) |
| Used in | `claude_team__ai_overage.sql` → column `is_enabled` | `claude_team__ai_dev_usage.sql:93` → `WHERE status = 'active'` |
| Meaning | undocumented | undocumented beyond `'active'` |

Consequence for seat-utilisation metrics: the denominator comes from `class_ai_overage`
while activity comes from `class_ai_dev_usage` filtered to `status='active'`. A deactivated
person keeps their overage row but loses their activity rows, and would register as an
under-utilised seat. The state filter has to live inside the overage branch. `is_enabled`
looked like the candidate when this section was written; §16 and audit decision D2 settle it
the other way — the gate is `credit_limit_cents IS NOT NULL` alone, and `is_enabled` is
carried as a dimension. This paragraph is kept for the reasoning, not for its conclusion.

### Other `overage_spend_limits` fields

Declared in `connector.yaml:168-210`:

| Field | Meaning | Landing place |
|---|---|---|
| `monthly_credit_limit` | **superseded — see §17.** Recorded here as the included monthly allowance; measurement shows it is the admin-set ceiling on extra usage. Already in cents (`10000` ⇒ $100.00) | `credit_limit_cents`, no ×100 |
| `used_credits` | extra usage billed this month once the seat's included usage was exhausted — the money (§17), also cents | `used_amount_cents` |
| `currency` | ISO currency of both money fields | `currency`, `coalesce(…,'USD')` |
| `out_of_credits` | hard exhaustion flag; distinct from the soft `used > limit` | `overage_metrics_json` |
| `used_credits_basis` | whether `used_credits` is pre- or post-discount ; see D6 | `overage_metrics_json` |
| `limit_type` | `seat_tier` (plan default) vs `member` (per-member override) | `overage_metrics_json` |
| `seat_tier` | tariff tier | `seat_tier` |
| `is_enabled` | undocumented | `is_enabled` |

`data_collector` reads `monthly_credit_limit` in its `ApiSpendItem` interface
(`apps/claude/src/usage-sync.ts:19`) but does not persist it
(`INSERT INTO claude_spending`, lines 158-161). Insight does persist it, and both
`overage_cents` and utilisation depend on it — so Insight's contract is the richer one here.

---

## 13. Correction windows

Claude and Cursor revise cost and usage data for up to ~30 days. `data_collector` handles
this with a bounded re-query window (`TOPUP_WINDOW_DAYS`, `RECONCILE_WINDOW_DAYS` ≈ 35 days),
a 180-day backfill in 31-day windows on first run, and a 3-day overlap in steady state.

Insight's incremental staging models use a 3-day lookback — e.g.
`claude_admin__ai_api_usage.sql`: `WHERE toDate(date) > (SELECT max(day) … - INTERVAL 3 DAY)`.
Revisions older than three days are therefore not picked up. This is an existing data-quality
gap independent of #1607 and should be tracked separately.

---

## 14. Backlog items raised by this work

1. **A 403 on `claude_team_overage_spend` is silently green.**
   `connectors/ai/claude-team/connector.yaml:215-224` maps HTTP 403
   (`sessionKey lacks billing:view`) to `action: IGNORE`, so a permissions failure and a
   genuinely empty stream are indistinguishable — both leave `class_ai_overage` empty with a
   green sync. Where the stream does return rows the permission is evidently present, so
   emptiness there has another cause - but this will hide the failure on an install where
   the permission is missing.
   Fix direction: surface it as a data-quality signal rather than a silent skip.
2. **3-day incremental window vs ~30-day vendor corrections** — see §13.
3. **Stale endpoint reference** — `claude_team__ai_dev_usage.sql` header cites
   `/api/organizations/{org_id}/claude_code/metrics`; the connector uses
   `/api/claude_code/metrics_aggs/users`.
4. **#1607 body corrections** — the overage bullet reads as done (the silver class exists
   under its contract, but no unified metric key does); the Cursor bullet predates the #1952
   fix; Claude Admin is listed as a per-person token-cost source although
   `cost_report` has no person grain without the price card.
5. **`dbt source freshness` is scheduled nowhere** — see §15. Every connector declares
   thresholds; no workflow runs the command. Platform-wide, not AI-specific; should become
   its own issue linked to `#1607`.
6. **Vendor pull-request attribution deferred** — `prs_with_cc_count` and `prs_total_count`
   reach silver unread. Removed from this decomposition as `#1660` territory; no requirement
   here needs them, and the vendor populates them only where Anthropic's GitHub app is
   connected — everywhere else both counters read zero without meaning it.
7. **#1986 "Verified state" corrections** — the Cursor hard-NULL claim is stale, and
   `ai_cost_person_period` is described as the grain the catalog could read, though the
   unified registry reads `ai_metric_observations`.

## 15. The data-quality framework, and what it can and cannot see

Checks are singular dbt tests that select violating rows; a scheduled job runs the tests
tagged `data_quality` (`dbt test --selector data_quality`) and emits one JSON finding per
check. Conventions in `src/ingestion/dbt/tests/README.md`; runner in
`charts/insight/templates/ingestion/data-quality-test.yaml`. Twelve checks existed before
this work.

**Checks may read silver and gold only, never bronze** — silver exists regardless of the
connector set, so a check stays valid on a tenant without the connector, whereas bronze may
be absent and would make the check error. This is what forces the 403 signal (NFR-5) to be
expressed as a silver coverage invariant rather than a direct read of the empty stream:
a person cannot use Claude Team without occupying a seat, so activity in
`class_ai_dev_usage` implies a `class_ai_overage` row for the same billing month.

**`dbt source freshness` is never invoked.** Every connector declares `freshness` thresholds
in its `schema.yml` — Claude Team's are tuned per stream — but no workflow, chart template
or script runs the command; the repo invokes
`dbt run`, `test`, `build`, `parse`, `seed`, `compile` and `snapshot` only. The thresholds are
inert, so a stream that stops producing raises nothing on that path.

**No e2e test exercises any data-quality check.** The rig selects a silver class without the
downstream `+`, and a singular test is a node below the model it references, so the
`data_quality` catalog never enters the selection. The scheduled workflow runs it in a
deployed environment (`dbt test --selector data_quality`), which is the only place these
checks execute today. The e2e schema fixture for `claude_team_overage_spend` now exists.

**Coverage check** — people with Claude Team activity but no seat row for the same billing
month. Months that predate the connector show a shortfall; months it covered show none.
`overage_spend_limits` is a snapshot of the month in progress and is never backfilled, so a
completed month holds only the seats captured while it was current. Any coverage check must
therefore be bounded to the current billing month.

## 16. What the vendor documents about `is_enabled` and a null allowance

The stream we read, `GET /api/organizations/{org}/overage_spend_limits`, is an internal
claude.ai web endpoint with no published contract. Anthropic does document a public analogue
for Enterprise, the **Spend Limits API** — and its model differs from ours in ways that matter.

`GET /v1/organizations/spend_limits/effective` returns one row per member with `amount`,
`currency`, `period`, `period_to_date_spend`, and a `source` naming the level the limit was
inherited from (`user` / `seat_tier` / `rbac_group` / `organization`). That `source` is the
same idea as our `limit_type` (`member` vs `seat_tier`). **There is no `is_enabled` field**;
enablement is not modelled as a per-row boolean at all.

**`amount` is nullable, and its meaning depends on which row you read.** On an effective row,
`null` means **unlimited** and `"0"` means the member may not spend beyond the plan's
included usage. On a configured row, `null` only means no numeric limit is set, and the docs
state explicitly that the effective row must be read to tell unlimited from included-only.

This is a live risk to our treatment of `monthly_credit_limit IS NULL` as *unknown*: on the
vendor's own model the same shape can mean *unlimited*, under which overage is definitionally
zero rather than unmeasurable. Where a NULL allowance co-occurs with an `unassigned` tier,
`is_enabled = false` and zero consumption, "unlimited" is implausible — but the
ambiguity is real, is acknowledged by Anthropic for its own API, and should be resolved
before any seat metric depends on it.

**On the meaning of `is_enabled`, the documentation now points the other way from §3's
reading.** Anthropic's help centre describes extra usage as admin-gated per person — an owner
"enables extra usage" and can "enable usage credits for specific users or the entire
organization". That vocabulary matches the original `schema.yml` description ("whether
overage/extra-usage is enabled for the seat") more directly than the audit's "the seat is
assigned and active". The single observed `false` row is consistent with both.

**The reference implementation cannot settle it either.** `data_collector` persists the field
(`claude_spending.is_enabled INTEGER NOT NULL DEFAULT 1`, written at
`apps/claude/src/usage-sync.ts:153`) and never reads, filters or reports on it.

**A third reading, reported.** The author of the reference implementation reads the field as
*whether a limit is set at all*. That is consistent with the shape a disabled seat takes here —
no allowance alongside it — and it would make the field equivalent to
`credit_limit_cents IS NOT NULL`, which is the gate D2 already chose. It is a reading offered by
someone who stores the field and never reads it, not vendor documentation, so it corroborates
the decision rather than settling the question; `disabled_reason` is what will settle it.

**Consequence.** Do not use `is_enabled` as a seat-state filter. Under the enablement reading
a seat can be assigned, active and simply barred from overspending, and excluding it would
drop a real seat from utilisation. It is also redundant wherever a disabled seat carries no
allowance: such a row is already excluded by `credit_limit_cents IS NOT NULL`.

Sources: [Spend Limits API](https://platform.claude.com/docs/en/manage-claude/spend-limits-api),
[Manage usage credits for Team and seat-based Enterprise plans](https://support.claude.com/en/articles/12005970-manage-usage-credits-for-team-and-seat-based-enterprise-plans),
[Claude Code and new admin controls for business plans](https://www.anthropic.com/news/claude-code-on-team-and-enterprise).

## 17. `monthly_credit_limit` is a spend cap, not an included allowance

This contradicts 12, which recorded the field as the
"included monthly allowance", and it changes what the seat metrics in 2.3 should compute.

**The limit behaves as an enforced ceiling.** Most seat-months where `used` exceeds `limit`
exceed it by a few cents — spending piling up exactly at a boundary and overshooting only by
the request that crossed it. The larger excesses read as caps lowered part-way through a
month. An included allowance would produce a smooth distribution of overspend; a cap produces
this one.

**The vendor's documented model matches, value for value.** In the Spend Limits API, `amount`
is the ceiling on a member's spend, `"0"` means the member may not spend beyond the plan's
included usage, and `null` means no ceiling — both shapes occur in practice. The two APIs are
the same model.

**What the money actually is.** `used_credits` is extra usage already beyond the seat's
included allowance, billed at API rates — that *is* the overage. The limit only caps it.

```sql
SELECT period_month,
       count()                              AS seats,
       sum(used_amount_cents)               AS extra_usage_cents,
       countIf(used_amount_cents = 0)       AS spending_nothing,
       countIf(is_over_limit = 1)           AS at_or_past_cap,
       max(used_amount_cents)               AS largest_single_seat,
       sum(coalesce(overage_cents, 0))      AS excess_over_cap
FROM silver.class_ai_overage FINAL
GROUP BY period_month
ORDER BY period_month
```

The two right-hand columns are the point: the excess over the cap is a small fraction of the
extra usage actually billed, and a majority of seats spend nothing extra at all.

**Consequences for the planned metrics.**

- `ai.extra_usage_cost` as `max(0, used − limit)` reports a figure orders of magnitude below
  the extra usage billed. The per-person overage figure is `used_credits` itself.
- `ai.seat_underuse` as `limit − used` is not wasted money: unused room under a spend cap was
  never purchased. `used / limit` remains meaningful as headroom against throttling — seats do
  reach the cap — but it is an operational signal, not a cost one.
- Genuine seat waste needs the seat's **price** against its **activity**, which is why it
  depends on invoices rather than on this stream.
- This also explains §5's ratio: `total_cost` from `code_metrics` values all consumption at
  API rates including what the included allowance covers, while `used_credits` counts only
  what is billed beyond it. Both are real; they measure different things and must never be
  summed (FR-5).
- The honest-NULL question from §16 mostly dissolves: the money figure does not read the
  limit at all. A null limit only leaves headroom undefined.

## 18. `cc_overage` computed the other quantity, on a surface since retired

A retired gold view, `insight.ai_bullet_rows`, served `cc_overage` from
`class_ai_overage.overage_cents` — that is, `max(0, used − limit)`. By §17 that is not the
money, so the figure it showed and `ai.extra_usage_cost` differ by orders of magnitude while
both are called overage.

**The surface is no longer created.** `refactor(ingestion): remove the superseded gold metric
views` (#2332) deleted the migrations that build all 51 legacy views, on the reasoning that
none has a reader: no gold model selects from one, the runtime accepts only relations ending in
`_metric_observations` or `_metric_evidence`, and the catalog that names them is write-only.
Two backend migrations still bind `cc_overage` into that catalog and seed its entry, and older
seed migrations still carry query text selecting from the view — all of it inert while nothing
reads the catalog.

What remains is therefore latent rather than live: a cluster provisioned before the removal
keeps the relations until they are dropped out of band, and the disagreement returns only if
something starts reading them again. Our metric was renamed to `ai.extra_usage_cost` so the two
names no longer collide either. If a reader ever appears, the choice recorded here still
applies: repoint `cc_overage` at `used_amount_cents`, relabel it as spend past the ceiling, or
retire it in favour of the registry metric.

## 19. Invoice feasibility

**The connector's own README is where the endpoint and chain contract lives** —
`src/ingestion/connectors/ai/claude-team-invoices/README.md`, which arrives with the connector
itself (#2429). It states what the wrapper
returns and does not return, the hops that reach the line items, the rule that identifies a
seat price, and what the connector must be allowed to reach. Kept there rather than here
because that is where the next person to touch the connector will look.

Two things from this investigation belong to the plan rather than to the connector:

**`num_seats` cannot carry the seat price**, so the hosted chain is a prerequisite for
`ai.seat_cost` and not an enrichment of it. That is why 2.5 lands before 2.6.

**The CDK runtime is an existing pattern, not new infrastructure.** Of 26 connectors, 19 are
declarative manifests and at least five are Python CDK sources with their own `Dockerfile`,
`pyproject.toml`, `descriptor.yaml` and `source_*/` package — github-copilot, hubspot,
salesforce, bitbucket-cloud, github-v2. `build-images.yml` already builds connector images.
The 2.5 risk note should be read accordingly: the chain is the hard part, the runtime is not.

## 20. The seat price, and what it binds to

The line-level rules are in the connector README with the rest of the chain contract. Two
consequences shape `ai.seat_cost` itself and are recorded here because the metric, not the
connector, carries them:

1. **A tenant runs several tiers at once**, and one invoice prices each of them separately. A
   seat price is therefore per tier, never per organisation, and reaches a person through
   `class_ai_overage.seat_tier` rather than by dividing an invoice total.
2. **Proration lines carry no unit price.** A mid-period seat-count change emits a credit for
   the unused time and a charge for the remainder; both are subscription lines and both are
   real money, but `amount / quantity` over them yields a partial-period figure that is not a
   price. They are excluded structurally, by
   `parent.subscription_item_details.proration`, never by reading a description.

Prepaid extra-usage purchases arrive as invoices of their own, which makes them the
invoiced-layer counterpart of `ai.extra_usage_cost` — the money behind the `used_credits` a
seat later consumes.

## 21. Hosted invoice URLs expire; the wrapper is what keeps history reachable

Stripe documents the lifetime of a `hosted_invoice_url` plainly:

> "Invoice URLs expire 30 days after the due date. If the invoice doesn't have a due date, the
> invoice expires 30 days after it finalizes. In all cases, the expiration window is never
> longer than 120 days." — [Hosted Invoice Page](https://docs.stripe.com/invoicing/hosted-invoice-page)

and adds that a URL *retrieved through the API* stays valid for at least 10 days even past
that point.

This is why an invoice far older than that window is still reachable through a URL taken
from the list call: **claude.ai re-issues a fresh URL on every list call.** The reference
implementation noticed the rotation without connecting it to expiry — its comment reads
"raw hosted_invoice_url can't be used directly (it rotates)".

**The operational rule this fixes:** follow the URL inside the run that fetched it, never
store one and follow it later. Storing it works in every test — the URL is fresh — and then
fails in production, starting with the oldest invoices and progressing forward, which is a
failure mode that reads as "old invoices stopped enriching" rather than as a defect.

**The bootstrap host is undocumented.** `invoicedata.stripe.com` appears in no Stripe
documentation and in no public discussion; it is what the hosted page's own front end calls.
There is no supported alternative: the official Invoice API authenticates as the merchant,
and for these invoices the merchant is Anthropic. The same documentation notes that Stripe
detects non-browser clients on PDF download URLs and answers them 400 — evidence that client
discrimination exists on that surface, though it did not appear on the path used here.
