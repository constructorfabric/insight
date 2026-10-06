{{ config(
    materialized='view',
    schema='insight',
    alias='bronze_insert_events',
    tags=['gold']
) }}

{#-
  merge(REGEXP(...)) has no structure to resolve when no table matches: a read
  fails with CANNOT_EXTRACT_TABLE_STRUCTURE ("there are no tables satisfied
  provided regexp") — a hard error, not an empty relation. Declaring the column
  list makes the CREATE succeed and moves the failure to every SELECT, which is
  worse, not better.

  Nothing pre-creates bronze any more — the Airbyte destination creates it on a
  connector's first sync — so a deploy onto a cluster that has never synced
  reaches this model with zero bronze tables. Emitting a correctly-typed empty
  relation keeps that from taking the rest of the gold build with it.
  `dbt run --select tag:gold` runs on every deploy, so the real view replaces
  the stub on the next one — the same cadence the bronze SELECT grants in
  provision-presentation-access.sh follow. For the same reason the view is kept
  out of the committed DDL snapshot (bootstrap-db/dump-ddl.sh): that snapshot is
  dumped from a warehouse that has bronze, so it could only ever carry the
  unguarded form.

  Counted over TABLES, not databases: merge() needs a matching table, and an
  empty bronze database would satisfy a database-level probe while still
  failing the read.
-#}
{%- set bronze_tables = 0 -%}
{%- if execute -%}
  {%- set probe = run_query(
        "SELECT count() AS n FROM system.tables WHERE startsWith(database, 'bronze_')"
      ) -%}
  {%- if probe is not none and probe.rows | length > 0 -%}
    {%- set bronze_tables = probe.columns[0].values()[0] | int -%}
  {%- endif -%}
{%- endif -%}

-- Ops/monitoring view: one row per PHYSICAL bronze record across every
-- connector database, exposing the extraction timestamp and its origin.
-- Consumers aggregate on top with any interval, e.g.:
--
--   SELECT toStartOfInterval(extracted_at, INTERVAL 15 MINUTE) AS bucket,
--          connector, count() AS rows
--   FROM insight.bronze_insert_events
--   GROUP BY bucket, connector
--
-- Scope filters: prefer `WHERE source_database = 'bronze_bamboohr'` (or
-- `stream = '...'`) — conditions on the merge() virtual columns _database
-- and _table prune non-matching tables before any data is read. The
-- derived `connector` column is for display/grouping; filtering on it works
-- but goes through the replaceOne() expression.
--
-- The toString() wrappers are load-bearing, not tidiness: _database and
-- _table arrive as LowCardinality(String), and a plainly-aliased virtual
-- column throws LOGICAL_ERROR ("input block structure") on any filter over
-- the view (ClickHouse 25.7.5). Casting at the projection makes the filter
-- work while still pruning — a scoped read touches only the matching
-- database's rows.
--
-- Documented exceptions to the layer conventions:
--   * Reads bronze directly from gold. This is an ops relation measuring
--     ingestion itself, not an analytics contract; it has no silver
--     equivalent by definition.
--   * Intentionally NO dedup (no FINAL / LIMIT 1 BY): duplicate physical
--     rows ARE the signal — the view measures insert intensity, not
--     logical row counts.
--   * No source()/ref() lineage: merge(REGEXP) resolves the table set at
--     query time, which is the point — new connector databases and streams
--     appear in the view automatically, with no dbt rebuild.
--
-- Semantics caveat: _airbyte_extracted_at is stamped by the SOURCE at
-- extraction time, not at ClickHouse insert time. The destination buffers
-- and flushes in batches, so rows can land up to ~1h after their
-- extracted_at. This is extraction intensity; true insert times would need
-- system.part_log, which is disabled on the clusters.
{% if bronze_tables > 0 %}
SELECT
    toString(_database)                                AS source_database,
    replaceOne(toString(_database), 'bronze_', '')     AS connector,
    toString(_table)                                   AS stream,
    _airbyte_extracted_at                              AS extracted_at
FROM merge(REGEXP('^bronze_'), '.*')
{% else %}
-- Nothing has been ingested yet. Same columns and types, no rows, so every
-- consumer reads "no extraction activity" instead of erroring.
SELECT
    CAST('' AS String)             AS source_database,
    CAST('' AS String)             AS connector,
    CAST('' AS String)             AS stream,
    CAST(0 AS DateTime64(3))       AS extracted_at
WHERE 1 = 0
{% endif %}
