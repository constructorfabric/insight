-- The Jira journal is derived in dbt from this release on; the class contract
-- changes in two ways with it.
--
-- The four discriminators become `LowCardinality(String)`. Each source
-- contributes its own arm to the class, and an enum type made every arm spell
-- out the values of all the others — the Jira derivation alone adds three kinds
-- (`retired_field`, `unclassified_field`, `snapshot_diff`) that the GitHub arm
-- has no reason to know about. The accepted values are data tests in the
-- class's schema.yml. Conversion is by name, so no value changes.
--
-- `title` leaves. It was an issue attribute copied onto every event row and
-- filled by the retired producer; the title is now an ordinary field bound to
-- the `title` role, and gold reads the role alone. The three incremental Jira
-- arms drop the column in apply-ch-migrations.sh, since a staging table exists
-- only after dbt has built it.
--
-- The table is not guarded: the migrations run after the connectors-ddl
-- snapshot has created every class table, so it is always there. Every
-- statement converges on re-run.
--
-- See src/ingestion/connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md §10.

ALTER TABLE silver.class_task_field_history
    MODIFY COLUMN event_kind LowCardinality(String),
    MODIFY COLUMN field_cardinality LowCardinality(String),
    MODIFY COLUMN delta_action LowCardinality(String),
    MODIFY COLUMN value_id_type LowCardinality(String),
    DROP COLUMN IF EXISTS title;
