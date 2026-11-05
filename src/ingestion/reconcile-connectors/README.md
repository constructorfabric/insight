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

## Connector schedules

Connector schedules use API > Secret annotation > descriptor schedule > default
precedence. The API key is the exact `(connector, source_id)` pair; a missing
entry falls through to the other tiers. Applied origins are `api`, `annotation`,
`descriptor` and `default`; the sweep preserves the applied origin in the sync history.

Set `CONNECTOR_SCHEDULES_ENABLED=true` to enable the API tier. All other values,
including unset, leave it off. When enabled, all of these must be non-empty:

- `CONNECTOR_SCHEDULES_URL`: the complete analytics schedules endpoint URL.
- `SERVICE_TOKEN_URL`: the complete authenticator token endpoint URL and assertion audience.
- `SERVICE_TOKEN_NAME`: the registered service name, with role `connector-schedules-reader`.
- `SERVICE_TOKEN_KEY_FILE`: a PEM EC P-256 private key, in PKCS#8 or SEC1 format.
- `SERVICE_TOKEN_TENANT_ID`: the tenant UUID for the token request.

Reconcile and adopt read the API once per run, including scoped runs and dry
runs, before applying any CronWorkflow. Each HTTP call has a 5 second connection
limit and a 15 second total limit, without retries or redirects. Node's built-in
crypto signs a unique ES256 assertion with a 50 second lifetime; Python's standard
library and curl handle the exchange without additional packages.

The whole schedules response must validate before any entry is used. A failed
read logs one ERROR with its reason, skips schedule resolution and apply,
continues other work and returns non-zero at the end. Cleanup still removes
CronWorkflows for deleted instances and connectors.
Dry runs still read the API and report the schedule origin they would apply.
