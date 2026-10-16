# shellcheck shell=bash
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

# Every producer of a task class carries its tenant right after unique_key. The
# class unions them positionally, so a producer not yet rebuilt since this
# deploy must already hold the column where its rebuilt siblings do.

# Prints a count read through ch_scalar; fails when the probe cannot be read,
# so an unreachable ClickHouse stops the deploy instead of reading as zero.
_task_producer_count() {
  local what="$1" query="$2" count
  count="$(ch_scalar "${query}")" || {
    echo "ERROR: cannot tell whether ${what}" >&2
    return 1
  }
  printf '%s' "${count}"
}

heal_task_producer_tenant() {
  local table="$1" type="$2" present
  present="$(_task_producer_count "staging.${table} exists" \
    "SELECT count() FROM system.tables WHERE database = 'staging' AND name = '${table}' AND engine != 'View' AND comment != 'INSIGHT_PLACEHOLDER_v1'")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS tenant_id ${type} AFTER unique_key;
ALTER TABLE staging.${table} MODIFY COLUMN tenant_id ${type} AFTER unique_key;
SQL
}

# A view cannot be altered and only its own connector's run recreates it, so a
# stale one would break every other connector's class union until then. Dropped
# instead: the union skips a producer with no relation, and the view holds no rows.
heal_task_producer_view() {
  local view="$1" stale
  stale="$(_task_producer_count "staging.${view} is a view without tenant_id" \
    "SELECT count() FROM system.tables AS t WHERE t.database = 'staging' AND t.name = '${view}' AND t.engine = 'View' AND NOT has((SELECT groupArray(name) FROM system.columns WHERE database = 'staging' AND table = '${view}'), 'tenant_id')")" || return 1
  [[ "${stale}" -gt 0 ]] || return 0
  echo "  staging.${view} (stale view dropped)"
  run_ch <<SQL
DROP VIEW IF EXISTS staging.${view};
SQL
}

# INVARIANT: each table sits in the list of the type its model emits; a mismatch
# converts the column on every deploy and back on every rebuild.
TASK_PRODUCERS_STRING_TENANT=(
  jira__field_history_derived jira__comment_lifecycle_events jira__worklog_lifecycle_events
  youtrack__task_field_history youtrack__task_field_metadata youtrack__task_issuetypes youtrack__task_links
)
TASK_PRODUCERS_NULLABLE_TENANT=(
  jira__availability_events jira__task_comments jira__task_worklogs jira__task_field_metadata jira__task_links
  youtrack__task_comments youtrack__task_projects youtrack__task_sprints youtrack__task_statuses
  youtrack__task_worklogs
  github__task_field_history github__task_links
)
TASK_PRODUCER_VIEWS=(
  jira__task_issuetypes jira__task_projects jira__task_sprints jira__task_statuses
  github__task_field_metadata github__task_issuetypes github__task_projects github__task_statuses
)

heal_task_producer_tenants() {
  local table
  for table in "${TASK_PRODUCERS_STRING_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" String
  done
  for table in "${TASK_PRODUCERS_NULLABLE_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" 'Nullable(String)'
  done
  for table in "${TASK_PRODUCER_VIEWS[@]}"; do
    heal_task_producer_view "${table}"
  done
}
