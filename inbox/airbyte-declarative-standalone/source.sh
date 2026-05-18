#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

IMAGE="${AIRBYTE_SOURCE_CONNECTOR_IMAGE:-airbyte/source-declarative-manifest:local}"
BASE_IMAGE="${AIRBYTE_SOURCE_BASE_CONNECTOR_IMAGE:-airbyte/source-declarative-manifest:latest}"
COMMAND_NAME="${AIRBYTE_SOURCE_COMMAND:-source-declarative-manifest}"
SECRETS_TMPFS_OPTS="${AIRBYTE_SECRETS_TMPFS_OPTS:-/secrets:rw,mode=1777}"

usage() {
  echo "Usage:" >&2
  echo "  $0 <command> --manifest=<file> [--env=<file>]" >&2
  echo "  $0 read --manifest=<file> --catalog=<file> --state=<file> [--env=<file>]" >&2
  echo "" >&2
  echo "Commands: check, discover, read" >&2
  echo "Config is read from AIRBYTE_SOURCE_CONFIG (JSON string)." >&2
  echo "--env is optional; loads an env file before reading AIRBYTE_SOURCE_CONFIG." >&2
}

command="${1:-}"
if [[ -z "${command}" ]]; then
  usage
  exit 1
fi
shift

# Parse named args
manifest_arg=""
env_file=""
catalog_path=""
state_path=""
extra_args=()

for arg in "$@"; do
  case "${arg}" in
    --manifest=*)  manifest_arg="${arg#--manifest=}" ;;
    --env=*)       env_file="${arg#--env=}" ;;
    --catalog=*)   catalog_path="${arg#--catalog=}" ;;
    --state=*)     state_path="${arg#--state=}" ;;
    *)             extra_args+=("${arg}") ;;
  esac
done

if [[ -z "${manifest_arg}" ]]; then
  echo "--manifest is required" >&2
  usage
  exit 1
fi

# Resolve manifest to an absolute path (relative to CWD)
manifest_path="$(realpath "${manifest_arg}")"
manifest_dir="$(dirname "${manifest_path}")"
manifest_file="$(basename "${manifest_path}")"

if [[ ! -f "${manifest_path}" ]]; then
  echo "Missing manifest: ${manifest_path}" >&2
  exit 1
fi

# Load env file if provided
if [[ -n "${env_file}" ]]; then
  env_file="$(realpath "${env_file}")"
  if [[ ! -f "${env_file}" ]]; then
    echo "Env file not found: ${env_file}" >&2
    exit 1
  fi
  # Read the raw value without bash quoting interpretation (sourcing strips JSON quotes)
  if [[ -z "${AIRBYTE_SOURCE_CONFIG:-}" ]]; then
    AIRBYTE_SOURCE_CONFIG="$(sed -n 's/^AIRBYTE_SOURCE_CONFIG=//p' "${env_file}")"
  fi
fi

if [[ -z "${AIRBYTE_SOURCE_CONFIG:-}" ]]; then
  echo "AIRBYTE_SOURCE_CONFIG is not set" >&2
  exit 1
fi

# Build local docker image if it doesn't exist yet
if ! docker image inspect "${IMAGE}" >/dev/null 2>&1; then
  echo "Building ${IMAGE}..." >&2
  docker build \
    --build-arg "BASE_IMAGE=${BASE_IMAGE}" \
    --build-arg "AIRBYTE_COMMAND=${COMMAND_NAME}" \
    -f "${SCRIPT_DIR}/sources/Dockerfile" \
    -t "${IMAGE}" \
    "${SCRIPT_DIR}/sources" >/dev/null
fi

case "${command}" in
  check|discover)
    docker run --rm \
      -e "AIRBYTE_SOURCE_CONFIG=${AIRBYTE_SOURCE_CONFIG}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${manifest_dir}:/input:ro" \
      "${IMAGE}" "${command}" \
        --config /secrets/config.json \
        --manifest-path "/input/${manifest_file}" \
        "${extra_args[@]+"${extra_args[@]}"}"
    ;;
  read)
    if [[ -z "${catalog_path}" || -z "${state_path}" ]]; then
      echo "read requires --catalog=<path> and --state=<path>" >&2
      usage
      exit 1
    fi
    catalog_path="$(realpath "${catalog_path}")"
    state_path="$(realpath "${state_path}")"
    if [[ ! -f "${catalog_path}" ]]; then
      echo "Missing catalog: ${catalog_path}" >&2
      exit 1
    fi
    if [[ ! -f "${state_path}" ]]; then
      echo "Missing state: ${state_path}" >&2
      exit 1
    fi
    docker run --rm \
      -e "AIRBYTE_SOURCE_CONFIG=${AIRBYTE_SOURCE_CONFIG}" \
      -e "AIRBYTE_COMMAND=${COMMAND_NAME}" \
      --tmpfs "${SECRETS_TMPFS_OPTS}" \
      -v "${manifest_dir}:/input:ro" \
      -v "${catalog_path}:/catalog.json:ro" \
      -v "${state_path}:/state.json:ro" \
      "${IMAGE}" read \
        --config /secrets/config.json \
        --manifest-path "/input/${manifest_file}" \
        --catalog /catalog.json \
        --state /state.json \
        "${extra_args[@]+"${extra_args[@]}"}"
    ;;
  *)
    usage
    exit 1
    ;;
esac
