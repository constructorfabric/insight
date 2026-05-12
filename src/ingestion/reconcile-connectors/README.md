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
| XDG_STATE_HOME | — | Local log dir parent (defaults to `$HOME/.local/state`) |

## Log destinations

- In-cluster: `/var/log/insight/reconcile-${YYYY-MM-DD}.log` on PVC `insight-reconcile-logs`
- Local: `${XDG_STATE_HOME:-$HOME/.local/state}/insight/reconcile-${YYYY-MM-DD}.log`

Quiet runs emit ZERO file lines and ONE stdout summary line.
