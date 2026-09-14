#!/usr/bin/env python3
"""Iterate K8s Secret list JSON and emit per-secret TSV with cfg-hash.

Stdin:  `kubectl get secret -l ... -o json` payload (an Items list).
Stdout: TSV `connector\tsource_id\tsecret_name\tcfg_hash` per labelled secret.
        A secret with no connector annotation emits a WARN to stderr and is
        skipped — it names nothing this loop manages.
Exit:   0 always; 1 on JSON decode error.

A secret that names its connector but no source id is that connector's `main`
instance, which is the id its Airbyte source and connection are already named
by. Skipping it instead would take the connector out of the desired state
entirely, and a connector absent from the desired state is one the reconcile
loop deletes.

The cfg-hash is computed as a canonical SHA-256 of the secret's `.data` map
matching `compute_cfg_hash.py` byte-for-byte (keys sorted, base64 verbatim).
"""
import hashlib
import json
import sys
from typing import Any, Dict

#: INVARIANT: the same fallback `reconcile_compute_source_id` applies. Two
#: places deciding what an unannotated instance is called is two names for one
#: thing — and the ledger would then record it under a different id from the one
#: its Airbyte source carries.
DEFAULT_SOURCE_ID = "main"


def _canonical_cfg_hash(secret_data: Dict[str, Any]) -> str:
    canonical = json.dumps(secret_data, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError as exc:
        sys.stderr.write(f"extract_secret_loop: bad JSON on stdin: {exc}\n")
        return 1
    for item in data.get("items", []):
        md = item.get("metadata", {}) or {}
        name: str = md.get("name", "")
        annotations: Dict[str, Any] = md.get("annotations", {}) or {}
        connector = annotations.get("insight.cyberfabric.com/connector")
        if not connector:
            sys.stderr.write(f"WARN: secret {name} missing connector annotation, skip\n")
            continue
        source_id = annotations.get("insight.cyberfabric.com/source-id") or DEFAULT_SOURCE_ID
        secret_data: Dict[str, Any] = item.get("data", {}) or {}
        cfg_hash = _canonical_cfg_hash(secret_data)
        print(f"{connector}\t{source_id}\t{name}\t{cfg_hash}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
