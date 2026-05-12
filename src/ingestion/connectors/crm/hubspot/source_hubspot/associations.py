"""Batch fetcher for HubSpot v4 associations.

The CRM Search endpoint does NOT return associations. To avoid a Silver-layer
round-trip to the HubSpot API, streams enrich records with their associations
before emitting to Bronze. This module batches the v4
``/crm/v4/associations/{from}/{to}/batch/read`` endpoint and flattens the
response into id arrays on the parent record.
"""

import logging
from typing import Any, Dict, Iterable, List, Mapping, MutableMapping

import requests

from airbyte_cdk.sources.streams.http import HttpClient

from source_hubspot.constants import ASSOCIATIONS_BATCH_SIZE, BASE_URL
from source_hubspot.rate_limiting import HubspotErrorHandler


logger = logging.getLogger("airbyte")


class AssociationFetcher:
    """Fetch and inline associations for a batch of records.

    One instance per parent stream. Accepts a pre-built HttpClient so the
    existing retry + rate-limit policy applies to association traffic too —
    associations share the portal's 10/100 rps burst budget.
    """

    def __init__(
        self,
        *,
        from_object_type: str,
        to_object_types: List[str],
        http_client: HttpClient,
    ) -> None:
        self._from = from_object_type
        self._to_list = list(to_object_types)
        self._http_client = http_client

    def enrich(
        self, records: List[MutableMapping[str, Any]]
    ) -> List[MutableMapping[str, Any]]:
        """Inline associations onto every record in ``records``.

        Adds ``associations_{to_object_type} = [id, id, ...]`` columns. Leaves
        the original record mapping intact otherwise. Returns the same list
        (mutated in place + returned for convenience so callers can yield from
        the result without a temporary).
        """
        if not records or not self._to_list:
            return records

        by_id: Dict[str, MutableMapping[str, Any]] = {}
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

    def _fetch_batch(
        self, to_type: str, ids: List[str]
    ) -> Mapping[str, List[str]]:
        url = f"{BASE_URL}/crm/v4/associations/{self._from}/{to_type}/batch/read"
        body = {"inputs": [{"id": i} for i in ids]}
        _, resp = self._http_client.send_request(
            "POST",
            url,
            headers={"Content-Type": "application/json"},
            json=body,
            request_kwargs={},
        )
        try:
            payload = resp.json()
        except ValueError:
            logger.warning(
                "Association batch %s -> %s returned non-JSON response",
                self._from,
                to_type,
            )
            return {}
        return _parse_association_response(payload)


def _parse_association_response(payload: Any) -> Mapping[str, List[str]]:
    """Turn the v4 batch_read response into ``{from_id: [to_id, ...]}``.

    v4 shape::

        {"status": "COMPLETE",
         "results": [
             {"from": {"id": "123"}, "to": [{"toObjectId": 456}, ...]},
             ...
         ]}
    """
    out: Dict[str, List[str]] = {}
    if not isinstance(payload, Mapping):
        return out
    for item in payload.get("results") or []:
        if not isinstance(item, Mapping):
            continue
        frm = (item.get("from") or {}).get("id")
        if not frm:
            continue
        to_ids: List[str] = []
        for t in item.get("to") or []:
            if isinstance(t, Mapping):
                tid = t.get("toObjectId") or t.get("id")
                if tid is not None:
                    to_ids.append(str(tid))
        out[str(frm)] = to_ids
    return out


def _chunked(seq: List[str], size: int) -> Iterable[List[str]]:
    for i in range(0, len(seq), size):
        yield seq[i : i + size]
