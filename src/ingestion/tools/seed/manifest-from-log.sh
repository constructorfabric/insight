#!/usr/bin/env bash
# Recover a seed manifest from a Job log and print it as one plain
# `SEED_MANIFEST_JSON:` line, whichever sentinel form the seeder emitted
# (see the README's "Reading the manifest back" section).
#
#   kubectl -n <ns> logs job/<job> --tail=-1 | manifest-from-log.sh
#   manifest-from-log.sh seed.log | cut -d' ' -f2- | jq .
set -euo pipefail

die() { echo "ERROR: $*" >&2; exit 1; }

log="$(cat -- "${1:--}")"

plain="$(grep -m1 '^SEED_MANIFEST_JSON: ' <<<"$log" || true)"
if [[ -n "$plain" ]]; then
  printf '%s\n' "$plain"
  exit 0
fi

# `sort -u` forgives an identical line read twice; a duplicate index with a
# DIFFERENT payload survives it and fails the index check below.
chunks="$(grep '^SEED_MANIFEST_GZ: ' <<<"$log" | sort -u || true)"
[[ -n "$chunks" ]] || die "no SEED_MANIFEST_JSON / SEED_MANIFEST_GZ line in the input."

# One emission has one <n>; two totals means the log mixes two runs.
totals="$(awk '{split($2, c, "/"); print c[2]}' <<<"$chunks" | sort -u)"
[[ "$(wc -l <<<"$totals" | tr -d ' ')" == "1" ]] \
  || die "chunk lines advertise conflicting totals ($(tr '\n' ' ' <<<"$totals")) — the input mixes two emissions."
total="$totals"
[[ "$total" =~ ^[0-9]+$ ]] || die "malformed chunk counter '<i>/$total'."

# SAFETY: exactly the indexes 1..n, each once — a missing, duplicated or
# out-of-range chunk must fail rather than decode a spliced document.
indexes="$(awk '{split($2, c, "/"); print c[1]}' <<<"$chunks" | sort -n)"
[[ "$indexes" == "$(seq 1 "$total")" ]] \
  || die "chunk indexes ($(tr '\n' ' ' <<<"$indexes")) are not exactly 1..$total — the log is truncated or mixes two emissions."

# Sorted numerically by <i>: a merged or filtered log may not preserve order,
# and 10 sorts before 2 as text.
payload="$(awk '{split($2, c, "/"); print c[1], $3}' <<<"$chunks" \
  | sort -n -k1,1 \
  | awk '{printf "%s", $2}')"

# WORKAROUND: `base64 -d` on GNU coreutils, `-D` on macOS.
decode_base64() {
  if base64 -d </dev/null >/dev/null 2>&1; then base64 -d; else base64 -D; fi
}

# SAFETY: decode through an assignment, not into printf's argument list — a
# command substitution used as an argument discards its status, and an empty
# `SEED_MANIFEST_JSON: ` line still matches every consumer's grep.
json="$(printf '%s' "$payload" | decode_base64 | gzip -dc)"
[[ -n "$json" ]] || die "the chunks decoded to an empty document."

printf 'SEED_MANIFEST_JSON: %s\n' "$json"
