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


_REPO_LISTING_PATH = "/repositories/{{ stream_partition.workspace }}"


def _repository_listings(streams: list[dict]) -> list[tuple[str, str]]:
    """(owner, fields) for every requester that lists a workspace's repositories.

    Eleven of the twelve are fan-out parents inlined inside a substream's
    partition router — the Builder's strict validator rejects a whole-object
    `$ref` for a substream parent — so they cannot share one definition and
    have to be audited instead.
    """
    found: list[tuple[str, str]] = []

    def walk(node: object, owner: str | None) -> None:
        if isinstance(node, dict):
            name = node.get("name") if isinstance(node.get("name"), str) else owner
            requester = node.get("requester")
            if isinstance(requester, dict) and requester.get("path") == _REPO_LISTING_PATH:
                params = requester.get("request_parameters") or {}
                found.append((name or "?", str(params.get("fields", ""))))
            for value in node.values():
                walk(value, name)
        elif isinstance(node, list):
            for value in node:
                walk(value, owner)

    walk(streams, None)
    return found


def test_every_repository_listing_projects_the_field_the_exclusion_reads() -> None:
    """`bitbucket_exclude_repositories` matches on `slug`, and Bitbucket's
    `fields` is an exact projection. A listing that omits it hands the filter an
    absent key: the Jinja render aborts, the condition evaluates to the raw
    template string, and a truthy string keeps the record — so the exclusion is
    silently ignored for that stream and its repositories are cloned anyway.
    """
    listings = _repository_listings(_streams())
    assert listings, "no repository listing found — the audit is not looking at anything"
    missing = sorted(owner for owner, fields in listings if "values.slug" not in fields)
    assert not missing, (
        "these repository listings do not project values.slug, so the exclusion "
        f"filter cannot see it: {missing}"
    )
