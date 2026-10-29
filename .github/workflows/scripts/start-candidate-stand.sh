#!/usr/bin/env bash
set -euo pipefail

directory="${1:?candidate input directory is required}"
source_sha="${2:?source SHA is required}"
mode="${3:?stand mode is required}"
[[ "$mode" == minimal || "$mode" == up ]]

python3 scripts/ci/candidate_inputs.py "$directory" "$source_sha" --compose-env "$directory/compose.env"
while IFS='=' read -r variable image; do
  docker pull "$image"
  export "$variable=$image"
done < "$directory/compose.env"
./dev-compose.sh test-stand "$mode" --prebuilt-backend --prebuilt-frontend
