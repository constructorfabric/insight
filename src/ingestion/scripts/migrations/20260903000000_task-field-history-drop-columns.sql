-- Drop three unread columns from the task field-history journal.
--
-- `title` is deliberately NOT among them. It becomes an ordinary field bound to
-- the `title` role, but its PRODUCER changes only at cutover: while the Rust
-- binary writes this journal, a `summary` row exists only for an issue whose
-- summary actually changed — the snapshot model the binary reads does not list
-- `summary` — whereas the binary fills the `title` column for every row. Gold
-- therefore reads the role and falls back to the column, and the column goes
-- with the binary. Dropping it here would leave every never-renamed issue with
-- no title at all.
--
-- `author_display`, `delta_value_id` and `delta_value_display` have no reader
-- in gold, in silver or in the backend. The class contract's job is to serve
-- ready state; a consumer that needs the detail of a particular change joins
-- back to the event it came from, and `assert_changelog_traceable_to_bronze`
-- is what guarantees that path exists. The two lifecycle arms kept the entity
-- id in `delta_value_id` AND in `value_ids[1]`, so nothing is lost there.
--
-- Only `silver.class_*` belongs in a numbered migration. The three Jira arms
-- that union into the class need the same columns dropped — they are
-- `incremental`, so their tables survive a run with whatever column list they
-- were created with, and the class unions them with `SELECT *` — but a staging
-- table exists only after dbt has built it, and dbt runs AFTER migrations. That
-- drop is therefore a guarded heal in apply-ch-migrations.sh, per the warehouse
-- contract rules in AGENTS.md.
--
-- `staging.jira__task_field_history` is deliberately NOT touched. That relation
-- is the Rust binary's output and the binary still writes all four columns, so
-- dropping them there would make the next enrich run fail on unknown columns —
-- the DDL macro that owns the table keeps declaring them for exactly that
-- reason. Both the table and the macro go away at cutover, which is when the
-- columns' last writer does.
--
-- Order matters. The models stop emitting these columns in the same change, so
-- this runs after they are deployed; on a cluster where the old models are
-- still live the columns simply stay until they are replaced. Every clause is
-- `DROP COLUMN IF EXISTS`, so re-running is a no-op and a partially-applied
-- state converges. The table itself is NOT guarded —
-- ClickHouse has no `ALTER TABLE IF EXISTS`, and the migrations run after the
-- connectors-ddl snapshot has created every class table, so it is always there.
--
-- See src/ingestion/connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md
-- §10.1.

ALTER TABLE silver.class_task_field_history
    DROP COLUMN IF EXISTS author_display,
    DROP COLUMN IF EXISTS delta_value_id,
    DROP COLUMN IF EXISTS delta_value_display;
