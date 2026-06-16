from __future__ import annotations

from collections import deque
from collections.abc import Iterable, Mapping
from datetime import datetime, timedelta, timezone
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpStream


class WindowTooLarge(RuntimeError):
    pass


class UnwindowableWindow(RuntimeError):
    pass


def _to_utc_z(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _parse(timestamp: str) -> datetime:
    parsed = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=timezone.utc)
    return parsed


class TimeWindowedReadMixin:
    SOFT_PAGE_LIMIT = 490

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._window_page_count = 0

    def next_page_token(self, response: Any) -> Mapping[str, Any] | None:
        token: Mapping[str, Any] | None = super().next_page_token(response)  # type: ignore[misc]
        if token is not None:
            self._window_page_count += 1
            if self._window_page_count >= self.SOFT_PAGE_LIMIT:
                raise WindowTooLarge
        return token

    def parse_response(self, response: Any, **kwargs: Any) -> Iterable[Mapping[str, Any]]:
        if response.status_code == 400 and "offset" in (response.text or "").lower():
            raise WindowTooLarge
        yield from super().parse_response(response, **kwargs)  # type: ignore[misc]

    def _windowed_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None,
        stream_slice: Mapping[str, Any] | None,
        stream_state: Mapping[str, Any] | None,
    ) -> Iterable[Any]:
        windows = deque([self._window_initial(stream_slice)])
        while windows:
            window = windows.popleft()
            self._window_page_count = 0
            last_value: str | None = None
            try:
                for record in HttpStream.read_records(
                    self,  # type: ignore[arg-type]
                    sync_mode,
                    cursor_field=cursor_field,
                    stream_slice=self._window_apply(stream_slice, window),
                    stream_state=stream_state,
                ):
                    if isinstance(record, Mapping):
                        value = self._window_value(record)
                        if value:
                            last_value = value
                    yield record
            except WindowTooLarge:
                for sub in reversed(self._window_split(window, last_value)):
                    windows.appendleft(sub)

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


class UpdatedAtWindowing(TimeWindowedReadMixin):
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


class CommittedDateWindowing(TimeWindowedReadMixin):
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
