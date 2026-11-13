"""Batch fetcher for HubSpot v4 associations.

The CRM Search endpoint does NOT return associations. To avoid a Silver-layer
round-trip to the HubSpot API, streams enrich records with their associations
before emitting to Bronze. This module batches the v4
``/crm/v4/associations/{from}/{to}/batch/read`` endpoint and flattens the
response into id arrays on the parent record.
"""

import logging
from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.sources.streams.http import HttpClient
from pydantic import ValidationError

from source_hubspot.constants import ASSOCIATIONS_BATCH_SIZE, BASE_URL
from source_hubspot.models import AssociationBatch, AssociationTarget, validation_summary

logger = logging.getLogger("airbyte")


class AssociationFetcher:
    """Fetch and inline associations for a batch of records.

    One instance per parent stream. Accepts a pre-built HttpClient so the
    existing retry + rate-limit policy applies to association traffic too —
    associations share the portal's 10/100 rps burst budget.
    """

    def __init__(self, *, from_object_type: str, to_object_types: list[str], http_client: HttpClient) -> None:
        self._from = from_object_type
        self._to_list = list(to_object_types)
        self._http_client = http_client

    def enrich(self, records: list[MutableMapping[str, Any]]) -> list[MutableMapping[str, Any]]:
        """Inline associations onto every record in ``records``.

        Adds ``associations_{to_object_type} = [id, id, ...]`` columns. Leaves
        the original record mapping intact otherwise. Returns the same list
        (mutated in place + returned for convenience so callers can yield from
        the result without a temporary).
        """
        if not records or not self._to_list:
            return records

        by_id: dict[str, MutableMapping[str, Any]] = {}
        for rec in records:
            # Always seed empty association arrays so Bronze has a stable set
            # of columns regardless of whether a row has a fetchable id.
            for to_type in self._to_list:
                rec.setdefault(f"associations_{to_type}", [])
            rid = rec.get("id")
            if rid:
                by_id[str(rid)] = rec

        if not by_id:
            return records

        ids = list(by_id.keys())
        for to_type in self._to_list:
            for batch in _chunked(ids, ASSOCIATIONS_BATCH_SIZE):
                mapping = self._fetch_batch(to_type, batch)
                for from_id, to_ids in mapping.items():
                    parent = by_id.get(from_id)
                    if parent is not None:
                        parent[f"associations_{to_type}"] = to_ids
        return records

    def _fetch_batch(self, to_type: str, ids: list[str]) -> Mapping[str, list[str]]:
        url = f"{BASE_URL}/crm/v4/associations/{self._from}/{to_type}/batch/read"
        body = {"inputs": [{"id": i} for i in ids]}
        _, resp = self._http_client.send_request(
            "POST", url, headers={"Content-Type": "application/json"}, json=body, request_kwargs={}
        )
        try:
            batch = AssociationBatch.model_validate_json(resp.content)
        except ValidationError as exc:
            logger.warning(
                "Association batch %s -> %s returned an unexpected payload (%s)",
                self._from,
                to_type,
                validation_summary(exc),
            )
            return {}
        return _association_ids(batch)


def _association_ids(batch: AssociationBatch) -> Mapping[str, list[str]]:
    """Turn the v4 batch_read response into ``{from_id: [to_id, ...]}``.

    v4 shape::

        {"status": "COMPLETE",
         "results": [
             {"from": {"id": "123"}, "to": [{"toObjectId": 456}, ...]},
             ...
         ]}

    Entries that fail validation arrive as ``None`` and are skipped.
    """
    out: dict[str, list[str]] = {}
    for item in batch.results:
        if item is None or item.source is None or not item.source.id:
            continue
        out[item.source.id] = _target_ids(item.to)
    return out


def _target_ids(targets: list[AssociationTarget | None]) -> list[str]:
    to_ids: list[str] = []
    for target in targets:
        if target is None:
            continue
        tid = target.toObjectId or target.id
        if tid is not None:
            to_ids.append(tid)
    return to_ids


def _chunked(seq: list[str], size: int) -> Iterable[list[str]]:
    for i in range(0, len(seq), size):
        yield seq[i : i + size]
