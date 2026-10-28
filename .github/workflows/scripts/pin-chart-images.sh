#!/usr/bin/env bash
set -euo pipefail

chart=charts/insight
pin_image() {
  local ref="$1" digest
  digest="$(docker buildx imagetools inspect "$ref" --format '{{json .Manifest}}' | jq -er '.digest')"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'Invalid image digest' >&2; return 1; }
  printf '%s@%s' "${ref%@*}" "$digest"
}

dependencies="$(yq -er '.dependencies[] | [.alias, .repository] | @tsv' "$chart/Chart.yaml")"
while IFS=$'\t' read -r alias repository; do
  dependency="$chart/${repository#file://}"
  image_repository="$(yq -r ".${alias}.image.repository // \"\"" "$chart/values.yaml")"
  [[ -n "$image_repository" ]] || image_repository="$(yq -r '.image.repository' "$dependency/values.yaml")"
  tag="$(yq -r ".${alias}.image.tag // \"\"" "$chart/values.yaml")"
  [[ -n "$tag" ]] || tag="$(yq -r '.image.tag // ""' "$dependency/values.yaml")"
  [[ -n "$tag" ]] || tag="$(yq -r '.appVersion' "$dependency/Chart.yaml")"
  ref="$(pin_image "$image_repository:$tag")"
  # INVARIANT: tag@digest preserves the existing repository:tag templates while fixing content identity.
  PINNED_TAG="${ref#"$image_repository:"}" yq -i ".${alias}.image.tag = strenv(PINNED_TAG)" "$chart/values.yaml"
done <<< "$dependencies"

for key in toolboxImage seedImage; do
  ref="$(yq -er ".ingestion.$key" "$chart/values.yaml")"
  pinned_ref="$(pin_image "$ref")"
  PINNED_REF="$pinned_ref" yq -i ".ingestion.$key = strenv(PINNED_REF)" "$chart/values.yaml"
done
