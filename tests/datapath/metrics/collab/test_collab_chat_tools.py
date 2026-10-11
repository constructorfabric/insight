"""Chat metrics add up across Microsoft Teams, Slack and Zulip.

Messages sent add every tool's total. Channel posts come only from tools that
report channel messages. The DM ratio divides direct messages by the totals of the
tools that tell direct from channel messages apart, so a total without that split
never dilutes it. Messages per active day divide by the distinct days with messages
in any tool, so a day spent in two tools, or under two addresses, counts once.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_chat_tools"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


def _period(person: str, metric_key: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": "2026-12-01", "to": "2026-12-31"},
            "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
        },
    }


@pytest.mark.parametrize(
    ("person", "metric_key", "expected"),
    [
        pytest.param(ALICE, "collab.messages_sent", 40, id="totals-add-across-tools-and-addresses"),
        pytest.param(BOB, "collab.messages_sent", 12, id="zulip-total-counts-as-messages"),
        pytest.param(CAROL, "collab.messages_sent", 20, id="slack-total-counts-as-messages"),
        pytest.param(ALICE, "collab.channel_posts", 4, id="teams-channel-posts-include-replies"),
        pytest.param(
            BOB, "collab.channel_posts", None, id="tool-without-channel-split-has-no-channel-posts"
        ),
        pytest.param(
            CAROL, "collab.channel_posts", 5, id="slack-channel-slice-counts-as-channel-posts"
        ),
        pytest.param(
            ALICE, "collab.dm_ratio", 60, id="tool-without-split-does-not-dilute-the-dm-ratio"
        ),
        pytest.param(BOB, "collab.dm_ratio", None, id="only-tools-without-split-leave-no-dm-ratio"),
        pytest.param(
            CAROL, "collab.dm_ratio", 75, id="slack-dm-ratio-uses-the-non-channel-residual"
        ),
        pytest.param(
            ALICE, "collab.msgs_per_active_day", 20, id="a-day-in-two-tools-is-one-active-day"
        ),
        pytest.param(
            BOB, "collab.msgs_per_active_day", 12, id="single-tool-messages-per-active-day"
        ),
    ],
)
def test_chat_metrics_across_tools(
    spec: SpecRun, person: str, metric_key: str, expected: float | None
) -> None:
    r = spec.call(_period(person, metric_key))
    assert r.status == 200, f"should answer 200 for {person} {metric_key}"
    row = r.row(metric_key, "period", entity_id=person)
    if expected is None:
        row.equals(value=None)
        return
    row.check(
        "value",
        lambda v: v is not None and float(v) == approx(expected),
        f"{metric_key} for {person} should be {expected}",
    )


def test_dm_ratio_breakdown_leaves_a_tool_without_split_empty(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.dm_ratio",
                        "views": [{"view": "breakdown", "dimensions": ["tool"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200, "should answer 200 for the DM ratio breakdown"
    by_tool = {
        entry["dimensions"][0]["value"]: entry["value"] for entry in r.breakdown("collab.dm_ratio")
    }
    assert by_tool.get("m365") == approx(60.0), f"Teams share should be 6 of 10, got {by_tool}"
    assert by_tool.get("zulip_proxy") is None, (
        f"Zulip has no split and must carry no share, got {by_tool}"
    )
