"""Reconciling what a generator wrote against what the target can hold.

The generators hardcode their column lists; the schema they write into comes from
the connectors-ddl snapshot plus the migrations that heal it. The two drift, and
every row the seed writes passes through one reconciliation, so what it decides
is the contract: a column the target cannot hold is dropped, and a column the
target has and the generator omits takes the engine default — refused when the
engine keeps one row per sorting key.

That decision is a pure function of the generator's values and the target's
shape, so `plan_insert` is what these call. The last three take a stand-in for
the shell instead: the shape query, and the plan reaching the driver.
"""

from __future__ import annotations

import datetime as _dt

import pytest

from insight_seed.generators import insert

_ROWS: list[tuple[object, ...]] = [("p1", 3), ("p2", 5)]


def _shape(
    columns: dict[str, str],
    *,
    engine: str = "MergeTree",
    sorting_key: str = "",
    key_columns: tuple[str, ...] = (),
) -> insert.TargetShape:
    return insert.TargetShape(
        columns=columns,
        key_columns=key_columns,
        engine=engine,
        sorting_key=sorting_key,
    )


def _collapsing(
    columns: dict[str, str], key: str, key_columns: tuple[str, ...]
) -> insert.TargetShape:
    return _shape(columns, engine="ReplacingMergeTree", sorting_key=key, key_columns=key_columns)


_KEY_SHAPE = _collapsing(
    {"person_id": "String", "commits": "UInt32", "unique_key": "String"},
    "unique_key",
    ("unique_key",),
)


def _plan_over_key_shape() -> insert.InsertPlan:
    return insert.plan_insert(
        "silver", "class_git_commits", ["person_id", "commits"], _ROWS, _KEY_SHAPE
    )


def test_a_column_the_target_lacks_is_dropped_with_its_values() -> None:
    plan = insert.plan_insert(
        "silver",
        "class_git_commits",
        ["person_id", "commits", "tool_label"],
        [("p1", 3, "retired"), ("p2", 5, "retired")],
        _shape({"person_id": "String", "commits": "UInt32"}),
    )

    assert plan.dropped == ("tool_label",)
    assert plan.columns == ("person_id", "commits")
    assert [tuple(row) for row in plan.rows] == _ROWS, "the dropped column keeps its values"


def test_a_target_with_no_writable_column_is_refused() -> None:
    with pytest.raises(RuntimeError, match="class_absent"):
        insert.plan_insert("silver", "class_absent", ["person_id"], _ROWS, _shape({}))


def test_what_the_generator_leaves_to_the_engine_is_named() -> None:
    plan = insert.plan_insert(
        "silver",
        "class_git_commits",
        ["person_id"],
        [("p1",), ("p2",)],
        _shape({"person_id": "String", "is_default_branch": "UInt8", "branch": "String"}),
    )

    assert plan.defaulted == ("is_default_branch", "branch")


def test_a_day_grain_date_widens_to_aware_utc_midnight() -> None:
    plan = insert.plan_insert(
        "silver",
        "class_git_commits",
        ["person_id", "day"],
        [("p1", _dt.date(2026, 1, 5))],
        _shape({"person_id": "String", "day": "DateTime"}),
    )

    assert plan.rows[0][1] == _dt.datetime(2026, 1, 5, tzinfo=_dt.UTC)


@pytest.mark.parametrize(
    ("engine", "collapses"),
    [
        ("ReplacingMergeTree", True),
        ("ReplicatedReplacingMergeTree", True),
        ("SummingMergeTree", True),
        ("MergeTree", False),
    ],
)
def test_an_omitted_key_column_is_refused_where_rows_collapse(engine: str, collapses: bool) -> None:
    shape = _shape(
        {"person_id": "String", "day": "Date", "commits": "UInt32"},
        engine=engine,
        sorting_key="person_id, day",
        key_columns=("person_id", "day"),
    )

    def plan() -> insert.InsertPlan:
        return insert.plan_insert(
            "silver", "class_git_commits", ["person_id", "commits"], _ROWS, shape
        )

    if not collapses:
        assert plan().defaulted == ("day",), f"should not refuse: {engine}"
        return

    with pytest.raises(RuntimeError, match="day") as caught:
        plan()
    assert engine in str(caught.value), "the refusal names the engine that collapses"


