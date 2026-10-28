#!/usr/bin/env bash
set -euo pipefail

directory="${1:?candidate directory is required}"
source_sha="${2:?source SHA is required}"
version="${3:?chart version is required}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]]
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
cd "$directory"
expected="$(printf '%s\n' "insight-$version.tgz" release.patch source-sha version | sort)"
actual="$(awk '{print $2}' SHA256SUMS | sort)"
[[ "$actual" == "$expected" ]] || { echo 'Unexpected candidate files' >&2; exit 1; }
sha256sum --check SHA256SUMS
[[ "$(cat source-sha)" == "$source_sha" ]]
[[ "$(cat version)" == "$version" ]]
