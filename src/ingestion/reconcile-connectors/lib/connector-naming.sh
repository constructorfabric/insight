#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Shared helpers that resolve per-connector identity fields from the
# declarative inputs (descriptor.yaml on disk + K8s Secret annotations)
# without any dependency on reconcile.sh internals. Sourced by both
# reconcile.sh and adopt.sh — previously these lived in reconcile.sh and
# adopt.sh relied on the implicit fact that reconcile.sh sourced adopt.sh
# first then continued running past the function definitions. That
# created a circular dependency that broke if adopt.sh was sourced
# standalone or if the load order changed; extracting the helpers makes
# the dependency explicit and removes the cycle.
#
# Function naming: kept as `reconcile_compute_*` so existing call sites
# (and `@cpt-*` markers in adopt.sh/reconcile.sh) stay valid; the prefix
# refers to *what* the helpers compute, not to the library that hosts
# them.
# ---------------------------------------------------------------------------

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
: "${CONNECTORS_DIR:?CONNECTORS_DIR must be set, typically src/ingestion/connectors}"

_CN_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_CN_PY_DIR="$(cd "${_CN_LIB_DIR}/../python" && pwd)"

# discover.sh provides `disc_match_descriptor_to_secret`. Sourcing here
# instead of relying on the caller keeps the function's behaviour
# stable under any load order.
# shellcheck source=./discover.sh
source "${_CN_LIB_DIR}/discover.sh"

# ---------------------------------------------------------------------------
# reconcile_compute_connection_name <connector_name>
# Derives the Airbyte connection name for a connector: pattern
#   {connector}-{source_id_label}-{tenant_id}-conn
# matching the name used when the connection was created.
# ---------------------------------------------------------------------------
reconcile_compute_connection_name() {
  local connector="$1"
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -z "${secret_name}" ]]; then
    printf '%s-main-%s-conn' "${connector}" "${INSIGHT_TENANT_ID:-}"
    return 0
  fi
  local source_id_label
  source_id_label="$(kubectl -n "${namespace}" get secret "${secret_name}" \
    -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/source-id}' \
    2>/dev/null || true)"
  [[ -n "${source_id_label}" ]] || source_id_label="main"
  printf '%s-%s-%s-conn' "${connector}" "${source_id_label}" "${INSIGHT_TENANT_ID:-}"
}

# ---------------------------------------------------------------------------
# reconcile_compute_schedule <connector_name>
# Schedule precedence: Secret annotation > descriptor.yaml.schedule > default.
# ---------------------------------------------------------------------------
reconcile_compute_schedule() {
  local connector="$1"
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name schedule
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -n "${secret_name}" ]]; then
    schedule="$(kubectl -n "${namespace}" get secret "${secret_name}" \
      -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/schedule}' \
      2>/dev/null || true)"
    [[ -n "${schedule}" ]] && { printf '%s' "${schedule}"; return 0; }
  fi
  # Resolve descriptor path by glob — connectors are nested under an area
  # directory (e.g. ${CONNECTORS_DIR}/collaboration/m365/), so the slug
  # alone doesn't give us a deterministic path.
  local desc_glob desc_path
  # shellcheck disable=SC2206
  desc_glob=("${CONNECTORS_DIR}"/*/"${connector}"/descriptor.yaml)
  desc_path="${desc_glob[0]}"
  if [[ -f "${desc_path}" ]]; then
    schedule="$(python3 "${_CN_PY_DIR}/parse_descriptor.py" \
      --descriptor "${desc_path}" \
      --field schedule 2>/dev/null || true)"
    [[ -n "${schedule}" ]] && { printf '%s' "${schedule}"; return 0; }
  fi
  printf '0 0 * * *'
}

# ---------------------------------------------------------------------------
# reconcile_compute_tenant <connector_name>
# Resolves tenant slug: env INSIGHT_TENANT_ID > Secret metadata > "default".
# ---------------------------------------------------------------------------
reconcile_compute_tenant() {
  local connector="$1"
  [[ -n "${INSIGHT_TENANT_ID:-}" ]] && { printf '%s' "${INSIGHT_TENANT_ID}"; return 0; }
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -z "${secret_name}" ]]; then
    printf 'default'
    return 0
  fi
  local secret_file
  secret_file="$(mktemp -t insight-reconcile.XXXXXX)" || { printf 'default'; return 0; }
  kubectl -n "${namespace}" get secret "${secret_name}" -o json > "${secret_file}" 2>/dev/null || true
  python3 "${_CN_PY_DIR}/resolve_tenant.py" \
    --secret-json "${secret_file}" 2>/dev/null || printf 'default'
  rm -f "${secret_file}"   # explicit cleanup; sourced libs MUST NOT install RETURN traps
}
