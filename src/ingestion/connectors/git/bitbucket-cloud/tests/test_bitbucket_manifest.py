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
    """(owner, fields) for every requester that lists a workspace's repositories."""
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
    # repository_visibility answers "does the token reach anything at all". It
    # generates no partitions and clones nothing, and an excluded repository is
    # an operator's choice rather than an access failure, so it deliberately
    # reads the workspace unfiltered.
    missing = sorted(
        owner
        for owner, fields in listings
        if "values.slug" not in fields and owner != "repository_visibility"
    )
    assert not missing, (
        "these repository listings do not project values.slug, so the exclusion "
        f"filter cannot see it: {missing}"
    )


def _requesters(node, out=None):
    """Every HttpRequester mapping anywhere in the manifest tree."""
    if out is None:
        out = []
    if isinstance(node, dict):
        if node.get("type") == "HttpRequester":
            out.append(node)
        for value in node.values():
            _requesters(value, out)
    elif isinstance(node, list):
        for item in node:
            _requesters(item, out)
    return out


def test_every_requester_declares_an_error_handler() -> None:
    """The CDK default handler makes a per-repository 403 fatal to the whole
    stream, and on the fan-out parents that aborts partition generation: every
    repository later in the updated_on-ordered walk is silently skipped while
    the sync reports success. A handler on every requester is the invariant;
    which action it takes per status is the stream's own decision."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    bare = [
        requester.get("path", "<no path>")
        for requester in _requesters(manifest["streams"])
        if "error_handler" not in requester
    ]
    assert not bare, f"requesters relying on the CDK default error handler: {bare}"


_LISTING_PATH_MARKERS = ("/repositories/{{ stream_partition.workspace }}", "/pullrequests?state=")


def _vendor_retrievers(node: object, out: list[dict] | None = None) -> list[dict]:
    """Every SimpleRetriever whose requester targets Bitbucket itself."""
    if out is None:
        out = []
    if isinstance(node, dict):
        requester = node.get("requester", {})
        if node.get("type") == "SimpleRetriever" and "bitbucket_api_base_url" in str(requester.get("url_base", "")):
            out.append(node)
        for value in node.values():
            _vendor_retrievers(value, out)
    elif isinstance(node, list):
        for item in node:
            _vendor_retrievers(item, out)
    return out


def test_every_fan_out_listing_walks_by_created_on_instead_of_page_number() -> None:
    """The vendor pages by number over an order the walk itself can move; the
    repository and pull-request listings fan out into children over hours, so
    they order by the immutable created_on, bound each page below by the
    previous page's last value read off the raw response, and never send a page
    number. Per-request children are fetched in one go and keep page links."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    keyset: list[str] = []
    paged: list[str] = []
    for retriever in _vendor_retrievers(manifest["streams"]):
        requester = retriever["requester"]
        paginator = retriever.get("paginator")
        if paginator is None:
            continue
        path = requester.get("path", "")
        params = requester.get("request_parameters") or {}
        if any(marker in path for marker in _LISTING_PATH_MARKERS):
            keyset.append(path)
            key = "id" if "/pullrequests?state=" in path else "created_on"
            assert "page_token_option" not in paginator, f"{path}: a page number must never reach the vendor"
            strategy = paginator["pagination_strategy"]
            assert f"response['values'][-1]['{key}']" in strategy["cursor_value"], f"{path}: {strategy}"
            assert params.get("sort") == key, f"{path}: {params.get('sort')}"
            assert f"{key} > " in params["q"] and "next_page_token" in params["q"], f"{path}: {params['q']}"
            assert f"values.{key}" in params["fields"].split(","), f"{path}: {params['fields']}"
        else:
            paged.append(path)
            assert paginator.get("page_token_option") == {"type": "RequestPath"}, f"{path}: {paginator}"
    # repositories and pull_requests, each read by reference from every stream
    # that fans out over it.
    assert len(keyset) == 2, keyset
    assert len(paged) == 7, paged


