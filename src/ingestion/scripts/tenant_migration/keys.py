"""How one row's tenant and key move to the new tenant, as ClickHouse expressions.

A key carries its tenant as the leading segment, `{tenant}{sep}{source}{sep}…`
with `-` or `:` as the separator, or — `bronze_bamboohr.whos_out` — as the first
element of a JSON array `["{tenant}","{source}"]`. Only that segment changes, and
only when it is followed by the row's own source: the source column is what tells
where a tenant that itself contains a separator ends.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SEPARATORS = ("-", ":")


class TenantError(ValueError):
    pass


@dataclass(frozen=True)
class Tenants:
    """The tenant rows move to, and the ones they move from: the old key and empty."""

    new: str
    old: str

    @classmethod
    def parse(cls, new: str, old: str) -> Tenants:
        if not UUID.match(new):
            raise TenantError(f"the new tenant must be a lowercase UUID, got {new!r}")
        if old == new:
            raise TenantError("the old tenant equals the new one: there is nothing to move")
        if "'" in old or "\\" in old:
            raise TenantError("the old tenant must contain no quote or backslash")
        return cls(new=new, old=old)

    @property
    def moving(self) -> tuple[str, ...]:
        return tuple(dict.fromkeys((self.old, "")))


@dataclass(frozen=True)
class Columns:
    """The columns of one table the rewrite reads; `key` and `source` may be absent."""

    tenant: str
    source: str | None
    key: str | None


def _lit(value: str) -> str:
    return "'" + value + "'"


def tenant_value(column: str) -> str:
    return f"ifNull(toString({column}), '')"


def _forms(tenant: str, key: str, source: str, new: str) -> list[tuple[str, str]]:
    """(the key carries `tenant` followed by the row's source, that key carrying `new` instead)."""
    forms = []
    rest = f"substring({key}, {len(tenant.encode()) + 1})"
    for sep in SEPARATORS:
        prefix = f"concat({_lit(tenant)}, '{sep}', {source}, '{sep}')"
        exact = f"concat({_lit(tenant)}, '{sep}', {source})"
        forms.append((f"(startsWith({key}, {prefix}) OR {key} = {exact})", f"concat({_lit(new)}, {rest})"))
    forms.append((f"{key} = toJSONString([{_lit(tenant)}, {source}])", f"toJSONString([{_lit(new)}, {source}])"))
    return forms


def _key_and_source(columns: Columns) -> tuple[str, str]:
    assert columns.key is not None and columns.source is not None
    return f"ifNull({columns.key}, '')", f"ifNull(toString({columns.source}), '')"


def new_key(columns: Columns, tenants: Tenants) -> str:
    """The row's key under the new tenant; a key whose tenant is not moving stays."""
    key, source = _key_and_source(columns)
    branches = [part for t in tenants.moving for form in _forms(t, key, source, tenants.new) for part in form]
    return f"multiIf({', '.join(branches)}, {key})"


def rewritten_key(columns: Columns, tenants: Tenants) -> str:
    """`new_key`, except that a NULL key stays NULL."""
    return f"if({columns.key} IS NULL, {columns.key}, {new_key(columns, tenants)})"


def key_is_ambiguous(columns: Columns, tenants: Tenants) -> str:
    """A key that starts like a moving tenant but is not followed by the row's source.

    Rewriting it would guess where the tenant ends, so the table is refused instead.
    """
    key, source = _key_and_source(columns)
    recognised = [m for t in (*tenants.moving, tenants.new) for m, _ in _forms(t, key, source, tenants.new)]
    looks_moving = [f"startsWith({key}, {_lit(t + sep)})" for t in tenants.moving for sep in SEPARATORS]
    looks_moving.append(f"startsWith({key}, '[')")
    return f"(({' OR '.join(looks_moving)}) AND NOT ({' OR '.join(recognised)}))"


def new_tenant(column: str, tenants: Tenants) -> str:
    """The row's tenant under the new tenant; an empty one moves too."""
    value = tenant_value(column)
    moving = ", ".join(_lit(t) for t in tenants.moving)
    return f"if({value} IN ({moving}), {_lit(tenants.new)}, {value})"


def moves(columns: Columns, tenants: Tenants) -> str:
    """The row changes: its tenant moves, or its key carries a moving tenant."""
    moving = ", ".join(_lit(t) for t in tenants.moving)
    tenant_moves = f"{tenant_value(columns.tenant)} IN ({moving})"
    if columns.key is None:
        return tenant_moves
    key = f"ifNull({columns.key}, '')"
    return f"({tenant_moves} OR {new_key(columns, tenants)} != {key})"


def foreign_tenant(column: str, tenants: Tenants) -> str:
    """A tenant that is neither moving nor the new one: a second tenant in the install."""
    known = ", ".join(_lit(t) for t in (*tenants.moving, tenants.new))
    return f"{tenant_value(column)} NOT IN ({known})"
