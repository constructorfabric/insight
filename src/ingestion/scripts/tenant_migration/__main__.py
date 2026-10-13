"""Move a warm install's stored tenant to the canonical tenant UUID.

    python3 -m tenant_migration plan     --tenant <uuid> --old-tenant <key>
    python3 -m tenant_migration apply    --tenant <uuid> --old-tenant <key> --dump-dir <new dir>
    python3 -m tenant_migration rollback
    python3 -m tenant_migration restore  --dump-dir <dir>
    python3 -m tenant_migration finalize

`--old-tenant` is the tenant segment the install's keys were written with; rows
with an empty tenant move too. Run with connectors and dbt stopped.

`apply` keeps each original until `finalize`. After an interruption anywhere,
`rollback` puts the originals back and `finalize` keeps what is live; both, and
`restore`, read the state from the warehouse and can be run again until they
succeed.

Required env: CLICKHOUSE_URL, CLICKHOUSE_USER, CLICKHOUSE_PASSWORD.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from tenant_migration import apply as migration
from tenant_migration.clickhouse import ClickHouse, ClickHouseError
from tenant_migration.inventory import BACKUP, discover, history_relations
from tenant_migration.keys import TenantError, Tenants
from tenant_migration.plan import Plan, survey
from tenant_migration.prerequisites import fill_youtrack_comment_ids


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="tenant_migration", description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "apply"):
        command = commands.add_parser(name)
        command.add_argument("--tenant", required=True, help="the canonical tenant, a lowercase UUID")
        command.add_argument("--old-tenant", required=True, help="the tenant the stored keys carry now")
        if name == "apply":
            command.add_argument("--dump-dir", required=True, type=Path, help="a new directory for the dump")
    commands.add_parser("rollback")
    commands.add_parser("finalize")
    commands.add_parser("restore").add_argument("--dump-dir", required=True, type=Path)
    return parser


def render(plan: Plan) -> str:
    lines = [f"{'relation':<64} {'rows':>12} {'moving':>12}"]
    lines += [f"{t.table.relation:<64} {t.rows:>12} {t.moving:>12}" for t in plan.tables if t.rows]
    lines.append(f"tables changing: {len(plan.changing)}; YouTrack comment ids to fill: {plan.comment_ids_to_fill}")
    lines += [f"REFUSED {reason}" for reason in plan.refusals]
    return "\n".join(lines)


def _plan(ch: ClickHouse, args: argparse.Namespace) -> Plan:
    tenants = Tenants.parse(args.tenant, args.old_tenant)
    return survey(ch, discover(ch, history_relations()), tenants)


def run(args: argparse.Namespace, ch: ClickHouse) -> int:
    if args.command == "plan":
        plan = _plan(ch, args)
        print(render(plan))
        return 1 if plan.refusals else 0
    if args.command == "apply":
        plan = _plan(ch, args)
        print(render(plan))
        if plan.refusals:
            return 1
        moved = migration.apply(
            ch, plan, args.dump_dir, before_rewrite=[lambda: fill_youtrack_comment_ids(ch, plan.tenants)]
        )
        print(migration.describe(moved))
        print(f"migrated {len(moved)} table(s); originals kept as *{BACKUP} until `finalize` or `rollback`")
        return 0
    if args.command == "rollback":
        print("\n".join(f"rolled back {r}" for r in migration.rollback(ch)) or "nothing to roll back")
        return 0
    if args.command == "restore":
        print("\n".join(f"restored {r}" for r in migration.restore(ch, args.dump_dir)) or "nothing to restore")
        return 0
    print("\n".join(f"dropped {r}" for r in migration.finalize(ch)) or "nothing to finalize")
    return 0


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        return run(args, ClickHouse.from_env())
    except (TenantError, ClickHouseError, migration.MigrationError, FileExistsError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
