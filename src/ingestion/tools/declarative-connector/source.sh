#!/usr/bin/env bash
set -euo pipefail

# RULE-DEFAULTS-OK: every default below is for the local Docker-based test harness; this file is not used in production deploy.

# ---------------------------------------------------------------------------
# Local declarative-connector runner
#
# Runs Airbyte declarative manifest connectors in Docker without the full
# Airbyte platform. Useful for rapid manifest development and validation.
#
# Usage:
#   ./source.sh validate        <class>/<connector>
#   ./source.sh validate-strict <class>/<connector>
#   ./source.sh check           <class>/<connector> <tenant>
#   ./source.sh discover        <class>/<connector> <tenant>
#   ./source.sh read            <class>/<connector> <tenant>
#
# validate vs validate-strict:
#   - `validate` passes if the CDK loader accepts the manifest at runtime. It is
#     lenient and resolves `$ref` before validation, so it will happily accept
#     manifests that the Airbyte Builder UI rejects.
#   - `validate-strict` runs the manifest through the Airbyte Builder JSON-schema
#     validator (no `$ref` resolution). Use this before attempting to open the
#     manifest in the Builder UI. Emits per-path error messages.
#
# Example:
#   $0 validate        collaboration/m365
#   $0 validate-strict task-tracking/youtrack
#   $0 check           collaboration/m365 example-tenant
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTORS_DIR="${SCRIPT_DIR}/../../connectors"

# Load global overrides (image tags, etc.) if present
GLOBAL_ENV_FILE="${SCRIPT_DIR}/.env.local"
if [[ -f "${GLOBAL_ENV_FILE}" ]]; then
  set -a; source "${GLOBAL_ENV_FILE}"; set +a
fi

IMAGE="${AIRBYTE_CONNECTOR_IMAGE:-airbyte/source-declarative-manifest:local}"  # RULE-DEFAULTS-OK: see file header
BASE_IMAGE="${AIRBYTE_BASE_IMAGE:-airbyte/source-declarative-manifest:latest}"  # RULE-DEFAULTS-OK: see file header
COMMAND_NAME="${AIRBYTE_COMMAND:-source-declarative-manifest}"  # RULE-DEFAULTS-OK: see file header
SECRETS_TMPFS_OPTS="${AIRBYTE_SECRETS_TMPFS_OPTS:-/secrets:rw,mode=1777}"  # RULE-DEFAULTS-OK: see file header

usage() {
  cat >&2 <<EOF
Usage:
  $0 validate        <class>/<connector>
  $0 validate-strict <class>/<connector>
  $0 check           <class>/<connector> <tenant>
  $0 discover        <class>/<connector> <tenant>
  $0 read            <class>/<connector> <tenant>

Commands:
  validate        Runtime validation — passes if the CDK loader accepts the
                  manifest. Resolves \$ref before checking. Lenient.
  validate-strict Strict Builder-UI validation — runs the manifest through the
                  declarative_component_schema.yaml validator WITHOUT \$ref
                  resolution. Use this before opening the manifest in the
                  Airbyte Builder UI. Reports per-path errors.
  check           Manifest + credentials against source API (smoke test)
  discover        List available streams and their schemas
  read            Extract data (outputs Airbyte Protocol JSON to stdout)

Arguments:
  <class>/<connector>  Path relative to connectors/ (e.g. collaboration/m365)
  <tenant>             Tenant name (reads credentials from connections/<tenant>.yaml)

Examples:
  $0 validate        collaboration/m365
  $0 validate-strict task-tracking/youtrack
  $0 check           collaboration/m365 example-tenant
  $0 discover        collaboration/m365 example-tenant
  $0 read            collaboration/m365 example-tenant
EOF
}

# --- Argument parsing ---
command="${1:-}"
connector="${2:-}"

if [[ -z "${command}" || -z "${connector}" ]]; then
  usage
  exit 1
fi

connector_dir="${CONNECTORS_DIR}/${connector}"
manifest_path="${connector_dir}/connector.yaml"
CONNECTIONS_DIR="${SCRIPT_DIR}/../../connections"

# Extract connector short name (last path component)
connector_name="$(basename "${connector}")"

# --- Validate inputs ---
if [[ ! -d "${connector_dir}" ]]; then
  echo "ERROR: Connector directory not found: ${connector_dir}" >&2
  echo "Available connectors:" >&2
  find "${CONNECTORS_DIR}" -name "connector.yaml" -exec dirname {} \; 2>/dev/null | \
    sed "s|${CONNECTORS_DIR}/||" | sort >&2
  exit 1
fi

if [[ ! -f "${manifest_path}" ]]; then
  echo "ERROR: Connector manifest not found: ${manifest_path}" >&2
  exit 1
fi

