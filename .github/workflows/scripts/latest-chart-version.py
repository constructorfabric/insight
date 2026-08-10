#!/usr/bin/env python3
"""Print the newest published semver tag of an OCI chart repository.

Usage:  latest-chart-version.py oci://ghcr.io/<owner>/<path>/<chart>

WHY THE REGISTRY AND NOT A FILE IN THE REPOSITORY
-------------------------------------------------
`deploy/gitops/.insight-version` names the version the publish job last pushed,
and reading it looks like the obvious answer. It is wrong anywhere the checkout
is not the upstream default branch, and wrong SILENTLY:

  * read from `main` of $GITHUB_REPOSITORY, a run in a fork reads the fork's
    mirror of main. One observed run resolved 0.4.8 against a stand running
    0.5.x — a downgrade of five minor versions, reported as a successful deploy
    because the post-upgrade check compares against the version REQUESTED.
  * read from the checkout, it is as old as the branch, which is the same bug
    with a smaller blast radius.

The registry has no such failure mode. It is what the input refers to — a
version that "must already be published" — and it gives the same answer to
whoever asks.

WHY PAGINATION IS NOT OPTIONAL
------------------------------
`/tags/list` returns 100 entries by default and the registry does not order
them, so a maximum taken from the first page is a maximum of an arbitrary
subset. This repository already has enough tags for that to select a version
from an entirely different minor series. `?n=1000` collects them in one request
today; the `Link: rel="next"` header is followed regardless, so nothing breaks
when it no longer does.

Only strict `X.Y.Z` tags are considered, and they are compared as integer
tuples: as strings, "0.5.9" sorts above "0.5.115".

No credential is needed — GHCR issues an anonymous pull token for a public
package. Diagnostics go to stderr; the version alone goes to stdout, so the
caller can use it directly.
"""

from __future__ import annotations

import json
import re
import sys
import urllib.error
import urllib.request

#: A published chart version. Deliberately strict: no pre-release, no build
#: metadata, no `v` prefix — the publish job emits exactly this shape, and a
#: looser pattern would let a hand-pushed experiment win a comparison.
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")

#: Enough that one request suffices today; the Link header is followed anyway.
PAGE_SIZE = 1000

#: A stop, not an expectation. Without it a registry that echoed a self-
#: referential `next` would spin forever.
MAX_PAGES = 20

_NEXT = re.compile(r'<([^>]+)>;\s*rel="next"')


def _split_ref(ref: str) -> tuple[str, str]:
    """`oci://host/a/b/c` -> ("host", "a/b/c")."""
    body = ref.removeprefix("oci://").strip("/")
    host, _, repository = body.partition("/")
    if not host or not repository:
        raise ValueError(f"not an OCI chart reference: {ref!r}")
    return host, repository


def _anonymous_token(host: str, repository: str) -> str:
    url = f"https://{host}/token?scope=repository:{repository}:pull&service={host}"
    with urllib.request.urlopen(url, timeout=30) as response:
        return str(json.load(response)["token"])


def tags(ref: str) -> list[str]:
    host, repository = _split_ref(ref)
    token = _anonymous_token(host, repository)

    collected: list[str] = []
    url: str | None = f"https://{host}/v2/{repository}/tags/list?n={PAGE_SIZE}"
    for _ in range(MAX_PAGES):
        if url is None:
            break
        request = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
        with urllib.request.urlopen(request, timeout=30) as response:
            collected += json.load(response).get("tags") or []
            link = response.headers.get("Link")
        match = _NEXT.search(link or "")
        url = f"https://{host}{match.group(1)}" if match else None
    return collected


def newest(all_tags: list[str]) -> str:
    versions = [t for t in all_tags if SEMVER.match(t)]
    if not versions:
        raise ValueError("the repository publishes no X.Y.Z tags")
    return max(versions, key=lambda t: tuple(int(part) for part in t.split(".")))


def main(argv: list[str]) -> int:
    # T201 suppressed per line, as in this directory's other tools: stdout IS the
    # answer here and stderr IS the diagnosis. A logger would put both somewhere
    # the calling workflow step does not read.
    if len(argv) != 2:
        print(f"usage: {argv[0]} oci://<host>/<repository>", file=sys.stderr)  # noqa: T201
        return 2
    try:
        found = tags(argv[1])
        version = newest(found)
    except (ValueError, urllib.error.URLError, urllib.error.HTTPError, KeyError, TimeoutError) as exc:
        print(f"cannot resolve the newest published chart version: {exc}", file=sys.stderr)  # noqa: T201
        print("Pass chart_version explicitly.", file=sys.stderr)  # noqa: T201
        return 1

    print(f"{len(found)} tags, newest X.Y.Z is {version}", file=sys.stderr)  # noqa: T201
    print(version)  # noqa: T201 — stdout is the answer; the caller consumes it
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
