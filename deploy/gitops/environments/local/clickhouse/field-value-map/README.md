# Seed bundle for `config.field_value_map`

Operator decisions binding a vendor value of a standardized field to the
canonical value gold consumes. The file mirrors the table: one dictionary for
every standardized field, no new format per field. A new field needs one
`FIELD_DOMAINS` entry in the loader, which rejects any field it has no domain
for. Every `*.tsv` here is packed
into the `insight-field-value-map` ConfigMap by `make field-value-map` (chained
into `make deploy`) and applied by the umbrella's `clickhouse-field-value-map`
hook Job on every upgrade. Files other than `defaults.tsv` land in
`config.field_value_map`; `defaults.tsv` lands in `config.field_value_defaults`.

Enable per environment with `clickhouse.fieldValueMap.enabled: true` in
`environments/<env>/values.yaml`. No files here means no rows — never an error.

The ConfigMap name is `$(RELEASE)-field-value-map`, and the chart's default is
`<insight.fullname>-field-value-map`; under a `fullnameOverride` these differ,
so set `clickhouse.fieldValueMap.configMap` explicitly there. The loader logs
the empty mount at WARNING if they ever drift apart.

## Format

Tab-separated, one decision per line. `#` comments and blank lines are ignored;
an optional header line is allowed. Only `note` may be omitted.

```
tenant_id  insight_source_id  data_source  field  source_key  display_name  target_value  note
```

| column              | meaning                                                                      |
|---------------------|------------------------------------------------------------------------------|
| `tenant_id`         | Insight tenant the decision belongs to                                        |
| `insight_source_id` | the connector instance the value came from                                    |
| `data_source`       | `github`, `jira`, …                                                           |
| `field`             | what is standardized — currently only `issue_type`                            |
| `source_key`        | the vendor's key for the value (see the `create_task_config_tables` macro)    |
| `display_name`      | the name the decision was made against, so a later rename is detectable       |
| `target_value`      | canonical value; domain per field — for `issue_type`: `bug`, `task`, `unknown` |
| `note`              | free text for whoever reads the row next                                      |

A `field` outside the supported set is rejected — the loader names the set in
the error. Extending the dictionary to a new field means adding its domain to
the loader, nothing else: no new file, no new format.

`valid_from`, `recorded_at` and `recorded_by` are not authored here — the loader
stamps them (`valid_from` epoch, i.e. the decision is the baseline for all
history; `recorded_by=gitops`).

## Defaults

`defaults.tsv` assigns, per (tenant, source, field), the value an unmapped
source key falls to. It is required: with no row an unmapped key silently
reaches gold's hardcoded `unknown` terminal, which reads exactly like a
decision nobody made. A source whose unmapped closures really are unclassified
says so with `default_value` `unknown`; that is a decision, and it is not the
same thing as leaving the row out. `assert_task_field_value_defaults_exist`
blocks the gold build in CI when a row is absent, and the Grafana rule
`insight-field-value-defaults-missing` reports it in a deployed install. All
four columns are required; `default_value` obeys the same per-field domain as
`target_value`.

```
tenant_id  insight_source_id  field  default_value
```

## Insert-only

The loader anti-joins each line against the live table on the business key —
`(tenant_id, insight_source_id, data_source, field, source_key)` for the map,
`(tenant_id, insight_source_id, field)` for the defaults — and writes only the
keys with no stored decision. A key that already carries one — including a
retraction (`is_deleted = 1`) — is left untouched, so editing a line here does
NOT change a decision that is already loaded, and re-deploying is a no-op.

That is deliberate. Both tables are ReplacingMergeTrees whose `unique_key`
includes `valid_from` and `recorded_at`, so a re-INSERT does not collide with
the stored row: it becomes a newer decision and silently wins. These files are
a seed for undecided keys only.

To correct a decision that is already loaded, INSERT the correction directly
against ClickHouse with a real `valid_from` / `recorded_at` — the journal keeps
both, and this loader never touches either.
