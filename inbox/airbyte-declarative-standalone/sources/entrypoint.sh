#!/usr/bin/env sh
set -eu

# Support both AIRBYTE_SOURCE_CONFIG (new) and AIRBYTE_CONFIG (legacy)
config_value="${AIRBYTE_SOURCE_CONFIG:-${AIRBYTE_CONFIG:-}}"
if [ -n "${config_value}" ]; then
  mkdir -p /secrets
  printf '%s' "${config_value}" > /secrets/config.json
fi

command_name="${AIRBYTE_COMMAND:-source-declarative-manifest}"
exec "${command_name}" "$@"
