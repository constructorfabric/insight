"""Build an in-process CDK source from a nocode connector package and run
protocol reads against it.

Implements `cpt-insightspec-algo-cn-mock-source-build` and
`cpt-insightspec-algo-cn-mock-read` from the feature spec.
"""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path
from typing import Any

import yaml
from jsonschema import Draft7Validator

from airbyte_cdk.models import AirbyteStateMessage, SyncMode
from airbyte_cdk.sources.declarative.yaml_declarative_source import YamlDeclarativeSource
from airbyte_cdk.test.catalog_builder import CatalogBuilder
from airbyte_cdk.test.entrypoint_wrapper import EntrypointOutput, read

# src/ingestion/tests/connectors/connector_tests/source.py -> src/ingestion/connectors
CONNECTORS_DIR = Path(__file__).resolve().parents[3] / "connectors"


def connector_dir(connector_path: str) -> Path:
    """Resolve '<category>/<name>' to the connector package directory."""
    pkg = CONNECTORS_DIR / connector_path
    if not (pkg / "connector.yaml").is_file():
        raise FileNotFoundError(
            f"no connector.yaml under {pkg} — expected '<category>/<name>' "
            f"relative to {CONNECTORS_DIR}"
        )
    return pkg


@lru_cache(maxsize=None)
def load_manifest(connector_path: str) -> dict[str, Any]:
    # Cached for the test-process lifetime: spec validation and schema asserts
    # re-read the same manifest many times per suite. YamlDeclarativeSource
    # still reads the file itself — deliberately, so the source consumes the
    # same bytes Airbyte would.
    with open(connector_dir(connector_path) / "connector.yaml") as f:
        return yaml.safe_load(f)


def _validate_config_against_spec(manifest: dict[str, Any], config: dict[str, Any]) -> None:
    spec_schema = manifest["spec"]["connection_specification"]
    errors = sorted(Draft7Validator(spec_schema).iter_errors(config), key=str)
    if errors:
        details = "; ".join(e.message for e in errors)
        raise ValueError(f"config does not satisfy the manifest spec: {details}")


def get_source(
    connector_path: str,
    config: dict[str, Any],
    state: list[AirbyteStateMessage] | None = None,
) -> YamlDeclarativeSource:
    """Instantiate the connector's declarative source in-process.

    Loads connector.yaml verbatim (no $ref preprocessing — the same bytes
    Airbyte receives) through the same CDK entry point as the
    source-declarative-manifest image, after validating `config` against the
    manifest spec.
    """
    pkg = connector_dir(connector_path)
    _validate_config_against_spec(load_manifest(connector_path), config)
    return YamlDeclarativeSource(
        path_to_yaml=str(pkg / "connector.yaml"),
        catalog=CatalogBuilder().build(),
        config=config,
        state=state or [],
    )


def read_stream(
    connector_path: str,
    stream: str,
    config: dict[str, Any],
    state: list[AirbyteStateMessage] | None = None,
    sync_mode: SyncMode = SyncMode.full_refresh,
    expecting_exception: bool = False,
) -> EntrypointOutput:
    """Run a full protocol read of one stream and return the typed output
    (`output.records`, `output.state_messages`, `output.logs`, `output.errors`).
    """
    source = get_source(connector_path, config, state)
    catalog = CatalogBuilder().with_stream(stream, sync_mode).build()
    return read(source, config=config, catalog=catalog, state=state,
                expecting_exception=expecting_exception)
