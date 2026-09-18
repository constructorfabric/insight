#!/usr/bin/env python3
"""Resolve the pod network for gateway.setRealIpFrom.

Prints the CIDR on stdout, progress on stderr. Env: KUBE_CTX,
ENVOY_GATEWAY_NAME (optional).

SAFETY: the trusted set must exclude every address a client could hold --
real_ip_recursive skips trusted hops, so a trusted client address lets a forged
entry left of it win. Pod network only, never RFC1918.
"""

import ipaddress
import os
import subprocess
import sys

WIDEN_TO_PREFIX = 16
MIN_PREFIX = 8


def fail(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    raise SystemExit(1)


def skip(message: str) -> None:
    """Undeterminable is not the same as wrong: a deployer scoped to one
    namespace cannot read cluster-scoped nodes, and blocking it would be worse
    than the unset trust list it already deploys with."""
    print(f"WARNING: {message}; leaving gateway.setRealIpFrom unset", file=sys.stderr)
    raise SystemExit(0)


def kubectl(ctx: str, *args: str) -> list[str]:
    result = subprocess.run(
        ["kubectl", "--context", ctx, *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return []
    return [line for line in result.stdout.split("\n") if line]


def node_pod_cidrs(ctx: str) -> list[ipaddress.IPv4Network | ipaddress.IPv6Network]:
    raw = kubectl(
        ctx,
        "get",
        "nodes",
        "-o",
        'jsonpath={range .items[*]}{.spec.podCIDR}{"\\n"}{end}',
    )
    return [ipaddress.ip_network(c, strict=False) for c in raw]


def envoy_pod_ips(ctx: str, gateway_name: str) -> list[str]:
    selector = "gateway.envoyproxy.io/owning-gateway-name"
    if gateway_name:
        selector = f"{selector}={gateway_name}"
    return kubectl(
        ctx,
        "get",
        "pods",
        "-A",
        "-l",
        selector,
        "-o",
        'jsonpath={range .items[*]}{.status.podIP}{"\\n"}{end}',
    )


def enclosing_network(slices):
    """Widened past the per-node slices, which go stale as soon as a node is
    added and routegen only re-reads at gateway startup."""
    merged = slices[0]
    while not all(s.subnet_of(merged) for s in slices):
        merged = merged.supernet()
    if merged.prefixlen > WIDEN_TO_PREFIX:
        merged = merged.supernet(new_prefix=WIDEN_TO_PREFIX)
    return merged


def main() -> None:
    ctx = os.environ.get("KUBE_CTX")
    if not ctx:
        fail("KUBE_CTX is required")

    slices = node_pod_cidrs(ctx)
    if not slices:
        skip(f"cannot read node podCIDR from cluster '{ctx}'")

    network = enclosing_network(slices)
    if not network.is_private:
        fail(f"computed pod network {network} is not private")
    if network.prefixlen < MIN_PREFIX:
        fail(f"computed pod network {network} is implausibly wide")

    uncovered = [
        ip
        for ip in envoy_pod_ips(ctx, os.environ.get("ENVOY_GATEWAY_NAME", ""))
        if ipaddress.ip_address(ip) not in network
    ]
    if uncovered:
        fail(f"pod network {network} does not cover Envoy pods: {' '.join(uncovered)}")

    joined = " ".join(str(s) for s in slices)
    print(f"gateway client-IP trust: {network} (node slices: {joined})", file=sys.stderr)
    print(network)


if __name__ == "__main__":
    main()
