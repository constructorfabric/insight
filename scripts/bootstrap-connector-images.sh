#!/usr/bin/env bash
# bootstrap-connector-images.sh — replicate CI's image build + descriptor
# bump LOCALLY, for the first run before the GHCR / branch-protection /
# INSIGHT_RELEASE_APP setup lands in CI.
#
# Runs the same logic as .github/workflows/build-images.yml's `discover-images`
# + `build-image` + `bump-descriptors` jobs, using the operator's GitHub CLI
# auth (`gh auth token`) for GHCR push instead of CI tokens. The same
# `discover-image-matrix.py` helper is the source of truth — no parallel
# implementation to drift.
#
# Effect: every connector with an `images.<key>` entry under
# `src/ingestion/connectors/*/*/descriptor.yaml` gets:
#   1. its image built from the declared `context` + `dockerfile`
#   2. its image pushed to `ghcr.io/cyberfabric/<images.<key>.name>:<BUILD_TAG>`
#   3. its `descriptor.yaml.images.<key>.image` patched with the new ref
#
# The script DOES NOT commit, push, or create a PR — it leaves the working
# tree dirty with the patched descriptors for the operator to review and
# commit by hand. After commit + push the next CI run will pick up from
# Run 2 (toolbox rebuild + chart publish) — no need to run image builds in CI
# at all for this bootstrap cycle.
#
# Prerequisites (verified at startup):
#   - docker
#   - python3 with stdlib + PyYAML (needed by discover-image-matrix.py;
#     `python3 -c 'import yaml'` — install via brew/pipx if missing)
#   - mikefarah/yq v4+ (NOT python-yq); used for descriptor read + version
#     bump. `brew install yq` — if you have python-yq, swap it.
#   - jq (matrix iteration)
#   - sed (line-replacement; ships with macOS + every Linux)
#   - gh (authenticated with `write:packages` scope; verified via gh auth status)
#   - git (working dir must be the repo root or any subdirectory under it)
#
# Usage:
#   scripts/bootstrap-connector-images.sh                # all connectors
#   scripts/bootstrap-connector-images.sh --dry-run      # discover + show plan only
#   scripts/bootstrap-connector-images.sh --no-push      # build locally, skip push
#   scripts/bootstrap-connector-images.sh --connector hubspot  # one connector
#
# Exit codes:
#   0   success (or dry-run print)
#   1   prereq missing
#   2   build or push failed
#   3   descriptor patch failed

set -euo pipefail

# ─── Args ──────────────────────────────────────────────────────────────────

DRY_RUN=false
PUSH=true
CONNECTOR_FILTER=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)        DRY_RUN=true; shift ;;
    --no-push)        PUSH=false; shift ;;
    --connector)      CONNECTOR_FILTER="${2:?--connector requires a name}"; shift 2 ;;
    -h|--help)
      sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown arg: $1" >&2; exit 1 ;;
  esac
done

# ─── Repo root ─────────────────────────────────────────────────────────────

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# ─── Prereqs ───────────────────────────────────────────────────────────────

prereq_fail() {
  echo "ERROR: $1 is required but not installed" >&2
  exit 1
}

command -v docker  >/dev/null 2>&1 || prereq_fail docker
command -v python3 >/dev/null 2>&1 || prereq_fail python3
command -v gh      >/dev/null 2>&1 || prereq_fail gh
command -v git     >/dev/null 2>&1 || prereq_fail git
command -v jq      >/dev/null 2>&1 || prereq_fail jq
command -v yq      >/dev/null 2>&1 || prereq_fail yq
command -v sed     >/dev/null 2>&1 || prereq_fail sed

# PyYAML — needed by discover-image-matrix.py
if ! python3 -c 'import yaml' 2>/dev/null; then
  echo "ERROR: python3 is missing PyYAML." >&2
  echo "       Install via one of:" >&2
  echo "         brew install pyyaml         # if it's a Homebrew Python" >&2
  echo "         pipx install --pip-args='--upgrade' --include-deps pyyaml" >&2
  echo "         python3 -m pip install --user --break-system-packages pyyaml" >&2
  exit 1
fi

# mikefarah/yq v4 (not python-yq) — needed by the descriptor patch + version
# bump steps below, mirroring the CI workflow.
if ! yq --version 2>&1 | grep -qE 'mikefarah|yq.*v?[4-9]\.'; then
  echo "ERROR: mikefarah/yq (v4+) is required; you appear to have python-yq." >&2
  echo "       brew uninstall python-yq && brew install yq" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "ERROR: gh is not authenticated. Run: gh auth login --scopes write:packages" >&2
  exit 1
fi

GH_USER="$(gh api user --jq .login)"
echo "GitHub user: ${GH_USER}"

