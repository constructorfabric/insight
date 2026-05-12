#!/usr/bin/env python3
"""Load an Airbyte declarative connector.yaml manifest and emit compact JSON.

Used by lib/airbyte.sh builder-project helpers to send the manifest as a
parsed object in the API request body.
"""
import json
import sys

import yaml


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("load_connector_manifest: expected 1 arg (path to connector.yaml)\n")
        return 2
    path = sys.argv[1]
    try:
        with open(path) as f:
            obj = yaml.safe_load(f)
    except FileNotFoundError:
        sys.stderr.write(f"load_connector_manifest: not found: {path}\n")
        return 2
    except OSError as e:
        # PermissionError, IsADirectoryError, and other I/O failures
        # are indistinguishable from "missing" for the caller (a
        # downstream pod cannot recover without operator intervention
        # either way), so map them to the same exit code instead of
        # letting Python emit a traceback.
        sys.stderr.write(f"load_connector_manifest: cannot read {path}: {e}\n")
        return 2
    except yaml.YAMLError as e:
        sys.stderr.write(f"load_connector_manifest: YAML parse error in {path}: {e}\n")
        return 2
    if not isinstance(obj, dict):
        sys.stderr.write(f"load_connector_manifest: top-level not a mapping: {path}\n")
        return 2
    json.dump(obj, sys.stdout, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
