# reconcile-connectors

Single entrypoint for the Airbyte connector reconcile loop.

## Usage

```
bash src/ingestion/reconcile-connectors/main.sh [adopt | reconcile (default)]
                                                [--connector <name>]
                                                [--dry-run]
                                                [--no-gc]
                                                [--no-sync-trigger]
```

## Folder map

```
src/ingestion/reconcile-connectors/
├── main.sh                  CLI dispatch
├── lib/                     sourceable libs (no top-level CLI)
├── python/                  pure-python helpers (CLI via argparse)
│   └── sweep/               the sync-history sweep (reads stdin, not argv)
└── templates/               Argo/K8s YAML templates
```

## The Bronze destination

Reconcile resolves the `Clickhouse` destination definition by name, so an
installation runs whatever `airbyte/destination-clickhouse` version its Airbyte
carries. Below the minimum named in `python/check_destination_version.py`
(2.1.29, the release that added `use_replicated_engines` / `cluster_name`) the
tick refuses rather than creating the destination, and the log line says which
version was found. Pinning the definition instead would fight the operator's
own Airbyte upgrades.

`RECONCILE_DESTINATION_ID` hands that destination's configuration to whoever
set it and skips the lookup; the version floor still applies, because the
catalogues reconcile hands over run on that destination either way.

## A tenant change

Sources, connections and CronWorkflows are named `{connector}-{source_id}-{tenant}`
(`-conn`, and a bounded `-sync` form for the schedule). When the install's tenant
changes, reconcile renames each instance's source and connection in place —
same ids, so the connection keeps its sync state — and removes the schedule named
for the former tenant. The former tenant is read from the source's own
`insight_tenant_id`; an instance with two candidates, or with its new name
already taken, is left for an operator. `tests/live` proves it against a real
Airbyte.

## The sync-history sweep

The tick's last layer copies the mover's account of every sync into
`ingestion_history.sync_events`, which is what the Connector health page reads.
`lib/sweep.sh` gathers — connector to connection map, token — and
`python3 -m sweep` plans and writes.

Each `connector.configured` row carries `schedules`, `schedule_timezone`,
`schedule_origin` and `schedule_suspended` from its live CronWorkflow's
`spec.schedules`, `spec.timezone`, `insight.cyberfabric.com/schedule-origin`
annotation and `spec.suspend`. The sweep lists connector
CronWorkflows once per tick and matches each instance by the name reconcile uses.
Missing or unreadable CronWorkflows leave the expressions, timezone and origin empty;
absent timezone and origin fields are empty too. Suspension defaults to false when absent,
unreadable or missing. These values are never recomputed from desired inputs.

Reconcile writes `spec.timezone: UTC` on every connector CronWorkflow, so the
recorded cron expressions read in UTC. A CronWorkflow applied before this change
may lack a timezone; the sweep records it as empty, meaning controller time.

It cannot fail a tick: `sweep_run` always returns 0, so a broken recorder costs
the page its freshness and costs reconciliation nothing. It adds no environment
of its own, reusing the `RECONCILE_DEST_CLICKHOUSE_*` credentials the Bronze
destination already needs.

Spec: `docs/components/backend/analytics/specs/connector-health`.

## The transform planner

Helm starts the `<fullname>-transform` hook Workflow after the release's
ClickHouse migrations on each install and upgrade, and also starts it on rollback
from the target revision's hooks. A failed migration stops the hook chain before
the transform Workflow is created. Helm returns without waiting for the Workflow
to finish. The next install, upgrade or rollback replaces a run still in progress.
Both the Workflow and its WorkflowTemplate are gated by
`ingestion.templates.enabled` and `ingestion.transformPlan.enabled`.

The `<fullname>-transform` WorkflowTemplate runs `transform-plan.sh`, which
executes `python3 -m transform_plan`. Argo retries failed attempts using
`ingestion.transformPlan.retry`: by default, at most 3 attempts, waiting 15 minutes
between attempts. Configure the attempt limit with `attempts`
and the fixed interval with `wait`. Each attempt has a
900-second deadline; there is no Workflow-level deadline. Succeeded Workflows
remain for a day; failed Workflows have no TTL and the next release replaces them.
Container resources come from `ingestion.transformPlan.resources`.

