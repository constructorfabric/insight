"""Manifest invariants for the Bitbucket streams.

Bitbucket's `fields` parameter is an exact projection, not a hint: a response
carries the named leaves and nothing else. A stream that hoists `author
.account_id` while asking only for `author.uuid` therefore reads an absent key
on every record — and because the hoisting expressions carry `or ''` / `or {}`
defaults, the column fills with a plausible empty value rather than failing.
`is_fork`, derived from the presence of `parent`, reads false for a fork.

Neither the schema nor the mock-server suite can catch that: the record shape
is valid and the mocks answer with whatever the fixture chose to include.
"""

from __future__ import annotations

import re

import yaml
from config import BitbucketCloudConfigBuilder  # noqa: F401  (keeps the suite's import shape)

from connector_tests import connector_dir

_CONNECTOR = "git/bitbucket-cloud"
_GETS = re.compile(r"record\s*\.get\('([^']+)'\)|\.get\('([^']+)'\)")
_CHAIN = re.compile(
    r"record\s*\.get\('[^']+'\)(?:\s*or\s*\{\}\s*\)?)?"
    r"(?:\s*\.get\('[^']+'\)(?:\s*or\s*\{\}\s*\)?)?)*"
)


def _streams() -> list[dict]:
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    return manifest["streams"]


def _dereferenced_paths(expression: str) -> set[str]:
    """Every dotted path an interpolation walks off `record`."""
    found = set()
    for chain in _CHAIN.finditer(expression):
        keys = [m.group(1) or m.group(2) for m in _GETS.finditer(chain.group(0))]
        if keys:
            found.add(".".join(keys))
    return found


def _add_fields(stream: dict) -> dict[str, tuple[int, str]]:
    out: dict[str, tuple[int, str]] = {}
    for index, transform in enumerate(stream.get("transformations", [])):
        if transform.get("type") == "AddFields":
            for field in transform["fields"]:
                out.setdefault(field["path"][0], (index, str(field.get("value", ""))))
    return out


def _removed(stream: dict) -> dict[str, int]:
    out: dict[str, int] = {}
    for index, transform in enumerate(stream.get("transformations", [])):
        if transform.get("type") == "RemoveFields":
            for pointer in transform.get("field_pointers", []):
                if len(pointer) == 1:
                    out.setdefault(pointer[0], index)
    return out


def _projection(stream: dict) -> set[str] | None:
    params = stream["retriever"]["requester"].get("request_parameters") or {}
    raw = params.get("fields")
    if not raw:
        return None
    return {
        part.strip().removeprefix("values.")
        for part in raw.split(",")
        if part.strip() not in ("next", "values")
    }


def _is_projected(path: str, projection: set[str]) -> bool:
    """Projected if the path, an ancestor, or any descendant of it was named.

    A descendant counts because naming `parent.full_name` puts a `parent`
    object on the record — which is all `is_fork` asks of it.
    """
    parts = path.split(".")
    if any(".".join(parts[:depth]) in projection for depth in range(1, len(parts) + 1)):
        return True
    return any(named.startswith(path + ".") for named in projection)


def test_every_hoisted_path_is_named_in_the_fields_projection() -> None:
    unrequested: dict[str, list[str]] = {}
    for stream in _streams():
        projection = _projection(stream)
        if projection is None:
            continue
        read: set[str] = set()
        for _, expression in _add_fields(stream).values():
            read |= _dereferenced_paths(expression)
        missing = sorted(p for p in read if not _is_projected(p, projection))
        if missing:
            unrequested[stream["name"]] = missing
    assert not unrequested, (
        "these streams hoist paths the `fields` projection never asks for, so the "
        f"column silently takes its default on every record: {unrequested}"
    )


def test_no_hoisted_field_is_deleted_by_a_later_remove() -> None:
    """A hoisted name that collides with a stripped raw key loses the value."""
    clashes: dict[str, list[str]] = {}
    for stream in _streams():
        added, removed = _add_fields(stream), _removed(stream)
        collided = sorted(
            name
            for name, (added_at, _) in added.items()
            if name in removed and removed[name] > added_at
        )
        if collided:
            clashes[stream["name"]] = collided
    assert not clashes, (
        f"RemoveFields deletes a value AddFields just wrote: {clashes}"
    )


def test_the_call_budget_meters_only_the_vendor() -> None:
    """The proxy is ours and admits work on its own terms; metering it against
    Bitbucket's hourly allowance would throttle traffic that allowance does not
    cover."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    budget = manifest["api_budget"]
    matchers = [m for p in budget["policies"] for m in p["matchers"]]
    assert matchers, "the budget must scope itself with matchers"
    assert all(m.get("url_base") == "https://api.bitbucket.org" for m in matchers), matchers
    # Bitbucket reports the reset as seconds remaining; the CDK reads that header
    # with fromtimestamp(). Naming a header it never sends leaves the value unread.
    assert budget["ratelimit_reset_header"] != "x-ratelimit-reset"
    assert budget["ratelimit_remaining_header"] == "x-ratelimit-remaining"


def test_the_call_budget_limit_survives_a_string_valued_config() -> None:
    """A Secret's values reach the source config as strings, so the budget's
    limit has to be declared and interpolated as one."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    field = manifest["spec"]["connection_specification"]["properties"][
        "bitbucket_api_calls_per_hour"
    ]
    assert field["type"] == "string", "an integer here rejects every value a Secret can carry"
    assert isinstance(field["default"], str)
    rate = manifest["api_budget"]["policies"][0]["rates"][0]
    # The fallback covers the key being absent entirely, which is the default path.
    assert "or 1000" in rate["limit"]
