"""Read Sentry envelope item headers without interpreting error payloads."""

import json


def envelope_has_event(body: bytes) -> bool:
    _, separator, remaining = body.partition(b"\n")
    if not separator:
        return False

    while remaining:
        header, separator, payload = remaining.partition(b"\n")
        if not separator:
            return False
        try:
            item = json.loads(header)
        except (ValueError, UnicodeDecodeError):
            return False
        if not isinstance(item, dict):
            return False
        if item.get("type") == "event":
            return True

        length = item.get("length")
        if length is None:
            _, _, remaining = payload.partition(b"\n")
            continue
        if type(length) is not int or length < 0 or length > len(payload):
            return False

        remaining = payload[length:]
        if remaining and not remaining.startswith(b"\n"):
            return False
        remaining = remaining.removeprefix(b"\n")

    return False
