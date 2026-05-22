#!/usr/bin/env bash
# Advisory: list type=cdk descriptors whose images.cdk.image is missing/empty.
# Exit 0 always (per dod-reconcile-cdk-image-required: missing →
# reconcile WARN+skip; not a hard failure).
# @cpt:cpt-insightspec-dod-reconcile-cdk-image-required
set -euo pipefail
SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PARSE_PY="${SCRIPT_DIR}/../python/parse_descriptor.py"

missing=()
while IFS= read -r desc; do
  [[ -n "${desc}" ]] || continue
  type="$(python3 "${PARSE_PY}" --descriptor "${desc}" --field type 2>/dev/null || true)"
  if [[ "${type}" != "cdk" ]]; then continue; fi
  cdk_image="$(python3 "${PARSE_PY}" --descriptor "${desc}" --field images.cdk.image 2>/dev/null || true)"
  if [[ -z "${cdk_image}" ]]; then
    missing+=("${desc}")
  fi
done < <(find "${CONNECTORS_DIR:-src/ingestion/connectors}" -name 'descriptor.yaml' 2>/dev/null | sort)  # RULE-DEFAULTS-OK: repo-root-relative path

if [[ ${#missing[@]} -gt 0 ]]; then
  printf 'CDK descriptors lacking images.cdk.image (advisory; reconcile WARN+skips these):\n'
  for m in "${missing[@]}"; do printf '  - %s\n' "$m"; done
fi
echo "audit OK (advisory)."
