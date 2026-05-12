#!/usr/bin/env bash
# Patch CoreDNS in a Kind cluster to use public DNS upstreams (8.8.8.8 / 8.8.4.4)
# instead of the host's `/etc/resolv.conf`.
#
# WHY:
#   On WSL2, /etc/resolv.conf forwards to a WSL-internal nameserver that does
#   not reliably resolve external domains. As a result, pods inside the Kind
#   cluster fail DNS for outbound calls (api.anthropic.com, api.zoom.us,
#   login.microsoftonline.com, atlassian.net, …) — every Airbyte sync to an
#   external API breaks, and OIDC validation in the API Gateway fails.
#
#   On non-WSL Linux this is rarely needed (the host resolv.conf is usually
#   correct), but the patch is idempotent and gated on detecting the default
#   `forward . /etc/resolv.conf` line, so it is a no-op once applied or when
#   CoreDNS already uses a different upstream.
#
# WHEN:
#   Called from `dev-up.sh` after the Kind cluster is up and `kubectl wait
#   --for=condition=Ready node` has succeeded. Safe to call repeatedly.
#
# OPT-OUT:
#   Set SKIP_COREDNS_PATCH=1 to skip (e.g. on corporate networks where
#   8.8.8.8 is firewalled — set DNS_UPSTREAMS=10.0.0.1 to override instead).
#
# OVERRIDES:
#   DNS_UPSTREAMS — space-separated upstreams (default: "8.8.8.8 8.8.4.4")
set -euo pipefail

if [[ "${SKIP_COREDNS_PATCH:-0}" == "1" ]]; then
  exit 0
fi

DNS_UPSTREAMS="${DNS_UPSTREAMS:-8.8.8.8 8.8.4.4}"

# Idempotency gate: only patch when CoreDNS still uses the default
# /etc/resolv.conf upstream. After the first run this returns nothing.
if ! kubectl get configmap coredns -n kube-system -o yaml 2>/dev/null \
    | grep -q "forward . /etc/resolv.conf"; then
  exit 0
fi

echo "  Patching CoreDNS to use public DNS (${DNS_UPSTREAMS})..."
kubectl get configmap coredns -n kube-system -o yaml \
  | sed "s|forward \\. /etc/resolv.conf|forward . ${DNS_UPSTREAMS}|" \
  | kubectl apply -f - >/dev/null
kubectl rollout restart deployment/coredns -n kube-system >/dev/null
kubectl rollout status deployment/coredns -n kube-system --timeout=60s >/dev/null || true
