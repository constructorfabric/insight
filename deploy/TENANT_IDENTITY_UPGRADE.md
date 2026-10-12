# Tenant identity: install and upgrade runbook

> **Status: draft.** Tracks [#3661](https://github.com/constructorfabric/insight/issues/3661).
> Steps marked *(lands with #3661)* describe behaviour that is not released yet;
> do not run this runbook against an install until the release notes say it is complete.

Insight identifies a tenant by one canonical UUID. It is the value in every data
tenant column, in identity, in authorization and in analytics filtering.

An install that ingested data before the canonical UUID was introduced may have
written a different value — a *legacy tenant key* — into Bronze record keys
(`unique_key`) and into Airbyte source, connection and schedule names. Those
keys and names are identities: Airbyte finds a source by its exact name, and
ReplacingMergeTree collapses rows by `unique_key`. Rewriting them would orphan
sync state and duplicate history, so a warm install keeps them and declares the
legacy key once.

| Value | Chart key | Used for |
|---|---|---|
| Canonical tenant UUID | `ingestion.reconcile.tenantId` | tenant columns, identity, authorization, filtering |
| Legacy tenant key | `ingestion.reconcile.legacyTenantKeyOverride` | Bronze `unique_key` prefix, Airbyte source/connection names, Argo schedule names — on warm installs only |

**Fresh installs never set `legacyTenantKeyOverride`.** Without it, keys and names
use the canonical UUID.

---

## 1. Decide which path applies

Run against the install's ClickHouse:

```sql
SELECT database, table
FROM system.columns
WHERE database LIKE 'bronze_%' AND name = 'tenant_id'
```

Then, for each table found:

```sql
SELECT tenant_id, count() FROM <database>.<table> GROUP BY tenant_id
```

- **No Bronze tables, or no rows** → follow [§2 Fresh install](#2-fresh-install).
- **Rows whose `tenant_id` is not the canonical UUID** → follow [§3 Warm install](#3-warm-install).
  The value they carry is the legacy tenant key. It must also be the tenant segment
  of the existing Airbyte source names (`<connector>-<source-id>-<legacy key>`).
- **Rows already carry only the canonical UUID** and Airbyte source names contain it
  → nothing to migrate; follow §2 for the values change only.

If Bronze carries more than one distinct non-canonical value, stop: the install has
data under several tenants and needs a separate plan.

## 2. Fresh install

1. Generate a lowercase UUID and set it once:
   ```yaml
   ingestion:
     reconcile:
       tenantId: "<TENANT_UUID>"
   ```
   Do not set `legacyTenantKeyOverride`.
2. Deploy the chart as described in [HELM_DEPLOY.md](HELM_DEPLOY.md).
3. Verify with [§4](#4-verification).

## 3. Warm install

Plan a maintenance window: syncs are paused for its duration, and dbt rebuilds
every derived layer.

### 3.1 Preflight

1. Record the canonical UUID (`<TENANT_UUID>`) and the legacy key (`<LEGACY_KEY>`) found in §1.
2. Take backups:
   - ClickHouse: every `bronze_*` database and the `config` database;
   - the Airbyte database (sources, connections, connection state);
   - the identity MariaDB database.
3. Record the current state for comparison after the upgrade:
   - Airbyte: source ids and names, connection ids and names, connection state;
   - Argo: CronWorkflow names in the app namespace;
   - ClickHouse: row count and `uniqExact(unique_key)` of each Bronze table.

### 3.2 Stop writers

1. Suspend the reconcile loop and every sync schedule:
   ```bash
   kubectl -n <app-namespace> get cronworkflows -o name \
     | xargs -I{} kubectl -n <app-namespace> patch {} --type merge -p '{"spec":{"suspend":true}}'
   ```
2. Wait until no sync workflow is running:
   ```bash
   kubectl -n <app-namespace> get workflows --field-selector status.phase=Running
   ```

### 3.3 Change values and deploy

```yaml
ingestion:
  reconcile:
    tenantId: "<TENANT_UUID>"
    legacyTenantKeyOverride: "<LEGACY_KEY>"
```

Remove `global.tenantDefaultId` from the values, or set it to the same UUID: two
different values fail the render.

Operator-authored config bundles (the TSV files the `clickhouse-field-value-map`
hook loads) carry a `tenant_id` column. Change it from `<LEGACY_KEY>` to
`<TENANT_UUID>` in the same change: the loader skips keys that already carry a
decision, and after §3.4 the stored decisions are under `<TENANT_UUID>`.

Deploy the chart version that contains the tenant identity change. Reconcile
keeps every Airbyte source, connection and schedule name, because they are
built from `legacyTenantKeyOverride`.

### 3.4 Migrate data

1. Rewrite the tenant column of every relation a dbt full refresh cannot
   reproduce: each `bronze_*`, `config` and `ingestion_history` table, and the
   dbt models tagged `history` — snapshots and append-only models, listed in
   `src/ingestion/scripts/tenant-history-tables.txt`. `unique_key` is not
   touched. Preview first, then apply:
   ```bash
   export CLICKHOUSE_URL=<url> CLICKHOUSE_USER=<user> CLICKHOUSE_PASSWORD=<password>
   src/ingestion/scripts/migrate-tenant-identity.sh \
     --canonical-tenant <TENANT_UUID> --legacy-tenant-key <LEGACY_KEY> --dry-run
   src/ingestion/scripts/migrate-tenant-identity.sh \
     --canonical-tenant <TENANT_UUID> --legacy-tenant-key <LEGACY_KEY>
   ```
   The script refuses, changing nothing, when a table holds a tenant that is
   neither of the two. It prints the rows it moved per table and the tables it
   skipped for having no `tenant_id` column. A second run reports zero rows.
2. Rebuild every other derived relation with a dbt full refresh, leaving the
   history models out — a full refresh rebuilds a snapshot from the current
   Bronze state and discards every earlier version:
   ```bash
   export INSIGHT_TENANT_ID=<TENANT_UUID> INSIGHT_TENANT_KEY=<LEGACY_KEY>
   dbt run --full-refresh --exclude tag:history
   ```
   The two variables keep the keys history models build on the legacy key
   (`dbt/macros/tenant_key.sql`); the chart passes them to every dbt run.
3. Re-run identity seeding once, so persons resolve from the identity inputs the
   full refresh rebuilt under the canonical tenant:
   ```bash
   kubectl -n <app-namespace> create job --from=cronjob/<release>-identity-resolution-seed tenant-identity-seed
   ```
   The seed reads only the inputs of the configured tenant, and refuses to run
   when there are none — the sign that Bronze or the identity inputs still
   carry the legacy key.

### 3.5 Resume

1. Unsuspend the schedules suspended in §3.2.
2. Let one reconcile tick run and check its log for created sources or connections:
   on a warm install there must be none.

### 3.6 Rollback

Before §3.4: revert the values and redeploy; nothing in the data has changed.

After §3.4: restore the ClickHouse and identity backups taken in §3.1, revert the
values, redeploy, and unsuspend.

## 4. Verification

- [ ] Every Bronze and config table holds only `<TENANT_UUID>` in its tenant column.
- [ ] Every Bronze table keeps the row count and `uniqExact(unique_key)` recorded in §3.1.
- [ ] Staging, Silver and Gold carry only `<TENANT_UUID>`.
- [ ] Airbyte source and connection ids are unchanged, and connection state matches §3.1.
- [ ] No new Airbyte sources or connections exist, and no CronWorkflow appeared or disappeared.
- [ ] Sources' `insight_tenant_id` configuration is `<TENANT_UUID>`.
- [ ] A signed-in user sees their own metrics; a user of another tenant sees none of them.
