# `config.task_value_map` seed

Operator-authored bindings from a vendor value to the canonical value a class
dimension carries — a status category, an issue kind. Every `*.tsv` here is
packed into the `insight-task-value-map` ConfigMap by `make task-value-map`
(chained into `make deploy`) and applied by the umbrella's
`clickhouse-task-value-map` hook Job on every upgrade.

Enable per environment with `clickhouse.taskValueMap.enabled: true` in
`environments/<env>/values.yaml`. No files here means no rows — never an error.

## Format

Tab-separated, one decision per line. `#` comments and blank lines are ignored;
an optional header line is allowed. `value_display` and `note` may be omitted.

```
tenant_id  insight_source_id  data_source  field_id  value_id  canonical_value  value_display  note
```

| column              | meaning                                                                      |
|---------------------|------------------------------------------------------------------------------|
| `tenant_id`         | Insight tenant the decision belongs to                                        |
| `insight_source_id` | the connector instance the value came from                                    |
| `data_source`       | `github`, `jira`, …                                                           |
| `field_id`          | the vendor field: `type`, `state`, `project_status:<board>`, …                |
| `value_id`          | the vendor's key for the value (see the `create_task_config_tables` macro)    |
| `canonical_value`   | the canonical value gold consumes                                             |
| `value_display`     | the name the decision was made against, so a later rename is detectable       |
| `note`              | free text for whoever reads the row next                                      |

`valid_from`, `recorded_at` and `recorded_by` are not authored here — the loader
stamps them (`valid_from` epoch, i.e. the mapping is the baseline for all
history; `recorded_by=gitops`).

## Insert-only

The loader anti-joins each line against the live table on
`(tenant_id, insight_source_id, data_source, field_id, value_id)` and writes
only the keys with no stored decision. A key that already carries one — including
a retraction (`is_deleted = 1`) — is left untouched, so editing a line here does
NOT change a mapping that is already loaded, and re-deploying is a no-op.

That is deliberate. `config.task_value_map` is a ReplacingMergeTree whose
`unique_key` includes `valid_from` and `recorded_at`, so a re-INSERT does not
collide with the stored row: it becomes a newer decision and silently wins. This
file is a seed for undecided keys only.

To correct a mapping that is already loaded, INSERT the correction directly
against ClickHouse with a real `valid_from` / `recorded_at` — the journal keeps
both, and this loader never touches either.
