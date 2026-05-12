#!/usr/bin/env python3
"""Resolve tenant slug for a given K8s Secret.

CLI:
  resolve_tenant.py --secret-json PATH

Reads the Secret's `metadata.labels."insight.cyberfabric.com/tenant"`.
Falls back to `metadata.namespace` if the label is missing.

Stdout: tenant slug (no trailing newline).
Exit:   0 success, 2 if label missing AND namespace missing.
"""
import argparse, json, sys

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--secret-json", required=True)
    args = p.parse_args()
    with open(args.secret_json, "r", encoding="utf-8") as f:
        sec = json.load(f)
    md = sec.get("metadata", {})
    labels = md.get("labels", {}) or {}
    tenant = labels.get("insight.cyberfabric.com/tenant") or md.get("namespace")
    if not tenant:
        print("resolve_tenant: cannot resolve tenant (label and namespace missing)", file=sys.stderr)
        return 2
    sys.stdout.write(tenant)
    return 0

if __name__ == "__main__":
    sys.exit(main())
