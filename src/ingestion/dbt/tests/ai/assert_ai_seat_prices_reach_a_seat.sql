{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'An invoiced seat price reaches a seat',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a priced seat tier on a vendor invoice that prices no seat, so its money sits on the ledger while ai.seat_cost serves nothing from it. bound_to_population and bound_to_seat_tier say what the binding claims: both empty means nothing binds tier_ref at all, and gold can only take an unbound price when the tenant runs a single connector instance on each side. A non-empty bound_to_population naming an instance that holds no seats, or a non-empty bound_to_seat_tier naming a tier no seat carries, is a stale or mistyped binding. Fix it in config.ai_seat_tier_map, which dbt does not own: name tenant_id, the invoice connector instance in insight_source_id, the class source, the vendor catalogue identifier in tier_ref, the seat_tier a seat carries in silver.class_ai_overage, and — where the tenant runs two instances on either side — the seat population in seat_source_id. Nothing is wrong with the pipeline: gold declines to price a seat it cannot attribute rather than guessing.'
    }
) }}
{#- Reproduces the reachability gold computes in `ai_cost_metric_evidence.sql`,
    which decides per seat population and not per month: an offer reaches the
    population it is bound to, or every population when it is unbound and the
    tenant runs one connector instance on each side. Only then does the tier
    disambiguate, and a population holding exactly one offer takes it whatever
    tier the seat carries.

    Collapsing that to one verdict per month, as an earlier draft did, is wrong
    in both directions: a single unbound offer beside two seat populations
    prices nobody and would go unreported, and a tier that no seat carries stays
    a real finding even when every seat in its population was priced through
    another offer.

    Bounded to the last three billing months: a tier retired long ago has no
    seat population left to reach and would report forever. Recent months are
    the ones a binding can still fix. -#}

WITH
-- Every priced seat line, carried with the binding that says which seats it may
-- reach. An unbound line keeps '' in both fields, and no seat carries an empty
-- instance id or an empty tier, so unbound never matches by accident.
priced_lines AS (
    SELECT
        invoice.insight_tenant_id               AS tenant_id,
        invoice.source                          AS source,
        invoice.period_month                    AS period_month,
        invoice.source_id                       AS invoice_source_id,
        invoice.tier_ref                        AS tier_ref,
        coalesce(binding.seat_source_id, '')    AS offer_population,
        coalesce(binding.seat_tier, '')         AS offer_tier,
        invoice.seat_unit_cents                 AS price_cents
    FROM {{ ref('class_ai_invoice') }} AS invoice FINAL
    LEFT JOIN (
        SELECT
            tenant_id,
            insight_source_id,
            source,
            tier_ref,
            seat_source_id,
            seat_tier
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
      AND invoice.source_id IS NOT NULL
      AND invoice.period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
),

-- INVARIANT: distinct over the triple, because gold collects its offers with
-- `groupUniqArray(tuple(seat_source_id, tier, price))`. Two lines naming the
-- same population, tier and price are one offer to it, and counting them twice
-- would make a population look ambiguous that gold resolves.
offers AS (
    SELECT DISTINCT
        tenant_id,
        source,
        period_month,
        offer_population,
        offer_tier,
        price_cents
    FROM priced_lines
),

month_invoice_sources AS (
    SELECT
        tenant_id,
        source,
        period_month,
        uniqExact(invoice_source_id)            AS invoice_sources
    FROM priced_lines
    GROUP BY tenant_id, source, period_month
),

-- gold's own seat gate, so the population count below matches the one it uses.
seat_months AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        period_month,
        source_id                               AS population,
        coalesce(seat_tier, '')                 AS seat_tier,
        credit_limit_cents
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE email IS NOT NULL
      AND email != ''
      AND collected_at IS NOT NULL
      AND insight_tenant_id IS NOT NULL
      AND source_id IS NOT NULL
),

month_seat_sources AS (
    SELECT
        tenant_id,
        source,
        period_month,
        uniqExact(population)                   AS seat_sources
    FROM seat_months
    GROUP BY tenant_id, source, period_month
),

-- A tier a seat actually holds, in the population holding it. The ceiling is
-- what gates emitting a seat fee at all, so a seat without one is not a seat any
-- price can reach.
seat_tiers AS (
    SELECT DISTINCT
        tenant_id,
        source,
        period_month,
        population,
        seat_tier
    FROM seat_months
    WHERE credit_limit_cents IS NOT NULL
),

populations AS (
    SELECT DISTINCT
        tenant_id,
        source,
        period_month,
        population
    FROM seat_months
),

-- INVARIANT: one join per level. Both sides carry a `tenant_id`, and folding the
-- two counts into one SELECT leaves the unqualified name unresolvable.
populations_with_invoice_count AS (
    SELECT
        population.tenant_id,
        population.source,
        population.period_month,
        population.population,
        invoices.invoice_sources
    FROM populations AS population
    INNER JOIN month_invoice_sources AS invoices
        ON  invoices.tenant_id = population.tenant_id
        AND invoices.source = population.source
        AND invoices.period_month = population.period_month
),

populations_scoped AS (
    SELECT
        counted.tenant_id,
        counted.source,
        counted.period_month,
        counted.population,
        counted.invoice_sources,
        seats.seat_sources
    FROM populations_with_invoice_count AS counted
    INNER JOIN month_seat_sources AS seats
        ON  seats.tenant_id = counted.tenant_id
        AND seats.source = counted.source
        AND seats.period_month = counted.period_month
),

-- The offers that can reach one population: bound to its own connector
-- instance, or unbound while the tenant runs a single instance on each side.
population_offers AS (
    SELECT
        scoped.tenant_id,
        scoped.source,
        scoped.period_month,
        scoped.population,
        offer.offer_population,
        offer.offer_tier,
        offer.price_cents
    FROM populations_scoped AS scoped
    INNER JOIN offers AS offer
        ON  offer.tenant_id = scoped.tenant_id
        AND offer.source = scoped.source
        AND offer.period_month = scoped.period_month
    WHERE offer.offer_population = scoped.population
       OR (offer.offer_population = '' AND scoped.invoice_sources = 1 AND scoped.seat_sources = 1)
),

population_offers_counted AS (
    SELECT
        tenant_id,
        source,
        period_month,
        population,
        offer_population,
        offer_tier,
        price_cents,
        count() OVER (
            PARTITION BY tenant_id, source, period_month, population
        )                                       AS offers_in_population,
        count() OVER (
            PARTITION BY tenant_id, source, period_month, population, offer_tier
        )                                       AS offers_sharing_tier
    FROM population_offers
),

-- An offer prices a seat when it is its population's only one — gold takes that
-- one whatever tier the seat carries — or when the seat's own tier names it and
-- no other offer in that population shares that tier.
reaching_offers AS (
    SELECT DISTINCT
        offer.tenant_id,
        offer.source,
        offer.period_month,
        offer.offer_population,
        offer.offer_tier,
        offer.price_cents
    FROM population_offers_counted AS offer
    INNER JOIN seat_tiers AS seat
        ON  seat.tenant_id = offer.tenant_id
        AND seat.source = offer.source
        AND seat.period_month = offer.period_month
        AND seat.population = offer.population
    WHERE offer.offers_in_population = 1
       OR (offer.offer_tier = seat.seat_tier AND offer.offers_sharing_tier = 1)
)

SELECT DISTINCT
    tenant_id,
    source,
    period_month,
    tier_ref,
    offer_population                            AS bound_to_population,
    offer_tier                                  AS bound_to_seat_tier,
    price_cents                                 AS seat_unit_cents
FROM priced_lines
-- INVARIANT: a tuple NOT IN, never a LEFT JOIN tested for NULL. An unmatched
-- ClickHouse left join yields the column's default, so an unbound offer's empty
-- population would read as a match rather than as the miss it is.
WHERE (tenant_id, source, period_month, offer_population, offer_tier, price_cents) NOT IN (
        SELECT
            tenant_id,
            source,
            period_month,
            offer_population,
            offer_tier,
            price_cents
        FROM reaching_offers
    )
