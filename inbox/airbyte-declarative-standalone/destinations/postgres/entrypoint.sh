#!/usr/bin/env sh
set -eu

if [ -n "${AIRBYTE_CONFIG:-}" ]; then
  mkdir -p /secrets
  printf '%s' "${AIRBYTE_CONFIG}" > /secrets/config.json
fi

exec "${AIRBYTE_COMMAND}" "$@"