def _parent_configs(node, out=None):
    """Every ParentStreamConfig mapping anywhere in the manifest tree."""
    if out is None:
        out = []
    if isinstance(node, dict):
        if node.get("type") == "ParentStreamConfig":
            out.append(node)
        for value in node.values():
            _parent_configs(value, out)
    elif isinstance(node, list):
        for item in node:
            _parent_configs(item, out)
    return out


def _owned_parent_configs(manifest: dict) -> list[tuple[dict, dict]]:
    """(owning top-level stream, ParentStreamConfig) for every parent config in the tree."""
    return [(stream, config) for stream in manifest["streams"] for config in _parent_configs(stream)]


def test_every_repository_walk_lists_repositories_from_the_shared_cursor_bounded_parent() -> None:
    """One repository listing serves every stream that fans out over
    repositories: each takes `repositories` by reference, so the CDK's per-name
    response cache turns their reads into one. The listing is bounded by the
    cursor the child persists, one lookback window back, so a sync visits only
    repositories pushed to since the last one. The vendor moves a repository's
    updated_on on commit activity only: exact for commits, file changes and
    commit authors; for pull requests, pipelines and deployments an event
    without a push on a repository nobody pushed to waits for its next push,
    the accepted cost of not listing every repository every sync. A full-refresh
    child persists no parent state, so its copy opens at the start date and
    every repository is re-read for heads each sync."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    repositories = manifest["streams"][0]
    assert repositories["name"] == "repositories"
    bound = repositories["retriever"]["requester"]["request_parameters"]["q"]
    assert "stream_interval.start_time" in bound, bound
    assert repositories["incremental_sync"]["cursor_field"] == "updated_on"
    assert repositories["incremental_sync"].get("lookback_window") == "P1D", "a push lands after its commit"

    repository_walks = set()
    for owner, config in _owned_parent_configs(manifest):
        parent = _referenced_stream(manifest, config["stream"])
        assert "$ref" in config["stream"] or parent["name"] == "repository_authors", (
            f"{owner['name']}: an inlined repository listing cannot share the cached read: {parent['name']}"
        )
        if parent["name"] != "repositories":
            continue
        repository_walks.add(owner["name"])
        if "incremental_sync" in owner:
            assert config.get("incremental_dependency") is True, f"{owner['name']}: state must persist"
        else:
            assert "incremental_dependency" not in config, f"{owner['name']}: full refresh persists no parent state"
    assert repository_walks == {
        "commits",
        "file_changes",
        "branches",
        "pull_requests",
        "pipelines",
        "deployments",
        "commit_authors",
    }


_PROXY_RESET_ACTIONS = {
    "/v1/commits": "SPLIT_USING_CURSOR",
    "/v1/file-changes": "SPLIT_USING_CURSOR",
    "/v1/branches": "RESET",
    "/v1/authors": "RESET",
}


def _proxy_retrievers(node, out=None):
    """Every SimpleRetriever whose requester targets the git proxy."""
    if out is None:
        out = []
    if isinstance(node, dict):
        requester = node.get("requester", {})
        if node.get("type") == "SimpleRetriever" and "git_proxy_url" in str(requester.get("url_base", "")):
            out.append(node)
        for value in node.values():
            _proxy_retrievers(value, out)
    elif isinstance(node, list):
        for item in node:
            _proxy_retrievers(item, out)
    return out


def test_a_superseded_proxy_snapshot_restarts_the_walk_instead_of_failing_it() -> None:
    """A 409 means the page token points into a snapshot the proxy no longer
    holds. Failing the partition freezes its cursor until the next run; a
    pagination reset restarts the walk, and a walk the proxy orders by the
    cursor restarts from the last value already seen. Commits and file
    changes come out ordered by committed_date; branches and authors carry
    no such order, so their restart is from the first page."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    retrievers = _proxy_retrievers(manifest["streams"])
    assert {r["requester"]["path"] for r in retrievers} == set(_PROXY_RESET_ACTIONS)
    for retriever in retrievers:
        path = retriever["requester"]["path"]
        filters = retriever["requester"]["error_handler"]["response_filters"]
        on_409 = [f["action"] for f in filters if 409 in f.get("http_codes", [])]
        assert on_409 == ["RESET_PAGINATION"], f"{path}: a 409 must reset pagination, got {on_409}"
        reset = retriever.get("pagination_reset")
        assert reset == {"type": "PaginationReset", "action": _PROXY_RESET_ACTIONS[path]}, f"{path}: {reset}"


