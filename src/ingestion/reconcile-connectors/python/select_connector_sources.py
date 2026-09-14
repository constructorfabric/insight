#!/usr/bin/env python3
"""Which Airbyte sources belong to one connector, and which instance each is.

CLI:
  select_connector_sources.py <connector> <tenant> <known_connectors_file> \
                              <definitions_file>

Args:   `known_connectors_file` holds `extract_descriptor_names.py` output — the
        JSON array of every connector this build ships. `definitions_file` holds
        `ab_list_definitions` output, which is what says whose a source is.
Stdin:  the `sources/list` payload (a JSON array of source objects).
Stdout: TSV `airbyte_source_id<TAB>instance_source_id` per matching source.
Exit:   0 always; 2 on bad arg count, 1 on an unreadable listing or file.

Sources are named `{connector}-{source_id}-{tenant}` by the reconcile loop, so
the instance's own id is what is left once the two known ends are removed. A
source whose name does not carry both ends is emitted with an empty instance id
rather than dropped: the caller still has to delete it, and guessing an id for
it would name an instance that never existed.

INVARIANT: ownership is `airbyte_sources.owner_of` — the definition, and the
name only as a fail-closed fallback. This pass runs on the way to deleting what
it selects.
"""

import sys

from airbyte_sources import decode, load_definitions, owner_of, read_json_list

SUBJECT = "select_connector_sources"


def instance_of(name: str, connector: str, tenant: str) -> str:
    """The instance id inside a source name, or empty when it carries none."""
    head = f"{connector}-"
    tail = f"-{tenant}"
    if not name.startswith(head) or not name.endswith(tail):
        return ""
    return name[len(head) : len(name) - len(tail)]


def main() -> int:
    if len(sys.argv) != 5:
        sys.stderr.write(
            f"{SUBJECT}: expected <connector> <tenant> <known_connectors_file> "
            "<definitions_file>\n"
        )
        return 2
    connector, tenant = sys.argv[1], sys.argv[2]

    listed = read_json_list(sys.argv[3], SUBJECT, "connector list")
    definitions = load_definitions(sys.argv[4], SUBJECT)
    if listed is None or definitions is None:
        return 1
    # The connector being cascaded is one of them whether or not the caller's
    # listing named it, or nothing it owns would ever be selected.
    known = {name for name in listed if isinstance(name, str) and name} | {connector}

    sources = decode(sys.stdin, SUBJECT)
    if sources is None:
        return 1

    for source in sources:
        if owner_of(source, definitions, known) != connector:
            continue
        print(f"{source.source_id}\t{instance_of(source.name, connector, tenant)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
