#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTORS_DIR="$(cd "${SCRIPT_DIR}/../../connectors" && pwd)"
PATTERN="${1:-*}"

FRAGMENTS="$(mktemp)"
trap 'rm -f "${FRAGMENTS}"' EXIT

for descriptor in "${CONNECTORS_DIR}"/*/*/descriptor.yaml; do
  connector_path="${descriptor#"${CONNECTORS_DIR}"/}"
  connector_path="${connector_path%/descriptor.yaml}"
  name="$(yq -r '.name' "${descriptor}")"
  [[ "${connector_path}" == ${PATTERN} || "${name}" == ${PATTERN} ]] || continue

  connector_dir="${CONNECTORS_DIR}/${connector_path}"
  connector_type="$(yq -r '.type // "nocode"' "${descriptor}")"
  if [[ "${connector_type}" == "cdk" ]]; then
    spec="$(jq '.connectionSpecification' "${connector_dir}"/source_*/spec.json)"
  else
    spec="$(yq -o=json '.spec.connection_specification' "${connector_dir}/connector.yaml")"
  fi
  descriptor_required="$(yq -o=json '.secret.required_fields // []' "${descriptor}")"

  jq -n \
    --arg name "${name}" \
    --arg path "${connector_path}" \
    --argjson spec "${spec}" \
    --argjson extra "${descriptor_required}" \
    '
    def fake:
      if .default != null then .default
      elif (.examples | type) == "array" and (.examples | length) > 0 then .examples[0]
      elif .format == "date" then "2020-01-01"
      elif .format == "date-time" then "2020-01-01T00:00:00Z"
      else
        (if (.type | type) == "array" then .type[0] else .type end) as $t
        | if $t == "array" then ["fake"]
          elif $t == "integer" or $t == "number" then 1
          elif $t == "boolean" then false
          elif $t == "object" then {}
          else "fake"
          end
      end;
    (($spec.required // []) + $extra | unique) as $fields
    | {($name): {
        path: $path,
        config: ($fields | map({key: ., value: {value: (($spec.properties[.] // {}) | fake)}}) | from_entries)
      }}
    ' >> "${FRAGMENTS}"
done

jq -s 'add // {} | {connectors: .}' "${FRAGMENTS}" | yq -P
