from __future__ import annotations

from collections.abc import Mapping
from datetime import datetime, timedelta, timezone
from typing import Any

from source_gitlab.streams.errors import UnwindowableWindow
from source_gitlab.streams.timeutil import parse_iso as _parse
from source_gitlab.streams.timeutil import to_utc_z as _to_utc_z

__all__ = ["CommittedDateWindowing", "UnwindowableWindow", "UpdatedAtWindowing"]


class WindowStrategy:
    def _window_initial(self, stream_slice: Mapping[str, Any] | None) -> dict[str, Any]:
        raise NotImplementedError

    def _window_apply(
        self, stream_slice: Mapping[str, Any] | None, window: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        raise NotImplementedError

    def _window_split(
        self, window: Mapping[str, Any], last_value: str | None
    ) -> list[dict[str, Any]]:
        raise NotImplementedError

    def _window_value(self, record: Mapping[str, Any]) -> str | None:
        return None


class UpdatedAtWindowing(WindowStrategy):
    def _window_initial(self, stream_slice: Mapping[str, Any] | None) -> dict[str, Any]:
        return {
            "updated_after": (stream_slice or {}).get("updated_after"),
            "updated_before": None,
        }

    def _window_apply(
        self, stream_slice: Mapping[str, Any] | None, window: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        applied = dict(stream_slice or {})
        applied["updated_after"] = window.get("updated_after")
        applied["updated_before"] = window.get("updated_before")
        return applied

    def _window_value(self, record: Mapping[str, Any]) -> str | None:
        return record.get("updated_at")

    def _window_split(
        self, window: Mapping[str, Any], last_value: str | None
    ) -> list[dict[str, Any]]:
        start = window.get("updated_after")
        if not last_value:
            raise UnwindowableWindow(
                f"offset cap hit but the window produced no records; window={window}"
            )
        next_after = _parse(last_value) - timedelta(seconds=1)
        if start is not None and next_after <= _parse(start):
            raise UnwindowableWindow(
                f"more than the offset cap of records share one updated_at "
                f"timestamp and cannot be windowed further; window={window}, "
                f"last={last_value}"
            )
        return [
            {
                "updated_after": _to_utc_z(next_after),
                "updated_before": window.get("updated_before"),
            }
        ]


class CommittedDateWindowing(WindowStrategy):
    def _window_initial(self, stream_slice: Mapping[str, Any] | None) -> dict[str, Any]:
        return {"since": (stream_slice or {}).get("since"), "until": None}

    def _window_apply(
        self, stream_slice: Mapping[str, Any] | None, window: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        applied = dict(stream_slice or {})
        applied["since"] = window.get("since")
        applied["until"] = window.get("until")
        return applied

    def _window_split(
        self, window: Mapping[str, Any], last_value: str | None
    ) -> list[dict[str, Any]]:
        epoch = datetime(1970, 1, 1, tzinfo=timezone.utc)
        since = _parse(window["since"]) if window.get("since") else epoch
        until = _parse(window["until"]) if window.get("until") else datetime.now(timezone.utc)
        since_str = _to_utc_z(since)
        mid_str = _to_utc_z(since + (until - since) / 2)
        if mid_str in (since_str, _to_utc_z(until)):
            raise UnwindowableWindow(
                f"more than the offset cap of commits fall within a one-second "
                f"window, or beyond the current time, and cannot be subdivided "
                f"further; window={window}"
            )
        return [
            {"since": since_str, "until": mid_str},
            {"since": mid_str, "until": window.get("until")},
        ]
