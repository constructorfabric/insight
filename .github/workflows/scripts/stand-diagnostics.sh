#!/usr/bin/env bash
# Curated, redacted evidence from a deployed stand after something went wrong.
#
# This is the whole of what a failed test-stand run publishes about the cluster.
# It is an allowlist, not a filter over "everything": each section below was
# chosen because it answers a question a red run actually raises, and anything
# not on the list is absent by decision rather than by omission.
#
#   release   which chart version is installed, whether helm thinks the release
#             is deployed or failed, and the last few revisions with the
#             description helm wrote — that description is where "timed out
#             waiting for the condition" lives.
#   pods      name, phase, ready, restart count, and the waiting/terminated
#             reason. Enough to tell CrashLoopBackOff from ImagePullBackOff from
#             a pod that never got scheduled.
#   warnings  Warning events as (reason, kind, name, count). REASONS ONLY — the
#             message body is where a scheduler or a kubelet quotes back
#             whatever it was handed, including image references, node names and
#             occasionally the content of a Secret volume it could not mount.
#   logs      the last 50 lines of containers that are not healthy, and the
#             previous container's tail when one has restarted. Everything goes
#             through redact-stand-log.py.
#
# What is deliberately NOT here, and must stay that way:
#
#   * `kubectl describe` — it prints the pod spec, which prints every
#     environment variable name AND the Secret/ConfigMap each one is bound to,
#     plus node names, image digests and volume paths. It is the single most
#     common way a public CI log becomes an infrastructure map.
#   * `kubectl get ... -o wide` / `-o yaml` / `-o json` dumps — same reason, plus
#     node IPs.
#   * anything reading a Secret or a ConfigMap, even by name-only listing.
#   * artifact uploads. A run log is public but ephemeral in attention; an
#     uploaded archive is a downloadable copy of whatever slipped through.
#   * `env`, `printenv`, `set -x` over a step that carries credentials.
#
# Every section is best-effort and this script ALWAYS exits 0. It runs after a
# stage has already failed; a diagnostic that fails is a missing paragraph, not
# a second failure, and turning it into one would relabel every red run with the
# wrong cause.
#
# Usage:
#   stand-diagnostics.sh <namespace> <release> [max-containers]
#
# Reads the ambient kubeconfig ($KUBECONFIG or ~/.kube/config) — the same
# context the deploy itself acted on. Runnable by hand: a human debugging the
# stand gets exactly the view CI got, which is the point of it being a committed
# script rather than an inline block in the workflow.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REDACT="$SCRIPT_DIR/redact-stand-log.py"

NAMESPACE="${1:-}"
RELEASE="${2:-}"
# A stand with a dozen sick pods produces a wall of text nobody reads, and the
# first few are almost always the same failure. Bounded, and the bound is
# reported when it bites.
MAX_CONTAINERS="${3:-8}"

if [[ -z "$NAMESPACE" || -z "$RELEASE" ]]; then
  echo "usage: stand-diagnostics.sh <namespace> <release> [max-containers]" >&2
  exit 0
fi

if [[ ! -f "$REDACT" ]]; then
  echo "::warning::redaction filter missing at $REDACT — refusing to print cluster output" >&2
  exit 0
fi

# Nothing reaches the console except through this. Written as a function so a
# new section cannot accidentally forget the pipe: every `kubectl`/`helm` call
# below ends in `| redact`.
redact() {
  python3 "$REDACT"
}

# A short timeout everywhere: this runs when the stand is already unwell, and an
# apiserver that has stopped answering must not hold the runner for ten minutes.
kube() {
  kubectl --request-timeout=20s -n "$NAMESPACE" "$@" 2>&1
}

section() {
  echo "::group::$1"
}

endsection() {
  echo "::endgroup::"
}

if ! kubectl --request-timeout=10s version --output=json >/dev/null 2>&1; then
  echo "::warning::no reachable cluster — diagnostics skipped (the failure is upstream of the deploy)"
  exit 0
fi

