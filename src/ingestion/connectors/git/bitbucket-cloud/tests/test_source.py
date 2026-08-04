import logging
from unittest.mock import patch

import pytest
from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.source import SourceBitbucketCloud

LOGGER = logging.getLogger("test")
CONFIG = {"bitbucket_token": "tok", "bitbucket_workspaces": ["ws"], "insight_tenant_id": "T", "insight_source_id": "S"}


def test_empty_workspaces_fail_fast():
    ok, reason = SourceBitbucketCloud().check_connection(LOGGER, {**CONFIG, "bitbucket_workspaces": []})
    assert ok is False
    assert "bitbucket_workspaces is empty" in reason


@patch("source_bitbucket_cloud.source.BitbucketClient")
def test_check_connection_probes_every_workspace(client_type):
    client = client_type.return_value
    ok, reason = SourceBitbucketCloud().check_connection(LOGGER, {**CONFIG, "bitbucket_workspaces": ["one", "two"]})
    assert ok is True
    assert reason is None
    assert [call.args[1] for call in client.request.call_args_list] == ["repositories/one", "repositories/two"]


@pytest.mark.parametrize(
    "code,fragment",
    [
        (401, "Authentication failed"),
        (403, "lacks permission"),
        (404, "not found or not accessible"),
        (500, "Bitbucket API returned 500"),
    ],
)
@patch("source_bitbucket_cloud.source.BitbucketClient")
def test_check_connection_maps_api_errors(client_type, code, fragment):
    client_type.return_value.request.side_effect = BitbucketApiError(code, "url", "body")
    ok, reason = SourceBitbucketCloud().check_connection(LOGGER, CONFIG)
    assert ok is False
    assert fragment in reason


@patch("source_bitbucket_cloud.source.BitbucketClient")
def test_check_connection_reports_transport_errors(client_type):
    client_type.return_value.request.side_effect = RuntimeError("offline")
    ok, reason = SourceBitbucketCloud().check_connection(LOGGER, CONFIG)
    assert ok is False
    assert reason == "Bitbucket API request failed: offline"


TRANSFORMED_STREAMS = [
    "repositories",
    "branches",
    "pull_requests",
    "pull_request_commits",
    "pull_request_comments",
    "pull_request_activity",
    "pull_request_diffstat",
    "commits",
    "file_changes",
    "commit_branch_reachability",
]
# Their first read of a repository pages whatever history it holds; everything
# before them is bounded by a watermark.
UNBOUNDED_STREAMS = ["commits", "file_changes", "commit_branch_reachability"]


def test_streams_are_independent_and_share_client_and_catalog():
    streams = SourceBitbucketCloud().streams(CONFIG)
    assert [stream.name for stream in streams] == TRANSFORMED_STREAMS
    assert len({id(stream._client) for stream in streams}) == 1
    assert len({id(stream._catalog) for stream in streams}) == 1
    assert not any(hasattr(stream, "parent") for stream in streams)


def test_streams_the_transform_layer_reads_come_first():
    """A sync that runs out of time must still have produced what dbt builds
    on; the trailing streams have no model reading them."""
    names = [stream.name for stream in SourceBitbucketCloud().streams(CONFIG)]

    assert names[: len(TRANSFORMED_STREAMS)] == TRANSFORMED_STREAMS


def test_watermark_bounded_streams_run_before_history_sized_ones():
    """A stream that can page a whole history must not be able to starve the
    streams that cannot."""
    names = [stream.name for stream in SourceBitbucketCloud().streams(CONFIG)]
    bounded = [name for name in TRANSFORMED_STREAMS if name not in UNBOUNDED_STREAMS]

    assert max(names.index(name) for name in bounded) < min(names.index(name) for name in UNBOUNDED_STREAMS)


def test_tenant_identity_and_spec():
    streams = SourceBitbucketCloud().streams(CONFIG)
    assert all(stream._tenant_id == "T" and stream._source_id == "S" for stream in streams)
    spec = SourceBitbucketCloud().spec(LOGGER)
    properties = spec.connectionSpecification["properties"]
    assert {"bitbucket_token", "bitbucket_workspaces", "bitbucket_api_base_url"} <= set(properties)


@patch("source_bitbucket_cloud.source.BitbucketClient")
def test_api_base_url_is_used_for_check_and_streams(client_type):
    config = {**CONFIG, "bitbucket_api_base_url": "http://emulator:8080/2.0/"}
    ok, reason = SourceBitbucketCloud().check_connection(LOGGER, config)
    assert ok is True
    assert reason is None
    SourceBitbucketCloud().streams(config)
    assert client_type.call_args_list[0].args == ("tok", "", "http://emulator:8080/2.0/")
    assert client_type.call_args_list[1].args == ("tok", "", "http://emulator:8080/2.0/")
