#!/usr/bin/env bash
#
# render-system-values.sh — resolve the ${NS_*} producer-namespace
# placeholders in L2 system values so cross-service hostnames follow the
# inventory instead of hardcoding `insight-infra`.
#
# Usage (the Makefile's _system_values_args is the only caller):
#   NS_INFRA=… NS_LOKI=… … render-system-values.sh <svc> <env> <out-dir>
#
# Renders system/<svc>/values.yaml (and the per-env overlay
# environments/<env>/<svc>-values.yaml when present and non-empty) into
# <out-dir> and prints the `-f <rendered>` arguments for helm.
#
# Substitution replaces ONLY the exact braced tokens for the NS_* variables
# below. Every other `$` construct passes through byte-identical — Grafana
# provisioning `$VAR` / `$$VAR` escapes, dashboard `$__auto` intervals, bare
# `$NS_*` text, and anything `${…}`-shaped inside Alloy configs.

set -euo pipefail

SVC=${1:?usage: render-system-values.sh <svc> <env> <out-dir>}
ENV_NAME=${2:?usage: render-system-values.sh <svc> <env> <out-dir>}
OUT_DIR=${3:?usage: render-system-values.sh <svc> <env> <out-dir>}

NS_VARS=(NS_INFRA NS_MONITORING)

SED_ARGS=()
for v in "${NS_VARS[@]}"; do
  val=${!v:-}
  # An empty namespace renders `svc..svc.cluster.local` — refuse rather
  # than hand helm a values file with a broken FQDN.
  [ -n "$val" ] \
    || { echo "render-system-values: \$$v is empty — inventory not readable?" >&2; exit 1; }
  # A DNS-1123 label is also what keeps the value inert inside the sed
  # expression below.
  [[ $val =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]] \
    || { echo "render-system-values: \$$v='$val' is not a DNS-1123 label" >&2; exit 1; }
  SED_ARGS+=(-e "s/[\$]{$v}/$val/g")
done

BASE="system/$SVC/values.yaml"
[ -f "$BASE" ] || { echo "render-system-values: $BASE not found" >&2; exit 1; }

mkdir -p "$OUT_DIR"

render() {
  local src=$1 dst=$2
  sed "${SED_ARGS[@]}" < "$src" > "$dst"
  printf ' -f %s' "$dst"
}

render "$BASE" "$OUT_DIR/$ENV_NAME-$SVC-values.yaml"

OVERLAY="environments/$ENV_NAME/$SVC-values.yaml"
if [ -s "$OVERLAY" ]; then
  render "$OVERLAY" "$OUT_DIR/$ENV_NAME-$SVC-overlay.yaml"
fi
echo