# --- Load credentials from K8s Secret file + tenant yaml (skip for validate modes) ---
if [[ "${command}" != "validate" && "${command}" != "validate-strict" ]]; then
  tenant="${3:-}"
  SECRETS_DIR="${SCRIPT_DIR}/../../secrets/connectors"

  # Find tenant config (for tenant_id)
  if [[ -z "${tenant}" ]]; then
    # Auto-detect: use first tenant yaml
    for f in "${CONNECTIONS_DIR}"/*.yaml; do
      [[ -f "$f" ]] || continue
      [[ "$(basename "$f")" == *.example ]] && continue
      tenant="$(basename "$f" .yaml)"
      echo "INFO: Auto-detected tenant: ${tenant}" >&2
      break
    done
    if [[ -z "${tenant}" ]]; then
      echo "ERROR: No tenant specified and none found in connections/" >&2
      echo "  Usage: $0 ${command} ${connector} <tenant>" >&2
      exit 1
    fi
  fi

  tenant_file="${CONNECTIONS_DIR}/${tenant}.yaml"
  if [[ ! -f "${tenant_file}" ]]; then
    echo "ERROR: Tenant config not found: ${tenant_file}" >&2
    exit 1
  fi
  tenant_id=$(yq -r '.tenant_id' "${tenant_file}")

  # Build AIRBYTE_CONFIG JSON from K8s Secret file (secrets/connectors/<name>.yaml)
  secret_file="${SECRETS_DIR}/${connector_name}.yaml"
  if [[ ! -f "${secret_file}" ]]; then
    echo "ERROR: Secret file not found: ${secret_file}" >&2
    echo "  Create from template: cp ${SECRETS_DIR}/${connector_name}.yaml.example ${secret_file}" >&2
    exit 1
  fi

  AIRBYTE_CONFIG=$(python3 -c "
import json, yaml, sys

with open('${secret_file}') as f:
    secret = yaml.safe_load(f)

data = secret.get('stringData', {})
# Parse JSON strings (arrays/objects) stored in Secret stringData
for k, v in list(data.items()):
    if isinstance(v, str):
        try:
            parsed = json.loads(v)
            if isinstance(parsed, (list, dict)):
                data[k] = parsed
        except (json.JSONDecodeError, TypeError):
            pass

# Inject platform fields from tenant config and Secret annotations
data['insight_tenant_id'] = '${tenant_id}'
annotations = secret.get('metadata', {}).get('annotations', {})
data['insight_source_id'] = annotations.get('insight.cyberfabric.com/source-id', '${connector_name}-default')

print(json.dumps(data))
")
  if [[ -z "${AIRBYTE_CONFIG:-}" || "${AIRBYTE_CONFIG}" == "null" ]]; then
    echo "ERROR: Could not build config from ${secret_file}" >&2
    exit 1
  fi
fi

# --- Build wrapper image if missing ---
if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
  echo "Building connector image (${IMAGE})..." >&2
  docker build \
    --build-arg "BASE_IMAGE=${BASE_IMAGE}" \
    --build-arg "AIRBYTE_COMMAND=${COMMAND_NAME}" \
    -f "${SCRIPT_DIR}/Dockerfile" \
    -t "${IMAGE}" \
    "${SCRIPT_DIR}" >/dev/null
fi

# --- Execute command ---
case "${command}" in
  validate)
    shift 2
    docker run --rm \
      -e 'AIRBYTE_CONFIG={}' \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${connector_dir}:/input:ro" \
      "${IMAGE}" check \
        --config /secrets/config.json \
        --manifest-path /input/connector.yaml \
        "$@" 2>&1 | {
      valid=true
      while IFS= read -r line; do
        if echo "$line" | grep -q "declarative_component_schema.yaml schema failed"; then
          valid=false
          echo "$line"
        elif echo "$line" | grep -q "ValidationError\|is not valid under"; then
          echo "$line" >&2
        fi
      done
      if $valid; then
        echo '{"type":"LOG","log":{"level":"INFO","message":"Manifest is valid"}}'
      else
        exit 1
      fi
    }
    ;;

  validate-strict)
    # Run the exact jsonschema check the Builder UI performs (no $ref resolution).
    # Logic lives in validate_strict.py (per the no-inline-Python rule in
    # .cf-studio/config/rules/code-conventions.md §"No inline scripts"); this case
    # just mounts the script + the connector dir and invokes it inside the
    # container which ships airbyte_cdk + pyyaml + jsonschema.
    docker run --rm \
      --entrypoint=/bin/sh \
      -v "${connector_dir}:/input:ro" \
      -v "${SCRIPT_DIR}/validate_strict.py:/scripts/validate_strict.py:ro" \
      "${IMAGE}" -c "python3 /scripts/validate_strict.py /input/connector.yaml"
    ;;

  check|discover)
    shift 2
    [[ -n "${tenant:-}" ]] && shift  # remove tenant arg
    docker run --rm \
      -e "AIRBYTE_CONFIG=${AIRBYTE_CONFIG}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${connector_dir}:/input:ro" \
      "${IMAGE}" "${command}" \
        --config /secrets/config.json \
        --manifest-path /input/connector.yaml \
        "$@"
    ;;

  read)
    configured_catalog="${connector_dir}/configured_catalog.json"
    state_path="${connector_dir}/state.json"

    if [[ ! -f "${configured_catalog}" ]]; then
      echo "ERROR: Missing configured catalog: ${configured_catalog}" >&2
      echo "  Generate with: ./tools/declarative-connector/generate-catalog.sh ${connector_name}" >&2
      exit 1
    fi

    # Create empty state if missing (first run)
    if [[ ! -f "${state_path}" ]]; then
      echo '[]' > "${state_path}"
      echo "INFO: Created empty state file: ${state_path}" >&2
    fi

    shift 3 2>/dev/null || shift 2
    docker run --rm \
      -e "AIRBYTE_CONFIG=${AIRBYTE_CONFIG}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${connector_dir}:/input:ro" \
      "${IMAGE}" read \
        --config /secrets/config.json \
        --catalog "/input/configured_catalog.json" \
        --manifest-path /input/connector.yaml \
        --state "/input/state.json" \
        "$@"
    ;;

  *)
    echo "ERROR: Unknown command '${command}'" >&2
    usage
    exit 1
    ;;
esac
