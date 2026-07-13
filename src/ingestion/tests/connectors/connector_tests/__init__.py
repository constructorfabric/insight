"""Mock-server test harness for Insight nocode (declarative-YAML) connectors.

L1 of the connector test ladder — see
docs/domain/connector/specs/feature-connector-mock-tests/FEATURE.md.

Public API for per-connector suites:

    from connector_tests import (
        ConfigBuilder, get_source, read_stream, assert_records_conform,
        HttpMocker, HttpRequest, HttpResponse, ANY_QUERY_PARAMS,
    )
"""

from airbyte_cdk.test.mock_http import HttpMocker, HttpRequest, HttpResponse
from airbyte_cdk.test.mock_http.request import ANY_QUERY_PARAMS

from connector_tests.builders import ConfigBuilder
from connector_tests.fixtures import load_fixture
from connector_tests.schema_assert import assert_records_conform, stream_schema
from connector_tests.source import connector_dir, get_source, read_stream

__all__ = [
    "ANY_QUERY_PARAMS",
    "ConfigBuilder",
    "HttpMocker",
    "HttpRequest",
    "HttpResponse",
    "assert_records_conform",
    "connector_dir",
    "get_source",
    "load_fixture",
    "read_stream",
    "stream_schema",
]
