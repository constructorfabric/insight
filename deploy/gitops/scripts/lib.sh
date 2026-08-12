# shellcheck shell=bash
# lib.sh — shared guard/helper library for deploy/gitops/scripts/*. Sourced
# only; guards act through a global KUBECTL array the caller assembles.
# shellcheck disable=SC2034  # colour vars are used by the sourcing scripts

if [ -t 1 ] && [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_CYA=$'\033[36m'; C_RST=$'\033[0m'
else
  C_RED=""; C_GRN=""; C_YEL=""; C_CYA=""; C_RST=""
fi

hdr()  { printf '\n%s── %s %s\n' "$C_CYA" "$1" "$C_RST" >&2; }
note() { printf '  %s\n' "$1" >&2; }
ok()   { printf '  %sok%s    %s\n' "$C_GRN" "$C_RST" "$1" >&2; }
die()  { printf '%sERROR%s: %s\n' "$C_RED" "$C_RST" "$1" >&2; exit 1; }

# cluster_guard CONTEXT EXPECT_CLUSTER — exits 2, not a return: this always
# runs at the caller's top level, so `exit` here ends the whole process.
cluster_guard() {
  local context="$1" expect="$2"
  [ -n "$expect" ] || die "cluster_guard: no expected cluster given"
  ACTUAL_CLUSTER="$("${KUBECTL[@]}" config view -o "jsonpath={.contexts[?(@.name==\"${context}\")].context.cluster}" 2>/dev/null || true)"
  [ -n "$ACTUAL_CLUSTER" ] || die "context '$context' is not present in this kubeconfig"
  if [ "$ACTUAL_CLUSTER" != "$expect" ]; then
    printf '%s\n' "$C_RED" >&2
    printf '  ┌──────────────────────────────────────────────────────────────┐\n' >&2
    printf '  │  REFUSING TO ACT — CLUSTER MISMATCH                          │\n' >&2
    printf '  └──────────────────────────────────────────────────────────────┘%s\n' "$C_RST" >&2
    printf '    context           : %s\n' "$context" >&2
    printf '    resolves to       : %s\n' "$ACTUAL_CLUSTER" >&2
    printf '    expected cluster  : %s\n\n' "$expect" >&2
    printf '  Nothing was read, deleted or written.\n' >&2
    exit 2
  fi
}

# namespace_assert NAMESPACE — dies if it does not exist or is not readable.
namespace_assert() {
  local namespace="$1"
  "${KUBECTL[@]}" get namespace "$namespace" -o name >/dev/null 2>&1 \
    || die "namespace '$namespace' does not exist or is not readable with this credential"
}

# oidc_secret_nonempty NAMESPACE SECRET KEY — returns 1 instead of dying, so
# a caller can fold it into a wider missing-prereqs count.
oidc_secret_nonempty() {
  local namespace="$1" secret="$2" key="$3"
  if [ -z "$("${KUBECTL[@]}" -n "$namespace" get secret "$secret" -o "jsonpath={.data['${key}']}" 2>/dev/null)" ]; then
    note "${C_RED}EMPTY${C_RST}   secret/$secret has no '$key' value"
    return 1
  fi
  note "${C_GRN}ok${C_RST}    secret/$secret carries a non-empty $key"
  return 0
}
