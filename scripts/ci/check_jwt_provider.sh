#!/usr/bin/env bash
# Assert that exactly ONE jsonwebtoken crypto provider is active in the unified
# backend build.
#
# jsonwebtoken v10 has two mutually-exclusive crypto backends selected by
# feature: `aws_lc_rs` and `rust_crypto`. Cargo unifies features across every
# workspace member that shares a build (the release images compile api-gateway
# + analytics + authenticator together), so a single dependency enabling the
# "other" provider silently turns BOTH on — which makes `jsonwebtoken::verify`
# panic on every call (incident #1725: a runtime 500 on gateway auth that all
# per-crate unit tests and the standalone authenticator e2e missed).
#
# This guard turns that feature-unification trap into a fast, red CI check
# (issue #1727, part 3). It does not compile anything — `cargo tree` only reads
# the resolved dependency graph.
set -euo pipefail

MANIFEST="${1:-src/backend/Cargo.toml}"

# The two mutually-exclusive provider features. Keep in sync with jsonwebtoken's
# Cargo features if it gains/renames a backend.
readonly PROVIDERS=(aws_lc_rs rust_crypto)

# Feature-inverted tree: every jsonwebtoken feature the unified build enables.
tree="$(cargo tree --manifest-path "$MANIFEST" --edges features --invert jsonwebtoken)"

active=()
for p in "${PROVIDERS[@]}"; do
    # Match a node like: jsonwebtoken feature "aws_lc_rs"
    if grep -qE "jsonwebtoken feature \"${p}\"" <<<"$tree"; then
        active+=("$p")
    fi
done

if [ "${#active[@]}" -ne 1 ]; then
    echo "FAIL: expected exactly ONE jsonwebtoken crypto provider, found ${#active[@]}: ${active[*]:-none}" >&2
    echo "More than one provider makes jsonwebtoken::verify panic at runtime (see #1725)." >&2
    echo "---- cargo tree --edges features --invert jsonwebtoken ----" >&2
    echo "$tree" >&2
    exit 1
fi

echo "OK: exactly one jsonwebtoken crypto provider active: ${active[0]}"