def test_every_proxy_request_carries_the_repository_size_hint() -> None:
    """The proxy reserves cache headroom from the hint instead of its per-repository
    cap; a proxy requester without it, or a proxy parent that does not pass the size
    along, silently falls back to the cap."""
    retrievers = _proxy_retrievers(_streams())
    assert retrievers
    for retriever in retrievers:
        hint = (retriever["requester"].get("request_headers") or {}).get("X-Repo-Size-Hint", "")
        assert "extra_fields.get('size')" in hint, retriever["requester"]["path"]
        for parent in retriever["partition_router"]["parent_stream_configs"]:
            if parent.get("partition_field") == "repo_clone_url":
                assert ["size"] in (parent.get("extra_fields") or []), retriever["requester"]["path"]


def _referenced_stream(manifest: dict, node: dict) -> dict:
    """Follow a `$ref: "#/streams/N"` to the stream it names; an inlined stream is returned as is."""
    if "$ref" in node:
        return manifest["streams"][int(node["$ref"].rsplit("/", 1)[1])]
    return node


def _direct_parent(manifest: dict, stream: dict) -> tuple[dict, dict]:
    configs = stream["retriever"]["partition_router"]["parent_stream_configs"]
    assert len(configs) == 1, stream["name"]
    return configs[0], _referenced_stream(manifest, configs[0]["stream"])


def test_the_pull_request_children_read_the_pull_requests_stream_itself() -> None:
    """Each child hangs off the top-level pull_requests stream by reference, and
    pull_requests off the top-level repositories stream, so one definition serves
    every listing and the CDK's per-name response cache turns the five reads of
    each into one. The cache flag is stated on both: a `$ref` copy inherits it,
    and without it every copy would read the vendor."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    by_name = {s["name"]: s for s in manifest["streams"]}
    children = [s for s in manifest["streams"] if s["name"].startswith("pull_request_")]
    assert len(children) == 4, [s["name"] for s in children]

    for child in children:
        config, parent = _direct_parent(manifest, child)
        assert config["stream"] == {"$ref": "#/streams/5"}, child["name"]
        assert parent is by_name["pull_requests"]
        assert config.get("incremental_dependency") is True, child["name"]

    prs_config, prs_parent = _direct_parent(manifest, by_name["pull_requests"])
    assert prs_config["stream"] == {"$ref": "#/streams/0"}
    assert prs_parent is by_name["repositories"]

    for name in ("repositories", "pull_requests"):
        assert by_name[name]["retriever"]["requester"].get("use_cache") is True, name

    bound = by_name["pull_requests"]["retriever"]["requester"]["request_parameters"]["q"]
    assert "updated_on <" not in bound and "now_utc" not in bound and "end_time" not in bound, (
        f"the listing request must read no clock, or the five streams build five URLs: {bound}"
    )


def test_the_listing_chain_runs_one_level_at_a_time() -> None:
    """A shared listing pays off only if the first read is cached before the next
    stream asks; streams starting together race the vendor. One blocking group per
    level — repositories, pull_requests, the four children — makes a dependant
    wait for its parent's group and the children take turns."""
    manifest = yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())
    groups = manifest["stream_groups"]
    assert all(group["action"]["type"] == "BlockSimultaneousSyncsAction" for group in groups.values())
    members = {
        name: {_referenced_stream(manifest, ref)["name"] for ref in group["streams"]} for name, group in groups.items()
    }
    assert members == {
        "repository_listing": {"repositories"},
        "pull_request_listing": {"pull_requests"},
        "pull_request_children": {
            "pull_request_comments",
            "pull_request_commits",
            "pull_request_diffstat",
            "pull_request_activity",
        },
    }