# ─── GHCR login (no-op if already logged in with the same token) ──────────

REGISTRY="ghcr.io"
IMAGE_PREFIX="ghcr.io/cyberfabric"

if [[ "${PUSH}" == "true" && "${DRY_RUN}" == "false" ]]; then
  echo "Logging into ${REGISTRY} via gh auth token..."
  gh auth token | docker login "${REGISTRY}" -u "${GH_USER}" --password-stdin
fi

# ─── Compute BUILD_TAG (same format as CI) ─────────────────────────────────

BUILD_TAG="$(date -u +%Y.%m.%d.%H.%M)-$(git rev-parse --short=7 HEAD)"
echo "BUILD_TAG: ${BUILD_TAG}"

# ─── Discover matrix via the same helper CI uses ──────────────────────────

DISCOVER_SCRIPT=".github/workflows/scripts/discover-image-matrix.py"
[[ -f "${DISCOVER_SCRIPT}" ]] || {
  echo "ERROR: ${DISCOVER_SCRIPT} not found; are you on the right branch?" >&2
  exit 1
}

MATRIX_JSON="$(python3 "${DISCOVER_SCRIPT}" \
  --connectors-root src/ingestion/connectors --all)"

if [[ -n "${CONNECTOR_FILTER}" ]]; then
  MATRIX_JSON="$(echo "${MATRIX_JSON}" | python3 -c "
import json, sys
slug = '${CONNECTOR_FILTER}'
data = json.load(sys.stdin)
filtered = [e for e in data if e['connector_dir'].rsplit('/', 1)[-1] == slug]
if not filtered:
    sys.stderr.write(f'ERROR: no images: entries for connector {slug!r}\n')
    sys.exit(2)
print(json.dumps(filtered))
")"
fi

LEN="$(echo "${MATRIX_JSON}" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
if [[ "${LEN}" -eq 0 ]]; then
  echo "discover-image-matrix.py returned 0 entries — nothing to do."
  exit 0
fi

echo "Plan: ${LEN} image(s) to build"
echo "${MATRIX_JSON}" | python3 -c "
import json, sys
for e in json.load(sys.stdin):
    print(f\"  - {e['connector_dir'].rsplit('/', 1)[-1]:20s} / {e['key']:8s} -> {e['name']}\")
"

if [[ "${DRY_RUN}" == "true" ]]; then
  echo
  echo "Dry-run only; nothing built or pushed. Drop --dry-run to proceed."
  exit 0
fi

# ─── Descriptor patcher (sed line-replacement) ────────────────────────────
# Replaces just the `image: "..."` line under `<key>:` under `images:`. The
# read side uses yq (validates the descriptor is parseable YAML and that
# `images.<key>` exists); the write side is `sed -i` line-substitution
# because mikefarah/yq's `-i` strips blank lines (issue #515) and we want
# byte-for-byte preservation across every bump.

patch_image_in_descriptor () {
  local desc="$1" key="$2" new_image="$3"
  # Sanity: confirm images.<key>.image actually exists before patching.
  local current
  current="$(yq -r ".images.\"${key}\".image // \"__missing__\"" "${desc}")"
  if [[ "${current}" == "__missing__" ]]; then
    echo "ERROR: ${desc}: no \`images.${key}.image\` field" >&2
    return 1
  fi
  # The descriptor format we author always indents `image:` four spaces under
  # the two-space `<key>:` under `images:` at column 0. Anchor the match on
  # that exact shape to avoid touching unrelated `image:` lines elsewhere.
  # We use awk for the in-block-only substitution because sed can't easily
  # express "replace the image: line inside the matching <key>: block".
  local tmp
  tmp="$(mktemp -t patch-descriptor.XXXXXX)"
  awk -v key="${key}" -v new="${new_image}" '
    BEGIN { in_images = 0; in_key = 0 }
    /^images:[[:space:]]*$/         { in_images = 1; print; next }
    in_images && /^[^[:space:]#]/   { in_images = 0; in_key = 0 }
    in_images && match($0, "^  "key":[[:space:]]*$") { in_key = 1;  print; next }
    in_images && /^  [^[:space:]]/  { in_key = 0 }
    in_key && match($0, "^    image:[[:space:]]") {
      print "    image: \"" new "\""; next
    }
    { print }
  ' "${desc}" > "${tmp}" && mv "${tmp}" "${desc}"

  # Sanity: yq re-reads the patched value; if awk somehow corrupted the
  # YAML the read fails and we surface a clear error.
  local actual
  actual="$(yq -r ".images.\"${key}\".image" "${desc}")"
  if [[ "${actual}" != "${new_image}" ]]; then
    echo "ERROR: ${desc}: patch did not stick (read back '${actual}', expected '${new_image}')" >&2
    return 1
  fi
  echo "  patched ${desc}: images.${key}.image = ${new_image}"
}

