#!/usr/bin/env python3
"""Pick one destination definition out of a workspace listing.

Stdin:  the /destination_definitions/list_for_workspace response body.
CLI:    select_destination_definition.py <name>
Stdout: <destinationDefinitionId>\t<dockerImageTag>
Exit:   0 on a match; 1 when no definition carries that name; 2 on bad usage.

The image tag travels beside the id because the definition is resolved by
name at runtime -- an installation runs whatever destination version its
Airbyte happens to carry, and the caller holds that version to a floor.
"""

from __future__ import annotations

import json
import sys
from collections.abc import Iterable, Mapping


def select(definitions: Iterable[Mapping[str, object]], name: str) -> tuple[str, str] | None:
    target = name.lower()
    for definition in definitions:
        listed = definition.get("name") or ""
        definition_id = definition.get("destinationDefinitionId") or ""
        if str(listed).lower() == target and definition_id:
            return str(definition_id), str(definition.get("dockerImageTag") or "")

    return None


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1]:
        print("select_destination_definition: expected 1 non-empty arg (name)", file=sys.stderr)
        return 2

    found = select(json.load(sys.stdin).get("destinationDefinitions", []), sys.argv[1])
    if found is None:
        return 1

    definition_id, image_tag = found
    print(f"{definition_id}\t{image_tag}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
