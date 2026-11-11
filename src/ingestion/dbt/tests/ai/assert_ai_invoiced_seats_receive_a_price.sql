{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'An invoiced seat receives a price',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a month whose vendor invoice priced a seat while some seat got no fee, so that money sits on the ledger and ai.seat_cost serves nothing for those people. The finding column says which of the two states it is, and they are fixed in different places. no_tiered_seats: the invoice priced seats and the month holds no seat a fee can reach, so no binding can help. Check whether seat rows arrived for that month at all and whether they carry ceiling and tier data: absent rows point at the seat connector and its last sync, while rows present without a ceiling point at what the vendor reported for them, since the ceiling is what makes a seat priceable. unpriced_seats is 0 there because there is no seat to count. seats_unpriced: that many seats of this tier exist and took no fee, so the binding is what is missing or wrong. Add or correct a row in config.ai_seat_tier_map, which dbt does not own, naming tenant_id, the invoice connector instance in insight_source_id, the class source, the vendor catalogue identifier in tier_ref, and the seat_tier those seats carry. Where the count is short of the tier total rather than equal to it, a SECOND seat population is the one left out: the tenant runs two connector instances, and each needs its own row naming its population in seat_source_id — an unbound price reaches neither. In neither state is the transformation at fault: gold declines to price a seat it cannot attribute rather than guessing. A month whose only priced tier is unambiguous needs no binding at all, and for such an installation an empty map is the correct state.'
    }
) }}
{#- Reports two observable states and deliberately does NOT reproduce the
    reachability rule gold applies (`ai_cost_metric_evidence.sql`: offers filtered
    per seat population, then a tier disambiguation). It reads that model's
    OUTPUT instead, so it cannot drift from the rule that decided it — a copy of
    that rule lived here once and was wrong in both directions.

    The seat side is COUNTED against the served side rather than tested for
    existence, which is what lets one unpriced seat population show through
    beside a priced one: they share a tenant, a tool and a tier, and differ only
    by the connector instance that evidence does not carry.

    What the outcome cannot see, and the trade taken knowingly: a priced tier
    that reaches nobody while every seat that does exist is priced — a stale
    binding naming a tier the organisation no longer holds, which leaves no seat
    to count short. Nobody is missing a figure in that state, which is why it is
    the half worth giving up for a check that cannot silently disagree with the
    model.

    Bounded to the last three billing months: a tier retired long ago has no seat
    population left to reach and would report forever. Recent months are the ones
    an operator can still act on. -#}

WITH
-- The only invoice shape that states a per-seat amount, so the only one whose
-- absence downstream is a coverage gap rather than a shape the vendor never sent.
-- Kept per seat population the operator bound the price to ('' when unbound), the
-- same reach gold gives it: one instance's invoice does not make a month priced
-- for another instance whose own invoice has not arrived yet.
priced_populations AS (
    SELECT DISTINCT
        invoice.insight_tenant_id               AS tenant_id,
        invoice.source                          AS source,
        invoice.period_month                    AS period_month,
        coalesce(binding.seat_source_id, '')    AS seat_source_id
    FROM {{ ref('class_ai_invoice') }} AS invoice FINAL
    LEFT JOIN (
        SELECT
            tenant_id,
            insight_source_id,
            source,
            tier_ref,
            seat_source_id
        FROM {{ source('config', 'ai_seat_tier_map') }} FINAL
        WHERE is_deleted = 0
    ) AS binding
        ON  binding.tenant_id = invoice.insight_tenant_id
        AND binding.insight_source_id = invoice.source_id
        AND binding.source = invoice.source
        AND binding.tier_ref = invoice.tier_ref
    WHERE invoice.line_id IS NOT NULL
      AND invoice.category = 'subscriptions'
      AND invoice.is_proration = 0
      AND invoice.seat_unit_cents IS NOT NULL
      AND invoice.insight_tenant_id IS NOT NULL
      AND invoice.period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
),

-- A seat a fee can reach at all. gold gates the seat measure on the ceiling, so
-- a seat without one is not a seat any invoice can price. `source` joins the
-- invoice side and `tool` joins the served side, which is why both are kept.
--
-- INVARIANT: 'unknown' for an absent tier, matching the DIMENSION gold writes —
-- not the '' it uses when picking a price. The same column carries two fillers
-- there, and this side joins the dimension. A seat with no tier can still be
-- priced, because the single-offer branch takes that price whatever tier the seat
-- carries, so the wrong filler here reports a seat that was in fact served.
tiered_seats_per_population AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        source_id,
        tool,
        period_month,
        coalesce(seat_tier, 'unknown')          AS seat_tier,
        count()                                 AS seats
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE credit_limit_cents IS NOT NULL
      AND email IS NOT NULL
      AND email != ''
      AND insight_tenant_id IS NOT NULL
    GROUP BY tenant_id, source, source_id, tool, period_month, seat_tier
),

