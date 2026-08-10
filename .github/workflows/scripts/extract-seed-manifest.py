#!/usr/bin/env python3
"""Recover the seed manifest from a seed run's merged log.

Usage:  extract-seed-manifest.py <seed-log> <manifest-out>

WHY THIS FILE EXISTS AT ALL, RATHER THAN INLINE IN ITS TWO CALLERS
-----------------------------------------------------------------
Two things read a seed log for its manifest: the `Stage 2/3 — capture the seed
manifest` step in .github/workflows/deploy-test-stand.yml, and
`extract_manifest()` in deploy/gitops/scripts/emulate-ci-deploy.sh. That harness
exists to prove the laptop path and the CI path run the SAME commands, and it
prints a parity report saying so.

They were two implementations. When the CI one was fixed for interleaved log
lines the shell one was not, and its comment went on claiming it matched — the
harness asserting a parity it no longer had, which is worse than not checking.
Extracting the logic here makes the parity structural instead of asserted: there
is one algorithm and both callers invoke it, so neither can drift from the other
without deleting this file.

WHY THE SCAN IS NOT "the last `{` at column 0 through the next `}` at column 0"
------------------------------------------------------------------------------
That is what both callers used to do, and it is wrong for a reason no amount of
care about brace nesting can fix: THE DOCUMENT AND THE LOG LINES ARE TWO
DIFFERENT STREAMS. The seeder prints the manifest to stdout, while the logging
handler it installs writes to stderr. Both `kubectl logs` and the shell
redirection that captures a seed run merge the two, and the container runtime
that recorded them drained the pipes independently — so a twelve-kilobyte
document does not arrive in one piece, and a log line written while it is being
drained is recorded BETWEEN two of the document's lines. The seeder logs
immediately after printing the manifest, which makes this the ordinary case
rather than an exotic one.

The old scan joined every line of the span, so one interleaved line made
`json.loads` raise, the step failed, and the smoke stage aborted for want of a
manifest — on a run whose deploy and seed had both succeeded. Please do not
simplify it back.

Two defences, in order:

  1. Drop the interlopers. The seeder's handler opens every line with a
     timestamp, a level and a dotted logger name. No line of a
     `json.dumps(indent=2)` document can match that: they are `{`, `}`, or
     indented by at least two spaces. Matching the logger's SHAPE rather than a
     keyword list is what keeps a persona's display name from being filtered out
     of the document.
  2. Try the surviving spans instead of trusting one, and accept only a span
     that parses AND carries `manifest_version`. That covers what rule 1 cannot:
     a traceback, a log record with an embedded newline, or some other column-0
     `{`…`}` block in the stream — the seed steps shell out to helpers on the
     container's own file descriptors, and their output is not the seeder's to
     shape.

Diagnostics go to stderr in plain text and the exit status is what callers
branch on; neither caller's annotation style is baked in here.
"""

from __future__ import annotations

import json
import re
import sys
from itertools import islice
from pathlib import Path

#: The seeder's log-line shape: "<date> <time>,<ms> <LEVEL> <dotted.logger> ".
#: Anchored at column 0, so an indented document line can never match.
LOG_LINE = re.compile(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}[,.]\d{3} [A-Z]+ [\w.]+ ")

#: A seed log is a log, not a JSON haystack. Bounded so a pathological stream
#: cannot turn this into a quadratic parse.
MAX_SPANS = 32


def extract(lines: list[str]) -> dict:
    """Return the manifest document, or raise ValueError with a readable cause."""
    kept = [line for line in lines if not LOG_LINE.match(line)]

    opens = [i for i, line in enumerate(kept) if line == "{"]
    closes = [i for i, line in enumerate(kept) if line == "}"]
    if not opens:
        raise ValueError("no manifest in the seed log — the seeder did not reach the end of its run")

    # Latest opening brace first, because a run that printed more than one
    # document is a run whose later document supersedes the earlier. Nearest
    # closing brace first within each, so the ordinary run costs exactly one
    # parse. A generator rather than a list comprehension sliced afterwards:
    # a log is untrusted input, and the full cross-product of braces in a
    # pathological one is not worth materialising to then throw away.
    spans = list(islice(((start, end) for start in reversed(opens) for end in closes if end > start), MAX_SPANS))
    if not spans:
        raise ValueError("the manifest block is never closed — the stream was truncated")

    failure: Exception | None = None
    for start, end in spans:
        try:
            candidate = json.loads("\n".join(kept[start : end + 1]))
        except json.JSONDecodeError as exc:
            failure = exc
            continue
        if isinstance(candidate, dict) and "manifest_version" in candidate:
            return candidate

    reason = failure if failure is not None else "no block carried a manifest_version key"
    raise ValueError(
        f"the log holds a manifest-shaped block that will not read as one ({reason}). "
        "The usual cause is seeder output interleaved into the document that the "
        "log-line filter did not recognise: the seeder prints the manifest on stdout "
        "and logs on stderr, and this log is the two merged."
    )


def main(argv: list[str]) -> int:
    # T201 is suppressed line by line below rather than for the module: this is a
    # command-line tool whose stdout IS its output and whose stderr IS its
    # diagnosis, the same disposition redact-stand-log.py takes. A logger here
    # would put the summary somewhere neither caller reads.
    if len(argv) != 3:
        print(f"usage: {argv[0]} <seed-log> <manifest-out>", file=sys.stderr)  # noqa: T201
        return 2

    source, target = Path(argv[1]), Path(argv[2])
    try:
        lines = source.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as exc:
        print(f"cannot read the seed log: {exc}", file=sys.stderr)  # noqa: T201
        return 1

    try:
        doc = extract(lines)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)  # noqa: T201
        return 1

    with target.open("w", encoding="utf-8") as handle:
        json.dump(doc, handle, indent=2, sort_keys=True)
        handle.write("\n")

    # A summary, not the document. Fixture NAMES are contract; persona emails,
    # UUIDs and the tenant are printed nowhere.
    fixtures = sorted((doc.get("fixtures") or {}).keys())
    personas = doc.get("personas") or []
    print(f"manifest_version: {doc.get('manifest_version')}")  # noqa: T201
    print(f"anchor_date:      {doc.get('anchor_date')}")  # noqa: T201
    print(f"data_window:      {doc.get('data_window')}")  # noqa: T201
    print(f"seed_revision:    {doc.get('seed_revision')}")  # noqa: T201
    print(f"personas:         {len(personas)}")  # noqa: T201
    print(f"fixtures:         {', '.join(fixtures) if fixtures else '(none)'}")  # noqa: T201
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
