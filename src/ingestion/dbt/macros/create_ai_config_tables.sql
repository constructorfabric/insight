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
{% endmacro %}