# ─── Iterate matrix: build + push + patch ──────────────────────────────────

FAILED=()

echo
# Stream matrix as TSV via jq; each line is one image-entry to build.
echo "${MATRIX_JSON}" \
  | jq -r '.[] | [.connector_dir, .key, .name, .dockerfile, .context] | @tsv' \
  | while IFS=$'\t' read -r connector_dir key name dockerfile context; do
  slug="$(basename "${connector_dir}")"
  ref="${IMAGE_PREFIX}/${name}:${BUILD_TAG}"
  ref_latest="${IMAGE_PREFIX}/${name}:latest"
  build_context="${connector_dir}/${context}"
  build_file="${connector_dir}/${dockerfile}"

  echo "─── ${slug} / ${key} ───────────────────────────────────────────"
  echo "  context:    ${build_context}"
  echo "  dockerfile: ${build_file}"
  echo "  tag:        ${ref}"

  if ! docker build \
        --tag "${ref}" \
        --tag "${ref_latest}" \
        --file "${build_file}" \
        "${build_context}"; then
    echo "FAIL: build ${slug}/${key}" >&2
    FAILED+=("${slug}/${key}:build")
    continue
  fi

  if [[ "${PUSH}" == "true" ]]; then
    if ! docker push "${ref}"; then
      echo "FAIL: push ${ref}" >&2
      FAILED+=("${slug}/${key}:push-tag")
      continue
    fi
    if ! docker push "${ref_latest}"; then
      echo "FAIL: push ${ref_latest}" >&2
      FAILED+=("${slug}/${key}:push-latest")
      continue
    fi
  else
    echo "  (push skipped per --no-push)"
  fi

  # Patch descriptor.yaml.images.<key>.image with the new full ref.
  # This mirrors the CI bump-descriptors job exactly.
  if ! patch_image_in_descriptor "${connector_dir}/descriptor.yaml" "${key}" "${ref}"; then
    FAILED+=("${slug}/${key}:patch")
    continue
  fi
done

# After all image patches, bump descriptor.version (minor) once per
# affected connector. Mirrors the CI bump-descriptors job. Read patched
# descriptors from git status to handle the subshell-variable lifetime
# issue (the same trick the summary step below uses).
BUMP_SCRIPT="${REPO_ROOT}/.github/workflows/scripts/bump-descriptor-version.sh"
if [[ -x "${BUMP_SCRIPT}" ]]; then
  PATCHED_FOR_BUMP="$(git status --porcelain src/ingestion/connectors/*/*/descriptor.yaml 2>/dev/null \
    | awk '{print $2}')"
  if [[ -n "${PATCHED_FOR_BUMP}" ]]; then
    echo
    echo "─── Bumping descriptor.version (minor) for each patched connector ───"
    echo "${PATCHED_FOR_BUMP}" | while IFS= read -r desc; do
      [[ -n "${desc}" ]] || continue
      "${BUMP_SCRIPT}" --descriptor "${desc}" || {
        echo "FAIL: version bump for ${desc}" >&2
        FAILED+=("${desc}:version-bump")
      }
    done
  fi
else
  echo "WARN: ${BUMP_SCRIPT} not found or not executable — skipping version bump" >&2
fi

# NOTE: subshell variables don't propagate out of the `while ... < pipe` —
# we re-derive PATCHED_DESCRIPTORS for the summary from git status.

echo
echo "─── Summary ─────────────────────────────────────────────────────────"
echo "BUILD_TAG: ${BUILD_TAG}"

PATCHED_FROM_GIT="$(git status --porcelain src/ingestion/connectors/*/*/descriptor.yaml 2>/dev/null \
  | awk '{print $2}')"
if [[ -n "${PATCHED_FROM_GIT}" ]]; then
  echo "Patched descriptors (working tree):"
  echo "${PATCHED_FROM_GIT}" | sed 's/^/  /'
else
  echo "No descriptors patched (nothing built, or git status is clean)."
fi

echo
echo "Next steps:"
echo "  1. Review the patched descriptors:    git diff src/ingestion/connectors/"
echo "  2. Commit them:                       git add -p && git commit -m \"chore(descriptors): bootstrap-bump image refs + version (minor) to ${BUILD_TAG}\""
echo "  3. Push to main (or open a PR).       After push, CI's toolbox + publish-chart"
echo "     jobs will produce the umbrella chart with these patched descriptors baked in."
echo "  4. Verify GHCR has the pushed images: gh api /user/packages?package_type=container | jq '.[].name'"
