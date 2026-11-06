# Moving an install to the canonical tenant UUID

Applies to the first release that contains
[#3661](https://github.com/constructorfabric/insight/issues/3661). From that
release on, an install has one tenant: the lowercase UUID in
`authenticator.oidc.defaultTenantId`. Every tenant column, every stored key,
identity resolution, sign-in and analytics filtering use it, and analytics
filters every read to the session tenant; there is no switch to turn that off.
A Bronze row without a tenant still builds: keys read it as an empty tenant,
and the `not_null` and `not_empty` dbt tests on each model's tenant report it in
the sync's `connector-checks` step without failing the sync. Identity
resolution refuses it: the identity inputs fail to build over a tenant that is
not a UUID.

A **fresh install** only sets that value — see [Fresh install](#fresh-install).
An install that **already holds data** wrote its Bronze, history relations,
operator config and sync ledger under the value of the removed
`ingestion.reconcile.tenantId`, often a string that is not a UUID, and named its
Airbyte sources, connections and Argo schedules after it. It moves once, in one
maintenance window, by following [Upgrading an install that holds
data](#upgrading-an-install-that-holds-data).

## Contents

- [Fresh install](#fresh-install)
- [Upgrading an install that holds data](#upgrading-an-install-that-holds-data)
  - [What changes](#what-changes)
  - [1. Decide the two values](#1-decide-the-two-values)
  - [2. Prepare the migration tool](#2-prepare-the-migration-tool)
  - [3. Stop every writer](#3-stop-every-writer)
  - [4. Back up](#4-back-up)
  - [5. Change the values and deploy](#5-change-the-values-and-deploy)
  - [6. Migrate the stored tenant](#6-migrate-the-stored-tenant)
  - [7. Rebuild everything else](#7-rebuild-everything-else)
  - [8. Resume](#8-resume)
  - [9. Verify](#9-verify)
  - [10. Finalize](#10-finalize)
- [Rolling back](#rolling-back)

## Fresh install

Set the tenant your IdP asserts in the `tenant_id` claim (the claim named by
`authenticator.oidc.tenantClaim`), lowercase:

```yaml
authenticator:
  oidc:
    defaultTenantId: "<TENANT_UUID>"
```

When the IdP asserts no tenant, generate a UUID as described in
[HELM_DEPLOY.md](HELM_DEPLOY.md#generate-the-tenant-id). Do not set
`global.tenantDefaultId`, `ingestion.reconcile.tenantId` or
`identityResolution.seed.tenantDefaultId`: the chart refuses to render with any
of them. Nothing else in this document applies.

## Upgrading an install that holds data

### What changes

| Where | Changed by | Kept as a way back |
|---|---|---|
| `bronze_*`, the `tag:history` relations (`src/ingestion/scripts/tenant_migration/history-tables.txt`), `config.*`, `ingestion_history.*` — tenant column and the tenant segment of `unique_key` | `tenant_migration apply` (step 6) | the original of each table until `finalize`, and the tool's dump |
| every other ClickHouse relation dbt owns (staging, silver, gold, `identity.identity_inputs`) | dbt full refresh (step 7) | full ClickHouse backup (step 4) |
| `silver.class_task_*` and their staging producers — a `tenant_id` column after `unique_key` | the deploy's heals (step 5) | full ClickHouse backup (step 4) |
| identity map views `identity.person_map` and `identity.account_assignment` | dropped by the deploy when they lack `tenant_id` (step 5), recreated by the rebuild (step 7) | nothing to keep: views hold no rows |
| identity MariaDB `persons` journal | the identity seed appends to it (step 7) | identity MariaDB backup (step 4) |
| Airbyte source and connection names, the source's `insight_tenant_id`; Argo sync schedules | reconcile renames in place on its first tick (step 8) | Airbyte database backup, exported CronWorkflows (step 4) |

The identity and analytics MariaDB stores keep their tenant: this runbook moves
nothing there, so the UUID must be the tenant they already use (step 1).

Plan for: syncs stopped for the whole window; analytics answering not-ready, or
reading empty or partial metrics, between the deploy (step 5) and the end of the
rebuild (step 7); and
ClickHouse disk for a second copy of every migrated table until `finalize`.

### 1. Decide the two values

- `<OLD_TENANT>` — the value of `ingestion.reconcile.tenantId` in the values the
  install runs today. It is also the last segment of every Airbyte source name,
  `<connector>-<source_id>-<OLD_TENANT>`.
- `<TENANT_UUID>` — the canonical tenant. It must already be the tenant of both:
  - a signed-in session: sign in, open `https://<insight-host>/auth/me`, read `tenant_id`;
  - the identity database. In the identity MariaDB:

    ```sql
    SELECT LOWER(HEX(insight_tenant_id)) AS tenant, COUNT(*) FROM persons GROUP BY tenant;
    ```

    exactly one row, equal to `<TENANT_UUID>` without dashes (or no rows at all).

  If either differs, or `persons` holds several tenants, stop: moving the
  MariaDB stores to another tenant is not covered here.

If `<OLD_TENANT>` already equals `<TENANT_UUID>`, skip step 6 (the tool refuses:
there is nothing to move) and do everything else — the previous release stamped
identity inputs with a hashed tenant, which only the rebuild replaces.

### 2. Prepare the migration tool

`tenant_migration` ships in `src/ingestion/scripts` of the release, needs only
Python 3.12+ and ClickHouse's HTTP interface, and writes a dump that has to
survive the window. Run it from a workstation with a checkout of the release
tag:

```sh
cd src/ingestion/scripts
export CLICKHOUSE_URL=http://127.0.0.1:8123        # ClickHouse HTTP, e.g. through kubectl port-forward
export CLICKHOUSE_USER=<clickhouse.username>
export CLICKHOUSE_PASSWORD="$(kubectl -n <NS> get secret insight-db-creds \
  -o jsonpath='{.data.clickhouse-password}' | base64 -d)"
python3 -m tenant_migration plan --tenant <TENANT_UUID> --old-tenant <OLD_TENANT>
```

The user must be able to create, exchange and drop tables in `bronze_*`,
`staging`, `silver`, `config` and `ingestion_history` — the chart's ClickHouse
user, the one the `clickhouse-migrate` hook uses, can. The same package is in
the toolbox image under `/ingestion/scripts`; from a pod, give `--dump-dir` a
mounted volume, never the pod's own filesystem.

`plan` reads only. Run it before the window: it prints every table it would
change with its row count, and a `REFUSED` line, exit code 1, for anything that
blocks the migration:

| Refusal | Meaning |
|---|---|
| `holds another tenant (…) — this install keeps several` | rows of a second tenant: stop, this runbook moves one |
| `key(s) start like a moving tenant but not with it and their source` | keys the tool cannot rewrite safely: stop and report |
| `row(s), and …` (no tenant column, a tenant that is not a string, a key without a source column, a name that is not a plain identifier) | a table in scope it cannot read: stop and report |
| `an earlier run left … — finalize or roll back first` | leftovers of an interrupted run: `rollback` or `finalize` |
| `bronze_jira.…: … keyed by the issue key — deploy this release before migrating` | expected before step 5; it must be gone after it |

### 3. Stop every writer

`apply` requires that nothing writes to the tables it moves. Suspend every Argo
CronWorkflow in the release namespace — the reconcile loop, the data-quality
checks and each connection's sync schedule — and the identity seed CronJob.
Record which were suspended already, so step 8 leaves them so:

```sh
kubectl -n <NS> get cronworkflows -o custom-columns=NAME:.metadata.name,SUSPENDED:.spec.suspend > suspended-before.txt
kubectl -n <NS> get cronworkflows -o name \
  | xargs -I{} kubectl -n <NS> patch {} --type merge -p '{"spec":{"suspend":true}}'
kubectl -n <NS> patch cronjob <release>-identity-resolution-seed --type merge -p '{"spec":{"suspend":true}}'
```

Then wait until no workflow is running and no Airbyte job is in progress:

```sh
kubectl -n <NS> get workflows -l 'workflows.argoproj.io/completed!=true'
```

### 4. Back up

With the writers stopped:

- **ClickHouse** — a full backup of every database of the install, with the
  tool your ClickHouse is backed up with. The migration's own dump covers only
  the tables it rewrites; a rollback after the rebuild needs everything.
- **Airbyte** — its configuration database, by your Airbyte backup procedure.
- **Argo** — `kubectl -n <NS> get cronworkflows -o yaml > cronworkflows-before.yaml`.
- **Identity MariaDB** — a dump of the identity database.
- **Counts** — keep the output of `plan` from this point: the rows per table to
  compare against in step 9.

### 5. Change the values and deploy

In the install's values:

```yaml
authenticator:
  oidc:
    defaultTenantId: "<TENANT_UUID>"
```

and delete `global.tenantDefaultId`, `ingestion.reconcile.tenantId` and
`identityResolution.seed.tenantDefaultId` — the render fails while any of them
is present. `analytics.metricCatalog.enforceTenantScope` no longer exists;
remove it if the values still set it (it is ignored).

Deploy the release (`helm upgrade`, or sync the GitOps application). Its
`clickhouse-migrate` hook applies the migrations and heals: the task classes
and their producers gain a `tenant_id` column — `String`, or `Nullable(String)`
on a table still holding rows written before it, until step 7 rebuilds that
table — and the identity map views the
previous release built are dropped, because they lack the `tenant_id` analytics
now joins on. It creates and rebuilds no dbt relation — every staging, silver
and gold relation is built by a dbt run — so gold keeps the rows the last sync
wrote under `<OLD_TENANT>`, and anything that reads the identity maps answers
not-ready until step 7.1 recreates them. Metrics read empty or not-ready until
step 7. Then confirm the CronWorkflows and the seed CronJob
are still suspended, and run `plan` again: it must print no `REFUSED` line now.

Operator-authored field-value-map bundles
(`clickhouse.fieldValueMap`) carry a `tenant_id` column. Leave them as they are
for this deploy; change it to `<TENANT_UUID>` after step 6 and before the next
deploy, or that deploy's loader writes the old tenant's rows again.

### 6. Migrate the stored tenant

```sh
python3 -m tenant_migration apply --tenant <TENANT_UUID> --old-tenant <OLD_TENANT> \
  --dump-dir <DUMP_DIR>
```

`<DUMP_DIR>` must not exist yet and must be on disk that outlives the window.
`apply` re-runs the plan and refuses, changing nothing, if anything is refused.
Otherwise it dumps every table it will change — DDL, rows and a
`manifest.json` with each table's row and key counts — then moves each table
by copy, count check and `EXCHANGE`, and prints the rows and keys it holds
afterwards. Rows under `<OLD_TENANT>` and rows with an empty tenant move;
rows already under `<TENANT_UUID>` stay. Each original stays beside its table
as `<table>__tenant_backup` until step 10.

If it stops part-way, run `python3 -m tenant_migration rollback` — it puts every
original back, wherever the run stopped — then `python3 -m tenant_migration
finalize` to drop the set-aside copies, and `apply` again with a new `--dump-dir`.

### 7. Rebuild everything else

Every other relation is rebuilt by a dbt full refresh of the installed
connectors, in the order a sync uses: staging and the identity inputs, the
identity seed, then everything downstream, gold included — the deploy builds no
model, so this rebuild is what moves gold to `<TENANT_UUID>`. History relations
(`tag:history`) refuse a full refresh: they keep their rows and run
incrementally, and one that does not exist yet is created. Like a sync, each
`dbt-run` below also builds the `tag:sync:always` views (the identity map and
`bronze_insert_events`) — the first one recreates the identity map views the
deploy dropped — and leaves out the models whose Bronze the install does not
hold.

List the installed connectors and compose the selections:

```sh
NS=<NS>
ARGO_INSTANCE_ID=<ingestion.reconcile.argoInstanceId, empty if unset>
LABELS=""
[ -n "$ARGO_INSTANCE_ID" ] && LABELS="  labels: {workflows.argoproj.io/controller-instanceid: $ARGO_INSTANCE_ID}"
CONNECTORS=$(kubectl -n "$NS" get cronworkflows -l app.kubernetes.io/component=connector-sync \
  -o jsonpath='{range .items[*]}{.metadata.labels.insight\.cyberfabric\.com/connector}{"\n"}{end}' | sort -u)
STAGING="" DOWNSTREAM=""
for c in $CONNECTORS; do
  if [ "$c" = jira ]; then
    STAGING="$STAGING tag:staging,tag:jira"
    DOWNSTREAM="$DOWNSTREAM tag:silver,tag:jira+ tag:gold,tag:jira+"
  else
    STAGING="$STAGING tag:$c"
    DOWNSTREAM="$DOWNSTREAM tag:$c+"
  fi
done
```

Each step runs the chart's `dbt-run` template and waits for it:

```sh
dbt_run() {  # dbt_run <select> <exclude> <full refresh: true|false>
  name=$(kubectl -n "$NS" create -o name -f - <<EOF
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: tenant-cutover-dbt-
$LABELS
spec:
  workflowTemplateRef:
    name: dbt-run
  arguments:
    parameters:
      - {name: dbt_select, value: "$1"}
      - {name: dbt_exclude, value: "$2"}
      - {name: dbt_full_refresh, value: "$3"}
EOF
)
  while :; do
    phase=$(kubectl -n "$NS" get "$name" -o jsonpath='{.status.phase}')
    case "$phase" in
      Succeeded) echo "$name succeeded"; return 0 ;;
      Failed|Error) echo "$name: $phase"; return 1 ;;
    esac
    sleep 15
  done
}
```

1. Staging and the identity inputs:

   ```sh
   dbt_run "$STAGING identity_inputs" "" true
   ```

2. The identity seed, once, over the rebuilt inputs:

   ```sh
   kubectl -n "$NS" create job --from=cronjob/<release>-identity-resolution-seed tenant-cutover-seed
   kubectl -n "$NS" wait --for=condition=complete job/tenant-cutover-seed --timeout=30m
   kubectl -n "$NS" logs job/tenant-cutover-seed | grep 'input guard'
   ```

   The `grep` must print nothing. The CronJob runs the seed with `--guard-ok`,
   which exits 0 when it refuses; a line naming `identity_inputs returned 0
   rows for tenant <TENANT_UUID>` means the inputs still carry another tenant —
   step 6 or step 7.1 did not take.

3. Everything downstream — each connector's silver and the gold it feeds —
   then every gold model once more, for a gold relation no installed
   connector's selection reaches:

   ```sh
   dbt_run "$DOWNSTREAM" "$STAGING identity_inputs" true
   dbt_run "tag:gold" "" false
   ```

A `dbt-run` workflow is capped at two hours; on an install where a step needs
longer, split its selection by connector.

### 8. Resume

1. Unsuspend the reconcile loop only:

   ```sh
   kubectl -n "$NS" patch cronworkflow <release>-reconcile-loop --type merge -p '{"spec":{"suspend":false}}'
   ```

2. After its next tick, read the tick's log:

   ```sh
   kubectl -n "$NS" get workflows -l workflows.argoproj.io/cron-workflow=<release>-reconcile-loop
   kubectl -n "$NS" logs -l workflows.argoproj.io/workflow=<tick workflow> -c main
   ```

   Each instance logs `renamed source <old name> (tenant <OLD_TENANT>) to <new
   name>, keeping its id and sync state` and `renamed connection …`; its
   schedule named for `<OLD_TENANT>` is removed and one named for
   `<TENANT_UUID>` is applied. A line `cannot carry the source over from its
   former tenant — leaving it for an operator` names an instance reconcile
   left alone: resolve it before going on.

3. Unsuspend the data-quality CronWorkflow and the identity seed CronJob, and
   any other CronWorkflow that was not suspended in `suspended-before.txt`.
   Connection schedules reconcile applied under the new name start unsuspended.

The first sync of Jira and GitHub after the upgrade runs a one-shot full refresh
of those connectors (their descriptors carry a new major version); it is
expected and leaves history relations alone.

### 9. Verify

- [ ] `plan` prints `tables changing: 0` and no `REFUSED` line.
- [ ] Each table `apply` printed holds the row and key counts `plan` printed
      before it (the `manifest.json` in `<DUMP_DIR>` holds the same numbers).
- [ ] Derived relations carry only the canonical tenant: for each
      `insight.*_metric_observations` table, `SELECT DISTINCT tenant_id`
      returns `<TENANT_UUID>` alone, and so does
      `SELECT DISTINCT insight_tenant_id FROM identity.identity_inputs`.
- [ ] A user signs in, `/auth/me` shows `tenant_id` `<TENANT_UUID>`, and their
      dashboards show metrics of every installed connector class.
- [ ] Airbyte: the source and connection ids are the ones recorded in step 4,
      their names end in `<TENANT_UUID>`, and no source or connection was created.
- [ ] Argo: one sync CronWorkflow per connection, none named for `<OLD_TENANT>`.

### 10. Finalize

Once verified, drop the originals `apply` kept:

```sh
python3 -m tenant_migration finalize
```

After this, only `restore` from `<DUMP_DIR>` or the step 4 backups bring the old
state back. Keep them until the install has run normally for a while.

## Rolling back

| Reached | Roll back by |
|---|---|
| step 5 (deployed, nothing migrated) | deploy the previous release with the previous values; unsuspend what step 3 suspended |
| step 6 (migrated, not rebuilt) | `tenant_migration rollback`, then `tenant_migration finalize`; then as above |
| step 7 (rebuilt) | restore the ClickHouse and identity MariaDB backups; then as above |
| step 8 (reconcile renamed Airbyte objects) | additionally, with Airbyte stopped, restore its database and re-apply `cronworkflows-before.yaml`: the previous release finds sources only by their old names and would create new ones |
| step 10 (finalized) | `tenant_migration restore --dump-dir <DUMP_DIR>` puts the migrated tables back as dumped; everything else as for step 7 |

`rollback` swaps the originals kept since `apply` back into place and sets the
rewritten tables aside as `<table>__tenant_rejected`; it works only until
`finalize`. `restore` loads each table from the dump into a fresh relation,
checks it against the counts in `manifest.json`, and swaps it in; it works
before and after `finalize`. Both, and `finalize`, read the state of an
interrupted run from the warehouse and can be run again until they succeed;
`finalize` afterwards drops what they set aside.
