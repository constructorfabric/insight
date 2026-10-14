# shellcheck shell=bash
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

# Every producer of a task class carries its tenant right after unique_key. The
# class unions them positionally, so a producer not yet rebuilt since this
# deploy must already hold the column where its rebuilt siblings do.

# Prints how many real (non-placeholder) tables answer to the name; fails when
# the probe cannot be read, so an unreachable ClickHouse stops the deploy.
_task_producer_present() {
  local count
  count="$(ch_scalar "SELECT count() FROM system.tables WHERE database = 'staging' AND name = '$1' AND comment != 'INSIGHT_PLACEHOLDER_v1'")" || {
    echo "ERROR: cannot tell whether staging.$1 exists" >&2
    return 1
  }
  printf '%s' "${count}"
}

heal_task_producer_tenant() {
  local table="$1" type="$2" present
  present="$(_task_producer_present "${table}")" || return 1
  [[ "${present}" -gt 0 ]] || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS tenant_id ${type} AFTER unique_key;
ALTER TABLE staging.${table} MODIFY COLUMN tenant_id ${type} AFTER unique_key;
SQL
}

# INVARIANT: the Jira journal and lifecycle producers normalize the tenant to
# String, so their old rows read '' rather than NULL; the rest keep it Nullable.
TASK_PRODUCERS_STRING_TENANT=(
  jira__field_history_derived
  jira__comment_lifecycle_events
  jira__worklog_lifecycle_events
)
TASK_PRODUCERS_NULLABLE_TENANT=(
  jira__availability_events
  jira__task_comments jira__task_worklogs jira__task_field_metadata
  youtrack__task_comments youtrack__task_field_history youtrack__task_field_metadata
  youtrack__task_issuetypes youtrack__task_links youtrack__task_projects youtrack__task_sprints
  youtrack__task_statuses youtrack__task_worklogs
  github__task_field_history github__task_links
  jira__task_links
)

heal_task_producer_tenants() {
  local table
  for table in "${TASK_PRODUCERS_STRING_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" String
  done
  for table in "${TASK_PRODUCERS_NULLABLE_TENANT[@]}"; do
    heal_task_producer_tenant "${table}" 'Nullable(String)'
  done
}
