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
#   - python3 with stdlib + PyYAML (the latter is needed by discover-image-matrix.py;
#     check `python3 -c 'import yaml'` — if missing, install via brew/pipx)
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

# PyYAML — needed by discover-image-matrix.py
if ! python3 -c 'import yaml' 2>/dev/null; then
  echo "ERROR: python3 is missing PyYAML." >&2
  echo "       Install via one of:" >&2
  echo "         brew install pyyaml         # if it's a Homebrew Python" >&2
  echo "         pipx install --pip-args='--upgrade' --include-deps pyyaml" >&2
  echo "         python3 -m pip install --user --break-system-packages pyyaml" >&2
  exit 1
fi
# Descriptor patching uses stdlib regex (no ruamel.yaml dependency) so the
# script runs on any Python 3 without extra installs. The patcher targets
# the exact format we control in descriptor.yaml — see patch_image() below.

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

# ─── Descriptor patcher (stdlib regex — preserves comments + formatting) ──
# Replaces just the `image: "..."` line under `  <key>:` under `images:`.
# Indentation is fixed (2-space images, 4-space sub-fields) per ADR-0016
# schema and the format we author. NO ruamel.yaml / PyYAML round-trip,
# so all comments and blank lines survive byte-for-byte except the one line.

PATCH_PY="$(mktemp -t patch-descriptor.XXXXXX.py)"
trap 'rm -f "${PATCH_PY}"' EXIT

cat > "${PATCH_PY}" <<'PY'
"""patch <descriptor.yaml> <key> <new_image_ref>

Targets:
    images:
      <key>:
        ...
        image: "<old>"

Replaces just the `    image: ...` line; leaves everything else byte-identical.
"""
import re
import sys

descriptor, key, new_image = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(descriptor).read()

# Find images: block at column 0.
m = re.search(r'(?m)^images:\s*$', text)
if not m:
    sys.exit(f"ERROR: {descriptor}: no `images:` block")
images_start = m.end()

# Find the key: line at indent 2 within the images block.
# The images block ends at the next column-0, non-blank, non-comment line.
images_end_re = re.compile(r'(?m)^(?:[^\s#].*)$')
end_match = images_end_re.search(text, images_start)
images_end = end_match.start() if end_match else len(text)
images_block = text[images_start:images_end]

key_re = re.compile(rf'(?m)^  {re.escape(key)}:\s*$')
km = key_re.search(images_block)
if not km:
    sys.exit(f"ERROR: {descriptor}: no `images.{key}:` sub-block")
sub_start_abs = images_start + km.end()

# Within the <key>: sub-block, find the first `    image: ...` line at indent 4.
# Sub-block ends at the next `  <something>:` (indent 2) or end of images block.
next_key_re = re.compile(r'(?m)^  \S')
nk = next_key_re.search(text, sub_start_abs)
sub_end_abs = nk.start() if nk and nk.start() < images_end else images_end
sub_block = text[sub_start_abs:sub_end_abs]

img_re = re.compile(r'(?m)^    image:\s.*$')
im = img_re.search(sub_block)
if not im:
    sys.exit(f"ERROR: {descriptor}: no `image:` field under `images.{key}:`")
img_start_abs = sub_start_abs + im.start()
img_end_abs   = sub_start_abs + im.end()

new_line = f'    image: "{new_image}"'
patched = text[:img_start_abs] + new_line + text[img_end_abs:]

with open(descriptor, "w") as f:
    f.write(patched)
print(f"  patched {descriptor}: images.{key}.image = {new_image}")
PY

# ─── Iterate matrix: build + push + patch ──────────────────────────────────

FAILED=()
PATCHED_DESCRIPTORS=()

echo
echo "${MATRIX_JSON}" | python3 -c "
import json, sys
for e in json.load(sys.stdin):
    print('\t'.join([e['connector_dir'], e['key'], e['name'], e['dockerfile'], e['context']]))
" | while IFS=$'\t' read -r connector_dir key name dockerfile context; do
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
  if ! python3 "${PATCH_PY}" "${connector_dir}/descriptor.yaml" "${key}" "${ref}"; then
    echo "FAIL: patch ${connector_dir}/descriptor.yaml" >&2
    FAILED+=("${slug}/${key}:patch")
    continue
  fi
  PATCHED_DESCRIPTORS+=("${connector_dir}/descriptor.yaml")
done

# After all image patches, bump descriptor.version (minor) once per
# affected connector. Mirrors the CI bump-descriptors job. Read patched
# descriptors from git status to handle the subshell-variable lifetime
# issue (the same trick the summary step below uses).
BUMP_SCRIPT="${REPO_ROOT}/.github/workflows/scripts/bump-descriptor-version.py"
if [[ -x "${BUMP_SCRIPT}" ]]; then
  PATCHED_FOR_BUMP="$(git status --porcelain src/ingestion/connectors/*/*/descriptor.yaml 2>/dev/null \
    | awk '{print $2}')"
  if [[ -n "${PATCHED_FOR_BUMP}" ]]; then
    echo
    echo "─── Bumping descriptor.version (minor) for each patched connector ───"
    echo "${PATCHED_FOR_BUMP}" | while IFS= read -r desc; do
      [[ -n "${desc}" ]] || continue
      python3 "${BUMP_SCRIPT}" --descriptor "${desc}" || {
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
