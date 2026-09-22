{#-
  Creates the operator-authored AI configuration relation. dbt does NOT own its
  contents: an operator writes the rows, dbt only guarantees the table exists and
  reads it as a source. A dbt-owned model would recreate it on every run and wipe
  the decisions it holds.

  It answers what no vendor states. An invoice line names the tier it prices with
  the vendor's own catalogue identifier; a seat names its tier with an identifier
  from a different API. Nothing in either payload says the two are the same tier,
  and the two vocabularies need not even resemble each other — so the binding is
  a decision, and it is made per installation.

  Keyed on the vendor's identifier rather than the plan's display name: the name
  is localised marketing copy and moves without notice, the catalogue identifier
  does not. Empty is the correct initial state — with no binding, gold prices a
  seat only where a month leaves no ambiguity to resolve.

  A row also names the seat population its prices reach, in `seat_source_id`.
  The invoice and the seats arrive through separate connector instances whose
  `insight_source_id` never matches, so nothing in the data says which seats an
  invoice billed; a tenant running two instances of one vendor needs that said
  out loud. Left empty a row reaches seats only where the tenant runs one
  instance on each side, which is why a single-install tenant needs no row at all
  to keep its seats priced, and why a tenant with two gets no price from an
  unbound row even in a month when only one of them invoiced.

  Called from `on-run-start` so the table exists before any model reads it.
-#}

{% macro create_ai_config_tables() %}
    {% do run_query("CREATE DATABASE IF NOT EXISTS config") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.ai_seat_tier_map
        (
            tenant_id         String,
            insight_source_id String,
            -- The class's own `source` value ('claude_team'), not its data_source:
            -- the binding is per vendor, and that is the column gold joins on.
            source            LowCardinality(String),
            tier_ref          String,
            unique_key        String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                    source, ':', tier_ref),
            seat_source_id    String DEFAULT '',
            seat_tier         String,
            is_deleted        UInt8   DEFAULT 0,
            note              String  DEFAULT '',
            recorded_by       String  DEFAULT '',
            _version          DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {#-
      The price of one usage credit, in the currency the vendor bills.
      Operator-authored for the same reason as the tier map: no vendor API states
      it. A credit is a vendor-internal unit, and the figure that turns it into
      money arrives on a contract, not on an endpoint.

      Dated, because the price it holds is a contract term with a start date the
      operator knows. `effective_from` is the day the vendor's price began to
      apply; the next row implicitly closes the one before it, so no interval can
      overlap another and there is nothing to keep consistent. A day earlier than
      the first row resolves to no price at all, which is the correct answer:
      absence is expressed, not filled. To stop pricing from a date — a contract
      that ended — insert a row at that date with is_deleted = 1 rather than a
      price of zero, which would read as "free".

      Currency conversion is NOT here. Gold converts to the reporting currency
      when it reads, and the converted figure is an estimate; what this table
      holds is the amount the vendor actually charges, which is the reproducible
      fact.

      Empty is the correct initial state. With no row, gold reports no money —
      never a zero, which would read as "this cost nothing".
    -#}
    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.ai_credit_price
        (
            tenant_id            String,
            insight_source_id    String,
            -- The class's own `source` value ('chatgpt_team'), matching
            -- ai_seat_tier_map: the price is per vendor, not per connector run.
            source               LowCardinality(String),
            -- The day this price began to apply. In the key, so a price change
            -- adds a row instead of overwriting what priced earlier days.
            effective_from       Date,
            unique_key           String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                       source, ':', toString(effective_from)),
            -- Minor units of price_currency per ONE credit, held as a Decimal so
            -- a sub-cent price does not round to nothing before it is summed.
            price_minor_units    Decimal(18, 6),
            price_currency       LowCardinality(String),
            is_deleted           UInt8   DEFAULT 0,
            note                 String  DEFAULT '',
            recorded_by          String  DEFAULT '',
            _version             DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {#-
      The rate that carries a billed currency into the reporting one.

      Undated on purpose, and that is a weaker guarantee than the price above —
      say so rather than imply otherwise. A real exchange rate moves daily and
      nothing in this system publishes one, so a dated table here would oblige an
      operator to maintain rows nobody will maintain, and a stale row wearing a
      date claims an accuracy it does not have. One rate, restated when someone
      chooses to, is the honest shape.

      The consequence is deliberate and bounded: the amount the vendor billed is
      reproducible because it is held in the billed currency, and only its
      presentation in the reporting currency moves when the rate is restated.
      Anything derived from this is an estimate and is labelled one.
    -#}
    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.ai_currency_rate
        (
            tenant_id            String,
            from_currency        LowCardinality(String),
            to_currency          LowCardinality(String),
            unique_key           String DEFAULT concat(tenant_id, ':', from_currency, ':', to_currency),
            rate                 Decimal(18, 6),
            is_deleted           UInt8   DEFAULT 0,
            note                 String  DEFAULT '',
            recorded_by          String  DEFAULT '',
            _version             DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}
{% endmacro %}
