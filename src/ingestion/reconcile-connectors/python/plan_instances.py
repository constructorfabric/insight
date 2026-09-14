#!/usr/bin/env python3
"""Join the descriptors this build ships with the instances this install has.

CLI:
  plan_instances.py <descriptors_tsv> <secrets_tsv>

Inputs are files, not stdin, because there are two of them and neither is
small enough to want in argv.

  descriptors_tsv  `disc_load_descriptors` output:
                   name, connector_dir, version, type, cdk_image, enrich_image,
                   dbt_select, namespace
  secrets_tsv      `disc_load_secrets` output:
                   connector, source_id, secret_name, cfg_hash

Stdout: one TSV row per connector INSTANCE, the descriptor's eight columns
        followed by source_id, secret_name, cfg_hash.
Exit:   0 success; 2 on bad arg count; 3 when two Secrets claim one instance.

A descriptor with no Secret emits one row with the three instance columns empty.
That row is what the reconcile loop reads as "not installed here", and it has to
be emitted rather than dropped: the cascade that removes such a connector's
leftovers is driven by walking the plan, so a descriptor missing from it would
leave its sources and schedules behind for ever.

INVARIANT: two Secrets claiming one (connector, source_id) is refused, not
resolved. Which of them won would depend on the order the API returned them, so
one tick would reconcile the instance towards one Secret's credentials and the
next towards the other's — and nothing in either tick would look wrong.

A Secret naming a connector this build does not ship is reported and otherwise
ignored: there is no descriptor to reconcile it against, and inventing one would
put a connector on the page that no version of the product has.
"""

import sys
from collections import defaultdict
from pathlib import Path

DESCRIPTOR_COLUMNS = 8

#: What an uninstalled descriptor carries where an instance would be.
NOT_INSTALLED = ("", "", "")


def _rows(path: str, columns: int) -> list[list[str]]:
    """TSV rows padded to `columns`, skipping blank lines.

    Padded rather than validated: a trailing empty field is dropped by every
    writer that joins on tabs, and a row short by one would otherwise shift
    every column after it.
    """
    text = Path(path).read_text(encoding="utf-8")
    rows = []
    for line in text.splitlines():
        if not line.strip():
            continue
        fields = line.split("\t")
        fields.extend([""] * (columns - len(fields)))
        rows.append(fields[:columns])
    return rows


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("plan_instances: expected <descriptors_tsv> <secrets_tsv>\n")
        return 2

    descriptors = _rows(sys.argv[1], DESCRIPTOR_COLUMNS)
    secrets = _rows(sys.argv[2], 4)

    known = {row[0] for row in descriptors}
    by_connector: dict[str, dict[str, tuple[str, str]]] = defaultdict(dict)
    for connector, source_id, secret_name, cfg_hash in secrets:
        if connector not in known:
            sys.stderr.write(
                f"WARN: secret {secret_name} names connector {connector}, "
                "which this build does not ship; ignoring it\n"
            )
            continue
        claimed = by_connector[connector].get(source_id)
        if claimed is not None:
            sys.stderr.write(
                f"ERROR: secrets {claimed[0]} and {secret_name} both claim "
                f"{connector} instance {source_id!r}; which one is reconciled "
                "would depend on the order the API listed them\n"
            )
            return 3
        by_connector[connector][source_id] = (secret_name, cfg_hash)

    # Sorted so one tick's work is in the same order as the next one's: the log
    # is read by comparing ticks, and an order that moves makes that impossible.
    for descriptor in sorted(descriptors, key=lambda row: row[0]):
        instances = sorted(by_connector.get(descriptor[0], {}).items())
        if not instances:
            print("\t".join([*descriptor, *NOT_INSTALLED]))
            continue
        for source_id, (secret_name, cfg_hash) in instances:
            print("\t".join([*descriptor, source_id, secret_name, cfg_hash]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
