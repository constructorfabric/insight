#!/usr/bin/env python3
"""The source one instance was created under before the install's tenant changed.

CLI:    find_former_tenant_source.py <definition_id> <connector> <source_id> <tenant>
Stdin:  the `sources/list` payload.
Stdout: TSV `airbyte_source_id<TAB>former_tenant<TAB>name`, or nothing.
Exit:   0 found or none; 1 refused (reason on stderr); 2 bad arg count.

A source is named `{connector}-{source_id}-{tenant}`, so a new tenant names every
instance anew, and reconcile would create a second source beside the first — a
fresh sync history and a duplicate schedule. The former one is renamed instead,
which keeps its id and with it the connection and its sync state.

The former tenant is read from the source's own configuration, never from its
name: `insight_tenant_id` is what the source was configured with, and the name
must then spell exactly that. Reading it out of the name would have to guess
where the source id ends — `main` and `main-eu` both prefix `main-eu-…`.
"""

from __future__ import annotations

import json
import sys

SUBJECT = "find_former_tenant_source"


def _text(item: dict[str, object], key: str) -> str:
    value = item.get(key)
    return value if isinstance(value, str) else ""


def _former(
    item: object, definition_id: str, connector: str, source_id: str, tenant: str
) -> tuple[str, str, str] | None:
    if not isinstance(item, dict) or _text(item, "sourceDefinitionId") != definition_id:
        return None
    config = item.get("connectionConfiguration")
    if not isinstance(config, dict) or _text(config, "insight_source_id") != source_id:
        return None
    former = _text(config, "insight_tenant_id")
    name = _text(item, "name")
    if not former or former == tenant or name != f"{connector}-{source_id}-{former}":
        return None
    return _text(item, "sourceId"), former, name


def main() -> int:
    if len(sys.argv) != 5:
        sys.stderr.write(f"{SUBJECT}: expected 4 args (definition_id, connector, source_id, tenant)\n")
        return 2
    definition_id, connector, source_id, tenant = sys.argv[1:]
    payload = json.load(sys.stdin)
    if not isinstance(payload, list):
        sys.stderr.write(f"{SUBJECT}: the payload is not a list of sources\n")
        return 1

    expected = f"{connector}-{source_id}-{tenant}"
    found = [f for item in payload if (f := _former(item, definition_id, connector, source_id, tenant)) and f[0]]
    if not found:
        return 0
    if len(found) > 1:
        names = ", ".join(name for _, _, name in found)
        sys.stderr.write(f"{SUBJECT}: {expected} has several former sources ({names}); rename one by hand\n")
        return 1
    if any(isinstance(item, dict) and _text(item, "name") == expected for item in payload):
        sys.stderr.write(f"{SUBJECT}: both {found[0][2]} and {expected} exist; keep one by hand\n")
        return 1

    print("\t".join(found[0]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
