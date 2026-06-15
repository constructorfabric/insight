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
└── templates/               Argo/K8s YAML templates
```

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
