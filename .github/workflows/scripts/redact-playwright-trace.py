#!/usr/bin/env python3
"""Strip credential-bearing headers from Playwright traces before they leave CI.

A Playwright trace is a replay of the browser session, and the journeys under
`tests/stand/ui/` sign a real persona in against a real IdP. So a captured trace
carries the session cookie (`__Host-sid`) and every `Authorization` header the
SPA sent — live credentials for the stand the run was aimed at. An uploaded
artifact is downloadable by anyone with read access to the run, for as long as
the retention window lasts.

This runs BETWEEN capture and upload, never after. The workflow ordering is the
control; this script is what makes the ordering worth anything.

**Fail closed.** A trace this script cannot fully parse and rewrite is DELETED
rather than passed through, and the script exits non-zero. Losing a diagnostic
is recoverable; publishing a session cookie is not.

What is redacted, in every JSON structure inside `*.trace` and `*.network`:

* header lists — `[{"name": "cookie", "value": "..."}]` — the shape Playwright
  records request and response headers in;
* header maps — `{"headers": {"authorization": "Bearer ..."}}`;
* any object key that is itself a sensitive header name;
* **every `cookies` array**, whatever the individual cookie is called.

A trace records each request TWICE: once as raw headers, and once as a parsed
HAR entry with `request.cookies` / `response.cookies` as
`[{"name": "__Host-sid", "value": "..."}]`. Redacting by header name leaves that
second copy untouched — the name there is the COOKIE's name, not `Cookie` — and
a real Chromium trace kept the session value in `trace.network` with only the
header rule in place. So cookie values are redacted by POSITION (inside a
`cookies` array) rather than by name, and no future cookie name has to be added
here to stay covered.

Matching is case-insensitive: the wire is case-insensitive about header names
and traces preserve whatever casing the sender used.

**What this does NOT cover, stated plainly.** Response *bodies* are stored as
`resources/<sha1>` blobs and are left alone — a token in a JSON body survives.
The rule this implements is about headers. Anyone widening the artifact policy
to bodies should widen this script in the same change.

Usage:
    redact-playwright-trace.py <dir> [<dir> ...]

Every `*.zip` found beneath each directory is treated as a trace.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Any

#: Header names whose values are credentials. Lowercase; matching lowercases the
#: candidate before comparing.
SENSITIVE_HEADERS: frozenset[str] = frozenset({"cookie", "set-cookie", "authorization", "proxy-authorization"})

#: What a redacted value becomes. Deliberately not an empty string — a reader of
#: the trace should be able to tell "this header was present and removed" from
#: "this header was never sent", and the verification pass below looks for it.
PLACEHOLDER = "[redacted by CI]"

#: The HAR key holding parsed cookies. Every value inside is a credential
#: regardless of what the cookie is named — see the module docstring.
COOKIE_ARRAY_KEY = "cookies"

#: Zip members parsed as NDJSON. Everything else is copied through byte for byte.
NDJSON_SUFFIXES = (".trace", ".network")


def _redact_cookie_array(node: Any) -> Any:
    """Blank the `value` of every entry in a HAR `cookies` array.

    The cookie's NAME is kept: a trace that still shows `__Host-sid` was present
    is a useful diagnostic, and the name is not the secret.
    """
    if not isinstance(node, list):
        return _redact(node)
    return [{**item, "value": PLACEHOLDER} if isinstance(item, dict) and "value" in item else item for item in node]


def _redact(node: Any) -> Any:
    """Return `node` with every sensitive header and cookie value replaced.

    Recursive because the shapes are not fixed: headers appear on request and
    response snapshots, on action parameters, and nested inside trace events
    that this script has no reason to enumerate exhaustively. Walking
    everything means a Playwright version that moves a header does not silently
    defeat the redaction.
    """
    if isinstance(node, dict):
        # The `{"name": ..., "value": ...}` header-pair shape.
        name = node.get("name")
        if isinstance(name, str) and name.lower() in SENSITIVE_HEADERS and "value" in node:
            return {**{k: _redact(v) for k, v in node.items()}, "value": PLACEHOLDER}
        result: dict[Any, Any] = {}
        for key, value in node.items():
            if key == COOKIE_ARRAY_KEY:
                result[key] = _redact_cookie_array(value)
            elif isinstance(key, str) and key.lower() in SENSITIVE_HEADERS and isinstance(value, str):
                # The `{"cookie": "...", "authorization": "..."}` header-map shape.
                result[key] = PLACEHOLDER
            else:
                result[key] = _redact(value)
        return result
    if isinstance(node, list):
        return [_redact(item) for item in node]
    return node


def _redact_ndjson(raw: bytes) -> bytes:
    """Rewrite one NDJSON member. Raises if any line is not JSON."""
    out: list[bytes] = []
    for number, line in enumerate(raw.split(b"\n"), start=1):
        if not line.strip():
            out.append(line)
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError as exc:  # noqa: PERF203 — the line number matters
            raise ValueError(f"line {number} is not JSON: {exc}") from exc
        out.append(json.dumps(_redact(parsed), separators=(",", ":")).encode())
    return b"\n".join(out)


def _find_leaks(archive: Path) -> list[str]:
    """Sensitive header values still present in a rewritten trace.

    The point of re-opening the file we just wrote is that redaction claiming to
    have worked is not the same as it having worked. This reads the result back
    and reports any sensitive header whose value is not the placeholder.
    """
    leaks: list[str] = []

    def scan(node: Any, where: str) -> None:
        if isinstance(node, dict):
            name = node.get("name")
            if isinstance(name, str) and name.lower() in SENSITIVE_HEADERS:
                value = node.get("value")
                if isinstance(value, str) and value != PLACEHOLDER:
                    leaks.append(f"{where}: header {name!r} survived redaction")
            for key, value in node.items():
                if (
                    isinstance(key, str)
                    and key.lower() in SENSITIVE_HEADERS
                    and isinstance(value, str)
                    and value != PLACEHOLDER
                ):
                    leaks.append(f"{where}: key {key!r} survived redaction")
                if key == COOKIE_ARRAY_KEY and isinstance(value, list):
                    for cookie in value:
                        if (
                            isinstance(cookie, dict)
                            and isinstance(cookie.get("value"), str)
                            and cookie["value"] != PLACEHOLDER
                        ):
                            leaks.append(f"{where}: cookie {cookie.get('name')!r} survived redaction")
                scan(value, where)
        elif isinstance(node, list):
            for item in node:
                scan(item, where)

    with zipfile.ZipFile(archive) as zf:
        for info in zf.infolist():
            if not info.filename.endswith(NDJSON_SUFFIXES):
                continue
            for line in zf.read(info).split(b"\n"):
                if not line.strip():
                    continue
                scan(json.loads(line), f"{archive.name}!{info.filename}")
    return leaks


def redact_archive(archive: Path) -> None:
    """Rewrite one trace zip in place, then prove the rewrite held."""
    with tempfile.TemporaryDirectory() as tmp:
        rewritten = Path(tmp) / archive.name
        with zipfile.ZipFile(archive) as src, zipfile.ZipFile(rewritten, "w", zipfile.ZIP_DEFLATED) as dst:
            for info in src.infolist():
                raw = src.read(info)
                if info.filename.endswith(NDJSON_SUFFIXES):
                    raw = _redact_ndjson(raw)
                # A fresh ZipInfo: the rewritten member has a different size, and
                # carrying the original's compression metadata across would make
                # the entry describe bytes that are no longer there.
                dst.writestr(info.filename, raw)
        shutil.move(str(rewritten), str(archive))

    leaks = _find_leaks(archive)
    if leaks:
        raise ValueError("; ".join(leaks[:5]))


def main(argv: list[str]) -> int:
    roots = [Path(a) for a in argv[1:]]
    if not roots:
        print("usage: redact-playwright-trace.py <dir> [<dir> ...]", file=sys.stderr)  # noqa: T201
        return 2

    traces = sorted({p for root in roots if root.is_dir() for p in root.rglob("*.zip")})
    if not traces:
        # Not an error. A green run with `--tracing=retain-on-failure` keeps no
        # trace at all, and that is the common case.
        print("no trace archives found — nothing to redact")  # noqa: T201
        return 0

    failed = 0
    for trace in traces:
        try:
            redact_archive(trace)
        except Exception as exc:  # noqa: BLE001 — any failure means "do not publish"
            failed += 1
            trace.unlink(missing_ok=True)
            print(f"::error::deleted {trace} — could not verify redaction: {exc}")  # noqa: T201
        else:
            print(f"redacted {trace}")  # noqa: T201

    print(f"redacted {len(traces) - failed}/{len(traces)} trace archive(s)")  # noqa: T201
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
