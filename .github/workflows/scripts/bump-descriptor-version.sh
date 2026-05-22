#!/usr/bin/env bash
# bump-descriptor-version.sh — bump the `version` field of a connector
# descriptor.yaml by one minor increment per ADR-0015 (strict semver) and
# ADR-0016 (descriptor.images: block).
#
# Called by the CI `bump-descriptors` job and by
# `scripts/bootstrap-connector-images.sh` whenever an image tag in
# `descriptor.yaml.images.<key>.image` is updated. The minor bump makes
# reconcile re-discover the source catalog on the next deploy (per
# ADR-0015 §catalog-refresh-on-bump) but stays below the major-bump
# threshold that would dispatch a `dbt --full-refresh` — a new image is
# a continuation of the same connector contract, not a breaking change.
#
# Tools: bash + mikefarah/yq (v4+) + sed only. The READ side uses yq so
# we parse YAML properly (handles surrounding quotes, anchors, refs).
# The WRITE side is `sed -i` line-replacement — mikefarah/yq's `-i`
# silently strips blank lines from the file (known limitation in v4)
# and we want comments + spacing preserved byte-for-byte across every
# CI bump. bash regex enforces strict semver per semver.org §2 (no
# leading zeros, no `v` prefix, no pre-release, no build metadata).
#
# Usage:
#   bump-descriptor-version.sh --descriptor PATH [--print-only]
#
# Stdout: the new version string (single line).
# Exit:
#   0   bumped (or --print-only printed)
#   1   filesystem / arg / tooling error
#   2   version field present but not strict semver — fail loud per
#       ADR-0015; the operator MUST fix it manually.

set -euo pipefail

DESCRIPTOR=""
PRINT_ONLY=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --descriptor) DESCRIPTOR="${2:?--descriptor needs a path}"; shift 2 ;;
    --print-only) PRINT_ONLY=true; shift ;;
    -h|--help)    sed -n '2,32p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown arg: $1" >&2; exit 1 ;;
  esac
done

[[ -n "${DESCRIPTOR}" ]] || { echo "ERROR: --descriptor is required" >&2; exit 1; }
[[ -f "${DESCRIPTOR}" ]] || { echo "ERROR: ${DESCRIPTOR} not found" >&2; exit 1; }

command -v yq >/dev/null 2>&1 || {
  echo "ERROR: mikefarah/yq (v4+) is required. Install: brew install yq" >&2
  exit 1
}

# Guard against accidentally invoking python-yq (kislyuk's jq+yaml tool,
# different syntax). mikefarah/yq's --version prints `yq (https://github.com/mikefarah/yq/) version v4.X.Y`.
if ! yq --version 2>&1 | grep -qE 'mikefarah|yq.*v?[4-9]\.'; then
  echo "ERROR: mikefarah/yq (v4+) required; you appear to have python-yq." >&2
  echo "       brew uninstall python-yq && brew install yq" >&2
  exit 1
fi

CURRENT="$(yq -r '.version // ""' "${DESCRIPTOR}")"

if [[ -z "${CURRENT}" || "${CURRENT}" == "null" ]]; then
  echo "ERROR: ${DESCRIPTOR}: no \`version:\` field. Every descriptor MUST" >&2
  echo "       declare a strict-semver version per ADR-0015." >&2
  exit 2
fi

# Strict semver per semver.org §2:
#   MAJOR/MINOR/PATCH = "0" OR a non-zero digit followed by more digits.
#   No `v` prefix, no pre-release, no build metadata.
SEMVER_RE='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

if ! [[ "${CURRENT}" =~ ${SEMVER_RE} ]]; then
  echo "ERROR: ${DESCRIPTOR}: \`version:\` is not strict semver MAJOR.MINOR.PATCH" >&2
  echo "       (per ADR-0015). Got: '${CURRENT}'." >&2
  echo "       Fix it manually (e.g. version: \"1.0.0\") before this script can bump it." >&2
  exit 2
fi

MAJOR="${BASH_REMATCH[1]}"
MINOR="${BASH_REMATCH[2]}"
# PATCH unused — strict-semver minor bump always resets it to 0.
NEW_VERSION="${MAJOR}.$((MINOR + 1)).0"

if [[ "${PRINT_ONLY}" == "false" ]]; then
  # Line-replacement via sed. The descriptor format we author always puts
  # `version:` at column 0 followed by a quoted string. We match the
  # whole line and substitute, which preserves leading whitespace
  # (there shouldn't be any here) plus all other lines + blank lines +
  # comments byte-for-byte. yq's `-i` strips blank lines in v4 — see
  # https://github.com/mikefarah/yq/issues/515 — so we don't use it.
  #
  # `sed -i.bak` is portable across GNU and BSD; we remove the .bak.
  sed -i.bak -E "s|^version:[[:space:]]+\"?[0-9.]+\"?[[:space:]]*$|version: \"${NEW_VERSION}\"|" "${DESCRIPTOR}"
  rm -f "${DESCRIPTOR}.bak"

  # Sanity: re-read via yq to confirm the file still parses as YAML and
  # the new value stuck. If sed somehow corrupted the file (e.g. version
  # line not unique), yq will fail here and the job stops with a clear
  # error rather than committing a malformed descriptor.
  ACTUAL="$(yq -r '.version' "${DESCRIPTOR}")"
  if [[ "${ACTUAL}" != "${NEW_VERSION}" ]]; then
    echo "ERROR: ${DESCRIPTOR}: sed-replace did not stick (read back '${ACTUAL}', expected '${NEW_VERSION}')." >&2
    echo "       The descriptor may have malformed YAML; restoring from git is the safest recovery." >&2
    exit 1
  fi
fi

echo "${NEW_VERSION}"
