from __future__ import annotations

from copy import deepcopy
from functools import lru_cache
from pathlib import Path
from typing import Any

import yaml
from airbyte_cdk.sources.declarative.concurrent_declarative_source import _get_declarative_component_schema


@lru_cache(maxsize=32)
def _parse_manifest(contents: str) -> dict[str, Any]:
    return yaml.safe_load(contents)


def read_manifest(source: object, path: str) -> dict[str, Any]:
    return deepcopy(_parse_manifest(Path(path).read_text()))


@lru_cache(maxsize=1)
def _component_schema() -> dict[str, Any]:
    return _get_declarative_component_schema()


def component_schema() -> dict[str, Any]:
    return deepcopy(_component_schema())
