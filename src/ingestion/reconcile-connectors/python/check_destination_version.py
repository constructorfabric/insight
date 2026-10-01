#!/usr/bin/env python3
"""Hold the resolved destination-clickhouse version to the floor this repo needs.

CLI:    check_destination_version.py <version>
Stdout: nothing when the version clears the floor; the one-line refusal when
        it does not, for the caller to log verbatim.
Exit:   0 when it clears the floor, 1 when it does not, 2 on bad usage.

Reconcile resolves the destination definition by name, so an installation runs
whatever version its Airbyte carries. Pinning the definition would fight the
operator's own Airbyte upgrades, so this states the requirement instead of
owning it: below the floor, the deploy stops rather than building bronze with
a destination whose behaviour the ingestion design does not describe.
"""

from __future__ import annotations

import re
import sys

#: The release that added `use_replicated_engines` / `cluster_name`, without
#: which a clustered warehouse gets local, unreplicated bronze tables, and the
#: oldest release whose `append_dedup` behaviour this repo's catalogs assume.
#: Independent of the snapshot pin in scripts/bootstrap-db/pins.env, which may
#: only ever be newer.
MINIMUM_VERSION = "2.1.29"

_RELEASE_RE = re.compile(r"^(\d+(?:\.\d+)*)(.*)$")


def _ordered(version: str) -> tuple[tuple[int, ...], int]:
    """A sortable reading of a version; ValueError when it is not one.

    The second element ranks a release above any pre-release that names it,
    so `2.1.29-dev.abc1234` does not satisfy a floor of `2.1.29`.
    """
    match = _RELEASE_RE.match(version.strip())
    if match is None:
        raise ValueError(version)

    return tuple(int(part) for part in match.group(1).split(".")), 0 if match.group(2) else 1


_MINIMUM_ORDER = _ordered(MINIMUM_VERSION)


def refusal(version: str) -> str | None:
    """Why this version may not be used, or None when it may be."""
    try:
        found = _ordered(version)
    except ValueError:
        return (
            f"Airbyte reports destination-clickhouse version {version!r}, which cannot be compared against the "
            f"required minimum {MINIMUM_VERSION} — point this installation at a released destination connector"
        )

    if found < _MINIMUM_ORDER:
        return (
            f"Airbyte carries destination-clickhouse {version}, below the minimum {MINIMUM_VERSION} this "
            "installation requires — upgrade the ClickHouse destination connector before reconciling"
        )

    return None


def main() -> int:
    if len(sys.argv) != 2:
        print("check_destination_version: expected 1 arg (version)", file=sys.stderr)
        return 2

    why = refusal(sys.argv[1])
    if why is None:
        return 0

    print(why)
    return 1


if __name__ == "__main__":
    sys.exit(main())
