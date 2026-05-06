#!/usr/bin/env bash
# secret_* — K8s Secret CRUD helpers (sourceable; NO top-level CLI)
# NOTE: this file is sourced; no top-level `set -euo pipefail`.

secret_list_by_label() {
  local label="$1"  # e.g. "insight.cyberfabric.com/connector"
  kubectl get secrets -A -l "${label}" -o json
}

secret_read_data() {
  local namespace="$1" name="$2"
  kubectl -n "$namespace" get secret "$name" -o json
}

secret_get_annotation() {
  local namespace="$1" name="$2" key="$3"
  kubectl -n "$namespace" get secret "$name" -o jsonpath="{.metadata.annotations.${key//./\\.}}"
}
