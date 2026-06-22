#!/usr/bin/env bash
#
# secret-fetch.sh — sample STUB for the seal-secret pipeline.
#
# Contract:
#   Argument 1: a resource name (e.g. "insight-local-mariadb-creds").
#   Stdout:     the cleartext Kubernetes Secret manifest for that
#               resource, ready for `kubeseal` (YAML or JSON; kubeseal
#               accepts either).
#   Exit 0 on success; non-zero on lookup failure (which aborts the
#   Makefile's seal target before any sealed file is overwritten).
#
# This stub reads from a local YAML file (default:
# `secrets-store.yaml` at the repo root) whose top-level keys are
# resource names and values are cleartext Secret manifests. See
# `secrets-store.yaml.template` for the format.
#
# IMPORTANT — replace this stub before going to production. The flat
# YAML file is only convenient for sandbox / first-time-walkthrough
# use. In real deployments the cleartext should live in a proper
# password manager / secret store (Vault, 1Password, Bitwarden, AWS
# Secrets Manager, GCP Secret Manager, Passbolt, …). To swap backends,
# rewrite this script to:
#   1. Resolve $1 to a record in your backend.
#   2. Print the cleartext Kubernetes Secret manifest to stdout.
#   3. Exit 0 on success, non-zero with a useful stderr message on
#      failure.
# `make seal-secret` pipes stdout straight into `kubeseal`, so the
# cleartext never lands on disk.

set -euo pipefail

NAME="${1:?usage: secret-fetch.sh <resource-name>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STORE="${SECRET_STORE_FILE:-$SCRIPT_DIR/../secrets-store.yaml}"

if [ ! -f "$STORE" ]; then
  echo "ERROR: secret-store file not found at $STORE" >&2
  echo "       Copy secrets-store.yaml.template to secrets-store.yaml," >&2
  echo "       fill in the cleartext Secret manifests, and re-run." >&2
  echo "       (Or override the path with SECRET_STORE_FILE=...)" >&2
  exit 1
fi

command -v yq >/dev/null 2>&1 || { echo "ERROR: yq is required" >&2; exit 1; }

VALUE=$(yq -r ".\"$NAME\" // \"\"" "$STORE")

if [ -z "$VALUE" ] || [ "$VALUE" = "null" ]; then
  echo "ERROR: no entry for resource '$NAME' in $STORE" >&2
  echo "       Available keys:" >&2
  yq -r 'keys | .[] | "         - " + .' "$STORE" >&2 || true
  exit 1
fi

printf '%s' "$VALUE"