tiered_seats AS (
    SELECT
        tenant_id,
        source,
        tool,
        period_month,
        seat_tier,
        sum(seats)                              AS seats
    FROM tiered_seats_per_population
    GROUP BY tenant_id, source, tool, period_month, seat_tier
),

-- Seats of a population some invoice priced: a price bound to the population
-- itself, or an unbound price, which may be the missing binding to report.
priced_tiered_seats AS (
    SELECT
        tenant_id,
        source,
        tool,
        period_month,
        seat_tier,
        sum(seats)                              AS seats
    FROM tiered_seats_per_population
    WHERE (tenant_id, source, period_month, source_id) IN (
            SELECT tenant_id, source, period_month, seat_source_id FROM priced_populations
        )
       OR (tenant_id, source, period_month, '') IN (
            SELECT tenant_id, source, period_month, seat_source_id FROM priced_populations
        )
    GROUP BY tenant_id, source, tool, period_month, seat_tier
),

-- COUNTED, not tested for existence. gold emits one seat-fee row per seat-month
-- it priced, so the count is how many seats got a fee — and a shortfall against
-- the seats that exist is the only way to see a population left unpriced beside
-- one that was priced. Existence alone cannot: the two share a tenant, a tool and
-- a tier, and only their connector instance differs, which evidence does not
-- carry. Counting needs no key from it.
--
-- INVARIANT: deliberately NOT reconstructed from `record_id`. That string is
-- built from the connector instance and the vendor seat id, and its shape has
-- already changed once — the outer projection used to fold the entity into it.
-- A check keyed on that formula reports every seat the day it changes again.
served_seats AS (
    SELECT
        tenant_id,
        arrayFirst(d -> d.1 = 'tool', dimensions).2      AS tool,
        metric_date                                      AS period_month,
        arrayFirst(d -> d.1 = 'seat_tier', dimensions).2 AS seat_tier,
        count()                                          AS served
    FROM {{ ref('ai_cost_metric_evidence') }}
    WHERE measure_key = 'seat_cost_usd'
    GROUP BY tenant_id, tool, period_month, seat_tier
),

-- The invoice priced a seat and its population holds none to price: the bound
-- population itself, or any population for an unbound price.
no_tiered_seats AS (
    SELECT DISTINCT
        tenant_id,
        source,
        period_month,
        'no_tiered_seats'                       AS finding,
        ''                                      AS seat_tier,
        toUInt64(0)                             AS unpriced_seats
    FROM priced_populations
    -- INVARIANT: a tuple NOT IN, never a LEFT JOIN tested for NULL. An unmatched
    -- ClickHouse left join yields the column's default, which for a count is 0
    -- and would read as a real answer.
    WHERE (seat_source_id != ''
           AND (tenant_id, source, period_month, seat_source_id) NOT IN (
               SELECT tenant_id, source, period_month, source_id FROM tiered_seats_per_population
           ))
       OR (seat_source_id = ''
           AND (tenant_id, source, period_month) NOT IN (
               SELECT tenant_id, source, period_month FROM tiered_seats
           ))
),

-- Seats of this tier exist, the month priced a seat, and fewer of them were
-- served a fee than exist. A shortfall equal to the tier's whole population is
-- the common shape — nothing was bound. A shortfall smaller than it means some
-- seats were served and others were not: a second seat population left out
-- beside one that was priced.
seats_unpriced AS (
    SELECT
        seat.tenant_id,
        seat.source,
        seat.period_month,
        'seats_unpriced'                        AS finding,
        seat.seat_tier,
        -- Cast because subtracting two counts yields Int64, and the other branch
        -- of the UNION carries a UInt64. The WHERE below makes it positive.
        toUInt64(seat.seats - coalesce(served.served, 0)) AS unpriced_seats
    FROM priced_tiered_seats AS seat
    LEFT JOIN served_seats AS served
        ON  served.tenant_id = seat.tenant_id
        AND served.tool = seat.tool
        AND served.period_month = seat.period_month
        AND served.seat_tier = seat.seat_tier
    -- A left join that matched nothing yields 0 here, which is the honest
    -- reading: no seat of this tier was served a fee.
    WHERE seat.seats > coalesce(served.served, 0)
)

SELECT tenant_id, source, period_month, finding, seat_tier, unpriced_seats
FROM no_tiered_seats
UNION ALL
SELECT tenant_id, source, period_month, finding, seat_tier, unpriced_seats
FROM seats_unpriced
