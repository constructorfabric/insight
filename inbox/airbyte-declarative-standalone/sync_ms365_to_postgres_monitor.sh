#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "${SCRIPT_DIR}"

./source.sh read \
  --manifest=./connectors/ms365/ms365.yaml \
  --catalog=./connections/ms365-constructor/configured_catalog.json \
  --state=./connections/ms365-constructor/state.json \
  --env=./connections/ms365-constructor/.env.local \
  | ./destination.sh postgres \
      --namespace=_raw_ms365 \
      --catalog=./connections/ms365-constructor/configured_catalog.json \
      --env=./connections/ms365-constructor/.env.local
