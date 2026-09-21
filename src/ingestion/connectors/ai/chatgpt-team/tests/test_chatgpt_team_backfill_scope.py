"""`start_date` backfills the daily analytics streams and nothing else.

Two windows live in this connector and they are not the same knob. The per-day
analytics streams walk a DatetimeBasedCursor from `start_date`, so moving it back
loads history. The subscription streams ask for a fixed rolling 30-day window
ending today; they do not read `start_date` and cannot be made to load a past
billing period by setting it.

The separation is worth pinning because it is invisible from the outside: both
requests carry a parameter literally named `start_date`, and wiring the config
value into the billing one would silently redefine what `amount` measures —
a rolling figure becoming an all-time one — under an unchanged `snapshot_date`,
with nothing downstream recording which it was.
"""

from __future__ import annotations

from config import ORG_ID, PROXY_URL, ChatGptTeamConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, get_source, read_stream
from freezegun import freeze_time

_CONNECTOR = "ai/chatgpt-team"
_USAGE_URL = f"{PROXY_URL}/api/subscriptions/{ORG_ID}/usage"

_NOW = "2026-08-19T11:30:00Z"
_FAR_BACK = "2026-01-05"

# What the subscription streams must ask for whatever `start_date` says: today
# and the 30 days before it.
_FIXED_BILLING_WINDOW = {"start_date": "2026-07-20", "end_date": "2026-08-19"}

_ANALYTICS_STREAMS = [
    "chatgpt_team_chat_activity",
    "chatgpt_team_codex_user_daily",
    "chatgpt_team_codex_user_daily_org",
]
_SUBSCRIPTION_STREAMS = ["chatgpt_team_subscription_usage", "chatgpt_team_subscription_balance"]


def _first_window(stream_name: str, config: dict[str, str]) -> tuple[str, str]:
    with freeze_time(_NOW):
        source = get_source(_CONNECTOR, config)
        stream = next(s for s in source.streams(config) if s.name == stream_name)
        first = next(iter(stream.generate_partitions())).to_slice()
    return first["start_time"], first["end_time"]


def test_a_backfill_date_moves_the_analytics_streams() -> None:
    """The half that does honour it — otherwise the test below proves nothing."""
    config = ChatGptTeamConfigBuilder().with_start_date(_FAR_BACK).build()

    for stream_name in _ANALYTICS_STREAMS:
        start, _ = _first_window(stream_name, config)
        assert start == _FAR_BACK, f"{stream_name} must backfill from the configured date, got {start}"


def test_a_backfill_date_does_not_move_the_subscription_window(http_mocker: HttpMocker) -> None:
    """The mock answers only the fixed rolling window. Were `start_date` wired
    into the billing request, the request would carry 2026-01-05 instead, match
    no matcher, and this would fail rather than quietly load an all-time figure
    under a daily snapshot key."""
    config = ChatGptTeamConfigBuilder().with_start_date(_FAR_BACK).build()

    # One matcher for both streams: they are the same request, and the mocker
    # fails a matcher that goes unused.
    http_mocker.get(
        HttpRequest(_USAGE_URL, query_params=_FIXED_BILLING_WINDOW),
        HttpResponse(body='{"usage_detail": [], "current_balance": 0}', status_code=200),
    )

    for stream_name in _SUBSCRIPTION_STREAMS:
        with freeze_time(_NOW):
            output = read_stream(_CONNECTOR, stream_name, config)

        assert not output.errors, f"{stream_name} asked for a window other than the fixed rolling one: {output.errors}"
