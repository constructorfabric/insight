#!/usr/bin/env bash
set -euo pipefail

CONFIG_FILE="${1:?usage: seed-connectors.sh <connectors-config.yaml>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTORS_DIR="$(cd "${SCRIPT_DIR}/../../connectors" && pwd)"

FAILED=()
while IFS= read -r name; do
  connector_path="$(yq -r ".connectors.\"${name}\".path" "${CONFIG_FILE}")"
  config_json="$(mktemp)"
  if yq -o=json ".connectors.\"${name}\".config" "${CONFIG_FILE}" \
      | jq 'with_entries(.value =
          (if .value | has("env")
           then (env[.value.env] // error(.value.env + " is not set"))
           else .value.value
           end))' \
      > "${config_json}" \
      && "${SCRIPT_DIR}/create-connector-tables.sh" "${CONNECTORS_DIR}/${connector_path}" "${config_json}"; then
    echo "[${name}] OK"
  else
    echo "[${name}] FAILED, continuing with the next connector" >&2
    FAILED+=("${name}")
  fi
  rm -f "${config_json}"
done < <(yq -r '.connectors | keys | .[]' "${CONFIG_FILE}")

if (( ${#FAILED[@]} > 0 )); then
  echo "failed connectors: ${FAILED[*]}" >&2
fi
