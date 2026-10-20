#!/usr/bin/env bash
set -euo pipefail

CONFIG_FILE="${1:?usage: seed-connectors.sh <connectors-config.yaml> [connector...]}"
shift
# Named connectors only, or all of them. The dbt transform suites take one: they
# need that connector's bronze and nothing else, and the full set is 23 image
# runs they would wait on for no assertion.
WANTED=("$@")

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTORS_DIR="$(cd "${SCRIPT_DIR}/../../connectors" && pwd)"

# shellcheck source=lib/connector-catalog.sh
source "${SCRIPT_DIR}/lib/connector-catalog.sh"

# `${a[@]+"${a[@]}"}` rather than `"${a[@]}"`: bash 3.2 (the macOS /bin/bash a
# developer may still be on) calls an empty array unbound under `set -u`.
wanted() {
  local name="$1" candidate
  (( ${#WANTED[@]} == 0 )) && return 0
  for candidate in ${WANTED[@]+"${WANTED[@]}"}; do
    [[ "${candidate}" == "${name}" ]] && return 0
  done
  return 1
}

for name in ${WANTED[@]+"${WANTED[@]}"}; do
  [[ "$(yq -r ".connectors | has(\"${name}\")" "${CONFIG_FILE}")" == "true" ]] ||
    { echo "no connector named ${name} in ${CONFIG_FILE}" >&2; exit 1; }
done

FAILED=()
while IFS= read -r name; do
  wanted "${name}" || continue
  path="$(connector_path "${CONFIG_FILE}" "${name}")"
  config_json="$(mktemp)"
  if connector_config_json "${CONFIG_FILE}" "${name}" > "${config_json}" \
      && "${SCRIPT_DIR}/create-connector-tables.sh" "${CONNECTORS_DIR}/${path}" "${config_json}"; then
    echo "[${name}] OK"
  else
    echo "[${name}] FAILED, continuing with the next connector" >&2
    FAILED+=("${name}")
  fi
  rm -f "${config_json}"
done < <(connector_names "${CONFIG_FILE}")

# Every connector is attempted before this point so one run reports every
# failure. Exiting non-zero is what stops bootstrap-db.sh from running dbt over
# a partial bronze layer, where the primary error would resurface as one
# UNKNOWN_DATABASE per downstream model.
if (( ${#FAILED[@]} > 0 )); then
  echo "failed connectors: ${FAILED[*]}" >&2
  exit 1
fi
