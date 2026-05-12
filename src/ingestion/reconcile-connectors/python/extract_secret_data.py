#!/usr/bin/env python3
"""Extract and decode a K8s Secret's stringData or data fields to a JSON object.

Reads `kubectl get secret -o json` from stdin.
Prefers `stringData`; falls back to base64-decoded `data`.
Decoded values are attempted as JSON; non-JSON scalars kept as strings.

CLI:
  kubectl get secret NAME -o json | extract_secret_data.py

Stdout: JSON object (key → decoded value).
Exit:   0 always.
"""
import argparse, base64, binascii, json, sys

def main() -> int:
    p = argparse.ArgumentParser()
    p.parse_args()
    secret = json.load(sys.stdin)
    string_data = secret.get("stringData") or {}
    if string_data:
        json.dump(string_data, sys.stdout)
        return 0
    data = secret.get("data") or {}
    out = {}
    for k, v in data.items():
        try:
            decoded = base64.b64decode(v).decode()
        except (binascii.Error, UnicodeDecodeError):
            decoded = v
        try:
            parsed = json.loads(decoded)
            out[k] = parsed if isinstance(parsed, (list, dict)) else decoded
        except json.JSONDecodeError:
            out[k] = decoded
    json.dump(out, sys.stdout)
    return 0

if __name__ == "__main__":
    sys.exit(main())