def test_a_column_the_key_reads_through_an_expression_is_still_demanded() -> None:
    """`ORDER BY (person_id, toDate(ts))` marks `ts` in `is_in_sorting_key` as
    much as `person_id`, and a defaulted `ts` makes `toDate(ts)` constant just as
    surely."""
    shape = _collapsing(
        {"person_id": "String", "ts": "DateTime"}, "person_id, toDate(ts)", ("person_id", "ts")
    )

    with pytest.raises(RuntimeError, match="ts") as caught:
        insert.plan_insert("silver", "class_git_commits", ["person_id"], [("p1",), ("p2",)], shape)

    assert "toDate(ts)" in str(caught.value), "the refusal quotes the key it is about"


def test_the_key_repair_runs_before_the_guard_it_would_trip() -> None:
    plan = _plan_over_key_shape()

    assert plan.key_synthesised
    assert plan.columns == ("person_id", "commits", "unique_key")
    assert plan.defaulted == ()


def test_rows_that_differ_get_keys_that_differ() -> None:
    assert len({row[2] for row in _plan_over_key_shape().rows}) == 2


def test_the_same_rows_seed_to_the_same_keys() -> None:
    first = [row[2] for row in _plan_over_key_shape().rows]
    second = [row[2] for row in _plan_over_key_shape().rows]

    assert first == second, "a re-seed must converge, not append"


def test_a_fixed_width_key_column_is_filled_to_its_width() -> None:
    plan = insert.plan_insert(
        "silver",
        "class_git_commits",
        ["person_id"],
        [("p1",)],
        _collapsing(
            {"person_id": "String", "unique_key": "FixedString(16)"},
            "unique_key",
            ("unique_key",),
        ),
    )

    assert len(str(plan.rows[0][1])) == 16


def test_a_version_bump_alone_does_not_change_the_key() -> None:
    """ReplacingMergeTree keeps the highest `_version` per key. A key that moved
    with the version would append the newer row beside the old one instead of
    replacing it — the engine's semantics, inverted."""
    shape = _collapsing(
        {"person_id": "String", "_version": "UInt64", "unique_key": "String"},
        "unique_key",
        ("unique_key",),
    )
    keys = [
        insert.plan_insert(
            "silver", "class_git_commits", ["person_id", "_version"], [("p1", version)], shape
        ).rows[0][2]
        for version in (1, 2)
    ]

    assert keys[0] == keys[1]


class _FakeResult:
    def __init__(self, rows: list[tuple[object, ...]]) -> None:
        self.result_rows = rows


class _RecordingClient:
    """Answers the two shape queries and records the insert. Holds no policy."""

    def __init__(self) -> None:
        self.queries: list[str] = []
        self.inserted: list[tuple[str, list[str], list[tuple[object, ...]]]] = []

    def query(self, sql: str, parameters: dict[str, object] | None = None) -> _FakeResult:
        self.queries.append(sql)
        if "system.columns" in sql:
            return _FakeResult([("person_id", "String", 1), ("commits", "UInt32", 0)])
        return _FakeResult([("MergeTree", "person_id")])

    def insert(
        self,
        table: str,
        rows: list[tuple[object, ...]],
        column_names: list[str],
        database: str,
    ) -> None:
        self.inserted.append((f"{database}.{table}", list(column_names), list(rows)))

    def columns_query(self) -> str:
        return next(sql for sql in self.queries if "system.columns" in sql)


def test_the_plan_reaches_the_driver_with_its_table_and_database() -> None:
    client = _RecordingClient()

    written = insert.bulk_insert(
        client,  # type: ignore[arg-type]
        "silver",
        "class_git_commits",
        ["person_id", "commits"],
        _ROWS,
    )

    assert written == 2
    assert client.inserted == [("silver.class_git_commits", ["person_id", "commits"], _ROWS)]


def test_an_empty_row_set_reaches_no_server_at_all() -> None:
    client = _RecordingClient()

    assert insert.bulk_insert(client, "silver", "class_git_commits", ["person_id"], []) == 0  # type: ignore[arg-type]
    assert (client.queries, client.inserted) == ([], [])


def test_the_shape_holds_only_what_a_generator_can_write_and_what_the_key_reads() -> None:
    """ClickHouse computes MATERIALIZED and ALIAS columns and refuses an explicit
    value, so one reaching the plan would be demanded of a generator that cannot
    supply it. Which columns count towards the key is the server's answer too."""
    client = _RecordingClient()

    shape = insert._target_shape(client, "silver", "class_git_commits")  # type: ignore[arg-type]

    assert shape.key_columns == ("person_id",)
    assert "default_kind NOT IN ('MATERIALIZED', 'ALIAS')" in client.columns_query()
    assert "is_in_sorting_key" in client.columns_query()
