#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

SECRETS_TMPFS_OPTS="${AIRBYTE_SECRETS_TMPFS_OPTS:-/secrets:rw,mode=1777}"
AIRBYTE_WRITE_MESSAGE_TYPES="${AIRBYTE_WRITE_MESSAGE_TYPES:-RECORD,STATE}"

usage() {
  echo "Usage:" >&2
  echo "  $0 <destination> check --namespace=<ns> [--env=<file>]" >&2
  echo "  $0 <destination> --namespace=<ns> --catalog=<file> [--env=<file>]" >&2
  echo "" >&2
  echo "Examples:" >&2
  echo "  $0 postgres check --namespace=_raw_ms365 --env=./destinations/postgres/monitor/.env.local" >&2
  echo "  ./source.sh ./connectors/ms365/ms365.yaml read \\" >&2
  echo "    --catalog=./connections/ms365-constructor/configured_catalog.json \\" >&2
  echo "    --state=./connections/ms365-constructor/state.json \\" >&2
  echo "    --env=./connections/ms365-constructor/.env.local \\" >&2
  echo "    | $0 postgres --namespace=_raw_ms365 \\" >&2
  echo "        --catalog=./connections/ms365-constructor/configured_catalog.json \\" >&2
  echo "        --env=./connections/ms365-constructor/.env.local" >&2
  echo "" >&2
  echo "Config is read from AIRBYTE_DESTINATION_CONFIG (JSON string)." >&2
}

destination="${1:-}"
if [[ -z "${destination}" ]]; then
  usage
  exit 1
fi
shift

# Parse remaining args
command="write"
namespace=""
catalog_path=""
env_file=""
extra_args=()

for arg in "$@"; do
  case "${arg}" in
    check)          command="check" ;;
    --namespace=*)  namespace="${arg#--namespace=}" ;;
    --catalog=*)    catalog_path="${arg#--catalog=}" ;;
    --env=*)        env_file="${arg#--env=}" ;;
    *)              extra_args+=("${arg}") ;;
  esac
done

if [[ -z "${namespace}" ]]; then
  echo "--namespace is required" >&2
  usage
  exit 1
fi

# Load AIRBYTE_DESTINATION_CONFIG from env file using sed (avoids bash quote stripping)
if [[ -n "${env_file}" ]]; then
  env_file="$(realpath "${env_file}")"
  if [[ ! -f "${env_file}" ]]; then
    echo "Env file not found: ${env_file}" >&2
    exit 1
  fi
  if [[ -z "${AIRBYTE_DESTINATION_CONFIG:-}" ]]; then
    AIRBYTE_DESTINATION_CONFIG="$(sed -n 's/^AIRBYTE_DESTINATION_CONFIG=//p' "${env_file}")"
    export AIRBYTE_DESTINATION_CONFIG
  fi
fi

if [[ -z "${AIRBYTE_DESTINATION_CONFIG:-}" ]]; then
  echo "AIRBYTE_DESTINATION_CONFIG is not set" >&2
  exit 1
fi

# Inject schema (namespace) and disable 1s1t format (requires Airbyte platform, not needed standalone)
resolved_config="$(
  python3 -c '
import json, os, sys
cfg = json.loads(os.environ["AIRBYTE_DESTINATION_CONFIG"])
cfg["schema"] = sys.argv[1]
cfg.pop("use_1s1t_format", None)
print(json.dumps(cfg, separators=(",", ":")))
' "${namespace}"
)"

case "${destination}" in
  postgres)
    IMAGE="${AIRBYTE_DESTINATION_POSTGRES_IMAGE:-airbyte/destination-postgres:local}"
    BASE_IMAGE="${AIRBYTE_DESTINATION_POSTGRES_BASE_IMAGE:-airbyte/destination-postgres:latest}"
    COMMAND_NAME="${AIRBYTE_DESTINATION_POSTGRES_COMMAND:-/airbyte/bin/destination-postgres}"
    DEST_DIR="${SCRIPT_DIR}/destinations/postgres"
    ;;
  *)
    echo "Unsupported destination: ${destination}" >&2
    echo "Supported: postgres" >&2
    exit 1
    ;;
esac

if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
  echo "Building ${IMAGE}..." >&2
  docker build \
    --build-arg "BASE_IMAGE=${BASE_IMAGE}" \
    --build-arg "AIRBYTE_COMMAND=${COMMAND_NAME}" \
    -f "${DEST_DIR}/Dockerfile" \
    -t "${IMAGE}" \
    "${DEST_DIR}" >/dev/null
fi

case "${command}" in
  check)
    docker run --rm \
      -e "AIRBYTE_CONFIG=${resolved_config}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      "${IMAGE}" --check \
        --config /secrets/config.json \
        "${extra_args[@]+"${extra_args[@]}"}"
    ;;
  write)
    if [[ -z "${catalog_path}" ]]; then
      echo "--catalog is required for write" >&2
      usage
      exit 1
    fi
    catalog_path="$(realpath "${catalog_path}")"
    if [[ ! -f "${catalog_path}" ]]; then
      echo "Missing catalog: ${catalog_path}" >&2
      exit 1
    fi
    # Inject generation_id/minimum_generation_id/sync_id into catalog (required by destination CDK 2.x)
    patched_catalog="$(mktemp /tmp/airbyte_catalog_XXXXXX.json)"
    trap 'rm -f "${patched_catalog}"' EXIT
    python3 -c '
import json, sys
with open(sys.argv[1]) as f:
    catalog = json.load(f)
for i, stream in enumerate(catalog.get("streams", [])):
    stream.setdefault("generation_id", 0)
    stream.setdefault("minimum_generation_id", 0)
    stream.setdefault("sync_id", i + 1)
print(json.dumps(catalog))
' "${catalog_path}" > "${patched_catalog}"
    catalog_path="${patched_catalog}"
    # Filter stdin: pass RECORD, STATE, and STREAM_STATUS traces to the destination
    python3 -u -c '
import json, os, sys
allowed = {t.strip().upper() for t in os.environ.get("AIRBYTE_WRITE_MESSAGE_TYPES","RECORD,STATE").split(",") if t.strip()}
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        continue
    mtype = str(msg.get("type","")).upper()
    if mtype in allowed:
        sys.stdout.write(json.dumps(msg, separators=(",",":")) + "\n")
        sys.stdout.flush()
    elif mtype == "TRACE" and str(msg.get("trace",{}).get("type","")).upper() == "STREAM_STATUS":
        sys.stdout.write(json.dumps(msg, separators=(",",":")) + "\n")
        sys.stdout.flush()
' | docker run --rm -i \
      -e "AIRBYTE_CONFIG=${resolved_config}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${catalog_path}:/input/configured_catalog.json:ro" \
      "${IMAGE}" --write \
        --config /secrets/config.json \
        --catalog /input/configured_catalog.json \
        "${extra_args[@]+"${extra_args[@]}"}"
    ;;
  *)
    usage
    exit 1
    ;;
esac
