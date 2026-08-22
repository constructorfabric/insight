"""Getting the manifest out of a pod that is about to stop existing.

A cluster Job's filesystem dies with it, so the document is also printed to
stdout — as one line while it fits, and as ordered gzip chunks once it does
not. `manifest-from-log.sh` and `decode_manifest_sentinel` read either form.

Typed against `Mapping[str, Any]` rather than the `Manifest` model: this is a
transport, and `manifest.py` imports nothing from here.
"""

from __future__ import annotations

import base64
import gzip
import json
from collections.abc import Iterable, Mapping
from typing import Any

#: The line CI greps for: `grep -m1 '^SEED_MANIFEST_JSON: ' | cut -d' ' -f2-`.
#: A single physical line, so the pod-log transport needs no parser.
SENTINEL_PREFIX = "SEED_MANIFEST_JSON: "

#: `SEED_MANIFEST_GZ: <i>/<n> <base64(gzip(compact json))>`, 1-based, in order.
GZ_SENTINEL_PREFIX = "SEED_MANIFEST_GZ: "

#: CRI reassembles a container's log line up to this many bytes; longer and
#: `grep -m1` on the far side would capture a fragment, not the document.
_SENTINEL_MAX_BYTES = 16 * 1024

#: Payload per chunk, inside that bound once prefix and counter are added.
_GZ_CHUNK_BYTES = 12 * 1024


def emit_manifest_sentinel(doc: Mapping[str, Any]) -> None:
    """Print the manifest to stdout as sentinel lines a log reader can recover.

    One plain `SEED_MANIFEST_JSON:` line while the document fits the CRI
    line-reassembly bound — which the committed roster does, at ~9 KB — and
    gzipped base64 chunks when it does not.

    Refusing an oversized manifest was the alternative, and it is the wrong one:
    this runs at the END of a seed, after every database write, in a Job with
    `backoffLimit: 0`. A 250-person stand would have been seeded correctly and
    then reported as a failure, with nothing to retry and nothing to fix. The
    personas array is not trimmed either — the stand suite resolves its fixtures
    out of it, so a shortened manifest is a broken one.
    """
    compact = json.dumps(doc, separators=(",", ":"), sort_keys=True, ensure_ascii=False)
    line = SENTINEL_PREFIX + compact
    if len(line.encode("utf-8")) <= _SENTINEL_MAX_BYTES:
        print(line)
        return

    # INVARIANT: mtime=0 keeps the sentinel a function of the document only.
    blob = base64.b64encode(gzip.compress(compact.encode("utf-8"), mtime=0)).decode("ascii")
    chunks = [blob[i : i + _GZ_CHUNK_BYTES] for i in range(0, len(blob), _GZ_CHUNK_BYTES)]
    for index, chunk in enumerate(chunks, start=1):
        print(f"{GZ_SENTINEL_PREFIX}{index}/{len(chunks)} {chunk}")


def _as_document(decoded: Any) -> dict[str, Any]:
    """SAFETY: `json.loads` returns whatever the text was — a bare annotation
    claiming `dict` is an assertion, not a check, and a non-object reaches the
    caller typed as a manifest."""
    if not isinstance(decoded, dict):
        raise ValueError(f"manifest sentinel is {type(decoded).__name__}, not a JSON object")
    return decoded


def decode_manifest_sentinel(lines: Iterable[str]) -> dict[str, Any]:
    """Recover the manifest from log lines carrying either sentinel form.

    The Python half of `manifest-from-log.sh`, kept beside the emitter so the
    two halves of one format cannot drift. Chunks may arrive in any order and
    interleaved with other output, and an identical line read twice (a re-read
    log) is tolerated. Everything else is an error rather than a best-effort
    splice — a missing chunk, a conflicting total or a differing duplicate all
    mean the input mixes two emissions, and a spliced document is not a
    manifest.
    """
    chunks: dict[int, str] = {}
    total: int | None = None
    for raw in lines:
        line = raw.rstrip("\n")
        if line.startswith(SENTINEL_PREFIX):
            return _as_document(json.loads(line[len(SENTINEL_PREFIX) :]))
        if not line.startswith(GZ_SENTINEL_PREFIX):
            continue
        counter, _, payload = line[len(GZ_SENTINEL_PREFIX) :].partition(" ")
        index_text, _, total_text = counter.partition("/")
        index, chunk_total = int(index_text), int(total_text)
        if total is None:
            total = chunk_total
        elif chunk_total != total:
            raise ValueError(
                f"chunk lines advertise conflicting totals {total} and {chunk_total}; "
                "the input mixes two emissions"
            )
        if not 1 <= index <= chunk_total:
            raise ValueError(f"chunk index {index} is outside 1..{chunk_total}")
        if index in chunks and chunks[index] != payload:
            raise ValueError(
                f"chunk {index} appears twice with different payloads; "
                "the input mixes two emissions"
            )
        chunks[index] = payload

    if total is None:
        raise ValueError(
            f"no '{SENTINEL_PREFIX.strip()}' or '{GZ_SENTINEL_PREFIX.strip()}' line in the input"
        )
    missing = sorted(set(range(1, total + 1)) - set(chunks))
    if missing:
        raise ValueError(f"manifest chunks {missing} are missing; got {len(chunks)} of {total}")
    blob = "".join(chunks[i] for i in range(1, total + 1))
    return _as_document(json.loads(gzip.decompress(base64.b64decode(blob)).decode("utf-8")))
