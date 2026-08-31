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

## The sync-history sweep

The tick's last layer copies the mover's account of every sync into
`ingestion_history.sync_events`, which is what the Connector health page reads.
`lib/sweep.sh` gathers — connector to connection map, token — and
`python3 -m sweep` plans and writes.

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