# ── release ─────────────────────────────────────────────────────────────────
# `helm list` and `helm history`, deliberately NOT `helm status`: status's
# document also carries `.info.notes` (the rendered NOTES.txt, which quotes
# hostnames and sign-in URLs) and the rendered manifest. The two commands used
# here have small, stable documents with nothing in them but identity and
# outcome — and `history`'s `description` is where helm records "Upgrade
# \"insight\" failed: timed out waiting for the condition", which is the single
# most useful line a failed deploy produces.
#
# The status flags are spelled out one by one rather than using `--all`, and
# stderr is NOT folded into the pipe. Both were real bugs:
#
#   * `helm list --all` is a Helm 3 flag. Helm 4 removed it, so on a runner (or
#     a laptop) carrying helm v4 the command exits with `Error: unknown flag:
#     --all` and prints nothing useful. The explicit set below means the same
#     thing and parses on both majors, which is what a diagnostics script has to
#     do — it runs precisely when something is already wrong, so it can never be
#     the thing that fails.
#   * `... 2>&1 | jq` fed helm's error text INTO jq, which then died with
#     `parse error: Invalid numeric literal` and swallowed the actual message.
#     A diagnostic that hides the diagnosis is worse than no diagnostic; stderr
#     now reaches the log on its own.
#
# The intent the flags preserve: the default listing hides a release that is
# neither deployed nor failed, and `pending-upgrade` — the state a killed
# upgrade leaves behind — is exactly the one worth seeing.
section "helm release"
if command -v jq >/dev/null 2>&1; then
  helm list -n "$NAMESPACE" \
      --deployed --failed --pending --uninstalling \
      --filter "^${RELEASE}\$" -o json |
    jq -r '
      if type == "array" and length > 0 then
        (.[0] | "release:    \(.name)",
                "namespace:  \(.namespace)",
                "revision:   \(.revision)",
                "status:     \(.status)",
                "chart:      \(.chart)",
                "appVersion: \(.app_version)",
                "updated:    \(.updated)")
      else
        "no release named \($rel) in this namespace"
      end' --arg rel "$RELEASE" | redact
else
  echo "jq is not on PATH — release summary skipped" | redact
fi
endsection

section "helm history (last 5 revisions)"
if command -v jq >/dev/null 2>&1; then
  helm history "$RELEASE" -n "$NAMESPACE" -o json |
    jq -r '
      if type == "array" then
        (sort_by(.revision) | reverse | .[0:5][]
          | "rev \(.revision)  \(.status)  \(.chart)  \(.updated)  \(.description // "")")
      else
        "helm history returned no revisions"
      end' | redact
else
  echo "jq is not on PATH — history skipped" | redact
fi
endsection

# ── pods ────────────────────────────────────────────────────────────────────
# custom-columns rather than `-o wide`: wide adds the node name and the pod IP,
# which are infrastructure detail this repo does not publish, and adds nothing
# to "which container is unhappy".
section "pods"
kube get pods \
  -o 'custom-columns=NAME:.metadata.name,PHASE:.status.phase,READY:.status.containerStatuses[*].ready,RESTARTS:.status.containerStatuses[*].restartCount,WAITING:.status.containerStatuses[*].state.waiting.reason,TERMINATED:.status.containerStatuses[*].state.terminated.reason' |
  redact
endsection

# ── warning events ──────────────────────────────────────────────────────────
# Reason and involved object only. The MESSAGE column is deliberately absent —
# see the header. Sorted oldest-first by the apiserver, so the tail is the
# recent end.
section "warning events (reasons only, no messages)"
kube get events --field-selector type=Warning --sort-by=.lastTimestamp \
  -o 'custom-columns=LAST:.lastTimestamp,REASON:.reason,KIND:.involvedObject.kind,OBJECT:.involvedObject.name,COUNT:.count' |
  tail -n 25 | redact
endsection

# ── logs of unhealthy containers ────────────────────────────────────────────
# "Unhealthy" is: not ready, or restarted at least once, or currently waiting,
# or terminated with a non-zero exit code. Init containers count — a stand that
# fails its migration hook never reaches its app containers at all.
section "tail of unhealthy containers"
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is not on PATH — container selection skipped" | redact
  endsection
  exit 0
fi

targets="$(
  kube get pods -o json 2>/dev/null |
    jq -r '
      (.items // [])[]
      | .metadata.name as $pod
      | ((.status.containerStatuses // []) + (.status.initContainerStatuses // []))[]
      | select(
          (.ready != true)
          or ((.restartCount // 0) > 0)
          or (.state.waiting != null)
          or ((.state.terminated.exitCode // 0) != 0)
        )
      | "\($pod) \(.name) \(.restartCount // 0)"
    ' 2>/dev/null
)"

if [[ -z "$targets" ]]; then
  echo "every container is ready with no restarts — the failure is not in a pod's log" | redact
  endsection
  exit 0
fi

printed=0
while read -r pod container restarts; do
  [[ -n "$pod" ]] || continue
  if [[ "$printed" -ge "$MAX_CONTAINERS" ]]; then
    echo "…more unhealthy containers than the $MAX_CONTAINERS this prints; the rest are in the pod table above" | redact
    break
  fi
  printed=$((printed + 1))

  echo "----- $pod/$container (restarts: $restarts) -----" | redact
  # --limit-bytes as well as --tail: a container logging one enormous line
  # would otherwise defeat the line count, and the point of the cap is to bound
  # what reaches a public log rather than to bound the line count as such.
  kube logs "$pod" -c "$container" --tail=50 --limit-bytes=20000 | redact

  if [[ "${restarts:-0}" -gt 0 ]]; then
    echo "----- $pod/$container (previous container) -----" | redact
    kube logs "$pod" -c "$container" --previous --tail=50 --limit-bytes=20000 | redact
  fi
done <<<"$targets"
endsection

exit 0
