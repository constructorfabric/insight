"""
Insight sample-data seed — orchestrator.

Subcommands:
    identity   Persons, org_chart, account_person_map (MariaDB).
    silver     CREATE silver tables + apply gold view migrations + INSERT
               sample rows (ClickHouse). Phase 2 — placeholder for now.
    all        Run every step.

See compose/seed/README.md for the ruff/mypy/venv setup and the
per-domain generators under compose/seed/generators/ for the data
shape each one emits.
"""

from __future__ import annotations

import argparse


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("identity", help="MariaDB identity seed")
    sub.add_parser("silver", help="ClickHouse silver seed (Phase 2 — placeholder)")
    sub.add_parser("all", help="run every step")
    args = parser.parse_args(argv)

    if args.cmd in ("identity", "all"):
        from identity import run as run_identity

        run_identity()

    if args.cmd in ("silver", "all"):
        from silver import run as run_silver

        run_silver()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
