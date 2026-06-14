"""Unit tests for the UI render contract (runnable with no infra: `pytest` or
`python test_render_contract.py`).

Each case is derived from intent, not from the current frontend output — several
of these are exactly the discrepancies observed on the live dashboard
(constructorfabric/insight#1336, #1337).
"""
from __future__ import annotations

from render_contract import COMING_SOON, NO_DATA, display_value, round_half_up


def test_round_half_up_examples() -> None:
    # the rounding-ownership example: API 4.4 must show 4; 4.6 must show 5.
    assert round_half_up(4.4) == 4
    assert round_half_up(4.6) == 5
    assert round_half_up(0.5) == 1
    assert round_half_up(2.5) == 3  # half-up, NOT banker's (which would give 2)


def test_focus_time_rounds_988_to_99() -> None:
    # live: API focus_time_pct = 98.8 → tile headline "99%". Correct.
    assert display_value(98.8, fmt="percent", unit="%") == "99%"


def test_not_ingested_shows_coming_soon_not_zero() -> None:
    # #1337: prs_merged is "not ingested yet — show ComingSoon", API returns 0.0,
    # the tile must NOT render "0".
    assert display_value(0.0, fmt="integer", unit="count", ingested=False) == COMING_SOON


def test_null_ratio_is_no_data_not_zero_percent() -> None:
    # Regression guard: a null ratio must be no-data, never "0%". The FE already
    # satisfies this (transformIcKpis maps null→no value); this pins it so it can't
    # regress. (Note: the live "AI acceptance 0%" was a real 0.0, not null — that is
    # a data effect of the broken git clean_loc lineage, not a render bug.)
    assert display_value(None, fmt="percent", unit="%") == NO_DATA
    assert display_value(None, fmt="percent", unit="%") != "0%"


def test_unit_has_separating_space() -> None:
    # #1291: tiles render "0tasks" / "0h"; the contract requires a space.
    assert display_value(0.0, unit="tasks") == "0 tasks"
    assert display_value(0.0, unit="h") == "0 h"
    assert " " in display_value(3.0, unit="tasks")


def test_real_zero_still_shows_zero() -> None:
    # a genuinely-measured 0 (ingested, not null) is a real value and shows "0".
    assert display_value(0.0, fmt="integer", unit="count", ingested=True) == "0"


if __name__ == "__main__":
    # allow running without pytest
    import traceback

    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    failed = 0
    for fn in fns:
        try:
            fn()
            print(f"  PASS {fn.__name__}")
        except AssertionError:
            failed += 1
            print(f"  FAIL {fn.__name__}")
            traceback.print_exc()
    print(f"\n{len(fns) - failed}/{len(fns)} passed")
    raise SystemExit(1 if failed else 0)