Start a run by hand in the release's namespace:

```bash
argo submit --from workflowtemplate/<fullname>-transform
```

All transform runs share the `<fullname>-transform` mutex, so manual and hook
runs cannot overlap. The planner takes no reconcile mutex and uses the
`argo-workflow` ServiceAccount. Planning remains independent of connector Secret
reads and Airbyte availability.

It parses the image's dbt project with a temporary
profile from `scripts/dbt_profiles.py`, then compares effective
`meta.transform_version` values with the latest rows in
`ingestion_history.transform_versions`. The table is append-only, migration-owned,
and has no TTL: `argMax(transform_version, applied_at)` selects each model's
applied version. Rows hold the dbt unique ID, resolved database/schema/alias,
version, optional release, application time and application identity.

An empty table receives one `applied_by=baseline` row per deployed model, with
no rebuild planned. The release comes from `INSIGHT_VERSION`, set to
`.Chart.Version` by the transform planner container; an unset variable is recorded as
NULL. Subsequent passes report version differences in
either direction, new and removed models, and the changed/new models plus their
model descendants. Removed models are reported only. Models whose effective
`full_refresh` is false are listed separately as incremental-only.

The plan's connector locks follow model parents to bronze sources, then use
the declaring connector directory's descriptor `name`. Other sources, including
`config`, require no lock. The planner trusts dbt's graph: reads hidden behind
`adapter.get_relation` are not represented.

The shared Python logger emits one `transform.planned` event with names and
counts for `baseline`, `changed`, `new`, `removed`, `rebuild`, `incremental_only`
and `connectors`, correlated by `RECONCILE_RUN_ID` set to the Workflow name.
`transform-plan.sh --dry-run` reports without
writing a baseline. Planning reuses the sweep's ClickHouse transport and existing
`RECONCILE_DEST_CLICKHOUSE_*` credentials and consistency settings. Failures emit
`transform.failed` and fail the attempt with the planner's exit code; Argo retries
it until the attempt limit is reached, then leaves the Workflow failed.
This planner submits no Workflow, runs no `dbt run`, and writes only a baseline.

## Environment variables

| Var | Default | Purpose |
|-----|---------|---------|
| AIRBYTE_API_URL | — | Airbyte server URL (in-cluster) |
| INSIGHT_NAMESPACE | `insight` | Namespace for K8s Secrets + CronWorkflows |
| INSIGHT_RECONCILE_TOKEN_TTL | `600` | Airbyte API token cache TTL (seconds) |
| RECONCILE_RUN_ID | — | Correlation id stamped on every log line (the chart injects the workflow pod name) |
| RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE | `false` | Warehouse topology (`clickhouse.clusterMode`): `compose_destination_config.py` sets the destination's `use_replicated_engines`; the sweep's `ledger.py` then writes with quorum session settings and reads the catalogue through `clusterAllReplicas` |
| RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME | — | The cluster the destination's `ON CLUSTER` and the sweep's `clusterAllReplicas` name; required when `_CLUSTER_MODE` is on, refused empty |

## Logging

Structured JSON to stdout, one object per line (`lib/log.sh`): fields
`ts`, `level`, `component:"reconcile"`, `msg`, optional `event` and
`run_id`. The cluster's log collector (Alloy → Loki) is the durable
destination — there is no file/PVC logging anymore.

Lifecycle events, emitted on EVERY run (including no-op ticks, so a
missing event means the loop did not run):

- `reconcile.started`  — tenant, subcommand, dry_run, connector scope
- `reconcile.completed` — status, changes, errors, duration_ms
- `reconcile.failed`   — abnormal abort (set -e path), exit_code

Find one tick in Loki: `{namespace="insight"} | json | run_id="<pod>"`.
