# Airbyte Declarative Standalone

Run Airbyte declarative sources and destinations without the full Airbyte platform — just Docker and shell scripts.

## Directory Structure

```
.
├── source.sh                   # Run a declarative source connector
├── destination.sh              # Run a destination connector
├── sources/
│   ├── Dockerfile              # Builds local source image
│   └── entrypoint.sh           # Docker entrypoint: writes config JSON to /secrets/
├── destinations/
│   └── postgres/
│       ├── Dockerfile          # Builds local destination-postgres image
│       ├── entrypoint.sh       # Docker entrypoint: writes config JSON to /secrets/
│       └── example.env.local   # Template for destination credentials
├── connectors/
│   └── ms365/
│       ├── ms365.yaml          # Declarative manifest
│       └── example.env.local   # Template for source credentials
└── connections/
    └── ms365-constructor/
        ├── .env.local          # Source + destination credentials (gitignored)
        ├── catalog.json        # Raw output from `discover` (reference only)
        ├── configured_catalog.json  # Which streams to sync and how
        └── state.json          # Incremental cursor state (updated after each sync)
```

## Prerequisites

- Docker (with access to pull from Docker Hub)
- Bash
- Python 3

## Setup

### 1. Credentials

Each connection has a single `.env.local` that holds both source and destination config:

```env
AIRBYTE_SOURCE_CONFIG={"client_id":"...","client_secret":"...","tenant_id":"..."}
AIRBYTE_DESTINATION_CONFIG={"host":"...","port":5432,"database":"...","username":"...","password":"...","ssl_mode":{"mode":"disable"},"raw_data_schema":"airbyte_internal"}
```

Copy the example and fill in values:

```bash
cp connectors/ms365/example.env.local connections/ms365-constructor/.env.local
```

> Config values must be valid JSON on a single line.
> Both variables can also be set as environment variables directly — `--env` is optional.

### 2. Docker images

Images are built automatically on first run. To build manually:

```bash
# Source
docker build -t airbyte/source-declarative-manifest:local sources/

# Destination (postgres)
docker build -t airbyte/destination-postgres:local destinations/postgres/
```

## Usage: source.sh

```
./source.sh <command> --manifest=<file> [--env=<file>] [options]
```

| Argument | Description |
|---|---|
| `<command>` | One of: `check`, `discover`, `read` |
| `--manifest=<file>` | Path to the declarative connector manifest |
| `--env=<file>` | Optional. Load env file before reading `AIRBYTE_SOURCE_CONFIG` |

### Check connection

```bash
./source.sh check \
  --manifest=./connectors/ms365/ms365.yaml \
  --env=./connections/ms365-constructor/.env.local
```

### Discover available streams

```bash
./source.sh discover \
  --manifest=./connectors/ms365/ms365.yaml \
  --env=./connections/ms365-constructor/.env.local
```

Save the output to `connections/<name>/catalog.json` for reference. Use it to build `configured_catalog.json`.

### Read data

```bash
./source.sh read \
  --manifest=./connectors/ms365/ms365.yaml \
  --catalog=./connections/ms365-constructor/configured_catalog.json \
  --state=./connections/ms365-constructor/state.json \
  --env=./connections/ms365-constructor/.env.local
```

Outputs a stream of JSON messages to stdout (`RECORD`, `STATE`, `TRACE`).

## Usage: destination.sh

```
./destination.sh <destination> <command> --namespace=<ns> [--env=<file>] [options]
```

| Argument | Description |
|---|---|
| `<destination>` | Destination type. Currently supported: `postgres` |
| `<command>` | `check` or omit for `write` (reads from stdin) |
| `--namespace=<ns>` | Target schema name in the destination database |
| `--catalog=<file>` | Required for `write`. Path to `configured_catalog.json` |
| `--env=<file>` | Optional. Load env file before reading `AIRBYTE_DESTINATION_CONFIG` |

### Check connection

```bash
./destination.sh postgres check \
  --namespace=_raw_ms365 \
  --env=./connections/ms365-constructor/.env.local
```

### Write data (reads from stdin)

```bash
./source.sh read ... | ./destination.sh postgres \
  --namespace=_raw_ms365 \
  --catalog=./connections/ms365-constructor/configured_catalog.json \
  --env=./connections/ms365-constructor/.env.local
```

## Full sync pipeline

```bash
./source.sh read \
  --manifest=./connectors/ms365/ms365.yaml \
  --catalog=./connections/ms365-constructor/configured_catalog.json \
  --state=./connections/ms365-constructor/state.json \
  --env=./connections/ms365-constructor/.env.local \
  | ./destination.sh postgres \
      --namespace=_raw_ms365 \
      --catalog=./connections/ms365-constructor/configured_catalog.json \
      --env=./connections/ms365-constructor/.env.local
```

## Setting up a new connection

1. Run `discover` and save the output to `connections/<name>/catalog.json`
2. Create `connections/<name>/configured_catalog.json` — select streams and set sync modes
3. Create `connections/<name>/state.json` with `{}` for the first run
4. Create `connections/<name>/.env.local` with source and destination credentials

### configured_catalog.json format

```json
{
  "streams": [
    {
      "stream": { ...stream object from catalog... },
      "sync_mode": "incremental",
      "cursor_field": ["<cursor_field>"],
      "destination_sync_mode": "append_dedup",
      "primary_key": [["<primary_key_field>"]]
    }
  ]
}
```

Use `full_refresh` / `overwrite` for streams that don't support incremental.

## Environment variables

### Source

| Variable | Description |
|---|---|
| `AIRBYTE_SOURCE_CONFIG` | Source connector config as a JSON string |
| `AIRBYTE_SOURCE_CONNECTOR_IMAGE` | Override local image name (default: `airbyte/source-declarative-manifest:local`) |
| `AIRBYTE_SOURCE_BASE_CONNECTOR_IMAGE` | Override base image for build (default: `airbyte/source-declarative-manifest:latest`) |
| `AIRBYTE_SOURCE_COMMAND` | Override connector command (default: `source-declarative-manifest`) |

### Destination

| Variable | Description |
|---|---|
| `AIRBYTE_DESTINATION_CONFIG` | Destination connector config as a JSON string (without `schema`) |
| `AIRBYTE_DESTINATION_POSTGRES_IMAGE` | Override local image name (default: `airbyte/destination-postgres:local`) |
| `AIRBYTE_DESTINATION_POSTGRES_BASE_IMAGE` | Override base image for build (default: `airbyte/destination-postgres:latest`) |
| `AIRBYTE_WRITE_MESSAGE_TYPES` | Message types forwarded to destination (default: `RECORD,STATE`) |
