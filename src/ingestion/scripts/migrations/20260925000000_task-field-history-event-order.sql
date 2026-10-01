-- `event_order` joins the class contract: the one column that orders a task's
-- history (`task_event_order`), which gold reads in the same release.
--
-- Positional inserts need the column where the models project it, right after
-- `_seq`. The staging arms follow in apply-ch-migrations.sh, since a staging
-- table exists only after dbt has built it.
--
-- Rows already stored get an approximation, so gold built at deploy never
-- sorts by the zeros a bare ADD COLUMN leaves: the old reading key folded into
-- one number. It lacks the per-entry rank and the creation floor of the real
-- value, which the full refresh the connectors' major versions dispatch writes
-- at the next sync. The guard keeps a re-run from touching rows twice.
--
-- The table is not guarded: the migrations run after the connectors-ddl
-- snapshot has created every class table, so it is always there.

ALTER TABLE silver.class_task_field_history
    ADD COLUMN IF NOT EXISTS event_order Int64 AFTER _seq;

ALTER TABLE silver.class_task_field_history
    MODIFY COLUMN event_order Int64 AFTER _seq;

ALTER TABLE silver.class_task_field_history
    UPDATE event_order = toUnixTimestamp64Milli(event_at) * 1000000
                       + multiIf(event_kind = 'synthetic_initial', 0, event_kind = 'changelog', 1, 2) * 100000
                       + _seq
    WHERE event_order = 0
    SETTINGS mutations_sync = 1;
