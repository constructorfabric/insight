from __future__ import annotations

import os
from collections.abc import Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

import pytest
from insight_stand import ApiClient, ApiResponse, JsonValue, scratch_name, v3_path
from insight_stand.stand import candidate_env_files, parse_env_file

from .. import scratch
from ..schemas.v3_untyped import CreatedId, Dataset, Named, RunResult

_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[4]
_INGEST_TOKEN_KEY: Final[str] = "INSIGHT_V3_INGEST_TOKEN"
_COMPOSE_FALLBACK_INGEST_TOKEN: Final[str] = "insight-v3-core-local-dev-ingest-token"
_INGEST_TOKEN_HEADER: Final[str] = "x-insight-token"


@dataclass(frozen=True)
class Record:
    day: str
    author: str
    lines: int

    def raw(self) -> dict[str, JsonValue]:
        return {"day": self.day, "author": self.author, "lines": self.lines}

    def key(self) -> tuple[str, str, int]:
        return self.day[:10], self.author, self.lines


RECORDS: Final[tuple[Record, ...]] = (
    Record("2026-10-01T09:00:00Z", "ana", 4),
    Record("2026-10-02T09:00:00Z", "ben", 4),
    Record("2026-10-03T09:00:00Z", "ana", 4),
    Record("2026-10-04T09:00:00Z", "ben", 7),
    Record("2026-10-05T09:00:00Z", "ana", 1),
)

DECLARATION_FIELDS: Final[list[JsonValue]] = [
    {"name": "day", "path": "day", "type": "datetime", "default_clock": True},
    {"name": "author", "path": "author", "type": "string"},
    {"name": "lines", "path": "lines", "type": "int"},
]


@dataclass(frozen=True)
class Ingest:
    client: ApiClient
    token: str

    def post(self, body: JsonValue, token: str | None) -> ApiResponse:
        headers = None if token is None else {_INGEST_TOKEN_HEADER: token}
        return self.client.post(v3_path("/v1/raw-data"), json_body=body, headers=headers)

    def feed(self, dataset: str, records: Sequence[Record]) -> None:
        for record in records:
            _expect(self.post({"dataset": dataset, "raw_data": record.raw()}, self.token), 204)


def _expect(response: ApiResponse, status: int) -> ApiResponse:
    assert response.status_code == status, (
        f"{response.url}: expected {status}, got {response.status_code} {response.text[:300]}"
    )
    return response


def _declare(client: ApiClient, removals: list[str]) -> Dataset:
    name = scratch_name("fed")
    declaration: JsonValue = {"title": name, "fields": DECLARATION_FIELDS}

    response = _expect(client.put(v3_path(f"/v1/datasets/{name}"), json_body=declaration), 200)
    scratch.track(v3_path("/v1/datasets"), "name", name, params={"q": name, "limit": 200})
    removals.append(v3_path(f"/v1/datasets/{name}"))

    return response.parse(Dataset)


def _create(
    client: ApiClient, removals: list[str], segment: str, body: dict[str, JsonValue]
) -> Named:
    name = scratch_name(segment)

    response = _expect(
        client.post(v3_path(f"/v1/{segment}"), json_body={"name": name, "body": body}), 201
    )
    created = response.parse(CreatedId).id
    scratch.track(v3_path(f"/v1/{segment}"), "id", created, params={"q": name, "limit": 200})
    removals.append(v3_path(f"/v1/{segment}/{created}"))

    return Named(id=created, name=name)


def _run(client: ApiClient, path: str) -> RunResult:
    return _expect(client.post(path), 200).parse(RunResult)


@pytest.fixture
def ingest(v3_admin: ApiClient) -> Ingest:
    env_file = next(
        (path for path in candidate_env_files(_REPO_ROOT, os.environ) if path.is_file()), None
    )
    settings = parse_env_file(env_file) if env_file is not None else {}

    token = (os.environ.get(_INGEST_TOKEN_KEY) or settings.get(_INGEST_TOKEN_KEY) or "").strip()
    return Ingest(v3_admin, token or _COMPOSE_FALLBACK_INGEST_TOKEN)


@pytest.fixture
def removals(v3_admin: ApiClient) -> Iterator[list[str]]:
    paths: list[str] = []
    yield paths

    for path in reversed(paths):
        v3_admin.delete(path)


@pytest.fixture
def fed_dataset(v3_admin: ApiClient, ingest: Ingest, removals: list[str]) -> Dataset:
    dataset = _declare(v3_admin, removals)
    ingest.feed(dataset.name, RECORDS)
    return dataset
