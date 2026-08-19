#!/usr/bin/env python3
"""Mask credential-shaped text out of stand output before it reaches a public log.

This repository is PUBLIC, and so is every line a workflow prints; cluster and
seeder output goes through here rather than straight to the console.

**Fail closed, per line.** A line this filter cannot prove it cleaned becomes
`LINE_MARKER`, and the pass at the end of `_clean` re-scans the *result* rather
than trusting the substitutions; an exception aborts the stream, because a
truncated diagnostic is recoverable and a published secret is not.

No structure awareness: a secret split across lines, or shaped like an ordinary
word, survives — the workflow's own no-print discipline covers the rest.

Usage:
    <producer> | redact-stand-log.py            # stdin -> stdout, line at a time
    redact-stand-log.py FILE [FILE ...]         # named files -> stdout
    redact-stand-log.py --max-line 400 FILE     # tighter truncation

Exit 0 when the stream was cleaned, 1 when it aborted — pipe under `pipefail`.
"""

from __future__ import annotations

import hashlib
import re
import sys
from collections.abc import Iterable, Iterator
from pathlib import Path

#: What an unrecoverable line becomes. Deliberately loud: a reader must be able
#: to tell "this line was removed" from "nothing was logged here".
LINE_MARKER = "[line withheld by CI — redaction could not be verified]"

#: Default ceiling on a single emitted line. Long enough for a helm error or a
#: Rust panic with a backtrace frame, short enough that a blob cannot ride out.
DEFAULT_MAX_LINE = 1000

#: Placeholder standing in for an image digest while the long-blob rule runs.
#: Restored verbatim afterwards — see `_clean`.
_DIGEST_SLOT = "\x00digest{}\x00"

_URL_CREDENTIALS = re.compile(r"(?P<scheme>[A-Za-z][A-Za-z0-9+.\-]*://)[^\s/@:]+:[^\s/@]+@")
_EMAIL = re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")
_JWT = re.compile(r"\beyJ[A-Za-z0-9_\-]{4,}\.[A-Za-z0-9_\-]{4,}(?:\.[A-Za-z0-9_\-]+)?")
_BEARER = re.compile(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]{8,}")
_SESSION_COOKIE = re.compile(r"(?i)(__Host-sid|__Secure-sid|sid)=[^\s;,\"']{8,}")
_PRIVATE_KEY_HEADER = re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")

#: Matched case-insensitively against the token left of a `:`/`=`. Boundary is
#: `(?<![A-Za-z0-9])`, not `\b`, which misses `MARIADB_PASSWORD=` — load-bearing.
_SECRET_KEY = re.compile(
    r"(?i)(?<![A-Za-z0-9])([A-Za-z0-9_.\-]*(?:"
    r"password|passwd|pwd|secret|secret[_\-]?key|client[_\-]?secret|"
    r"token|access[_\-]?token|refresh[_\-]?token|id[_\-]?token|bearer[_\-]?token|"
    r"api[_\-]?key|private[_\-]?key|signing[_\-]?key|"
    r"client-key-data|client-certificate-data|certificate-authority-data"
    r"))([\"']?\s*[:=]\s*)(?!\s*$)\S+"
)

#: An image digest: exempt, because it answers "which image failed to pull".
_DIGEST = re.compile(r"\bsha(?:256|512):[0-9a-fA-F]{32,128}\b")

#: The catch-all. 40 chars is above every object name and image tag in this
#: stack, below every key and token; alphabet covers base64 and base64url.
_LONG_BLOB = re.compile(r"(?<![A-Za-z0-9+/_=-])[A-Za-z0-9+/_-]{40,}={0,2}(?![A-Za-z0-9+/_=-])")

_IPV4 = re.compile(r"(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])")


def _blob_mask(match: re.Match[str]) -> str:
    """Mask unless the token provably is not base64 of either variant.

    Standard base64 never contains `_`/`-`; base64url never contains `/`/`+`.
    A token mixing the two exclusive sets (a pytest path like
    tests/stand/ui/test_logged_out_access_refused) is therefore not a blob.
    """
    token = match.group(0)
    if ("/" in token or "+" in token) and ("-" in token or "_" in token):
        return token
    return "[blob redacted]"


def _blob_survivor(text: str) -> bool:
    """True when a maskable (non-exempt) blob is still present in `text`."""
    return any(_blob_mask(m) != m.group(0) for m in _LONG_BLOB.finditer(_DIGEST.sub("", text)))


def _email_slot(match: re.Match[str]) -> str:
    """A stable one-way handle for one address.

    Six hex characters: enough that two personas in the same log are told apart,
    far too few to attack, and short enough to keep a table readable.
    """
    digest = hashlib.sha256(match.group(0).lower().encode("utf-8")).hexdigest()[:6]
    return f"[email:{digest}]"


def _clean(line: str, *, max_line: int) -> str:
    """Return `line` with every credential-shaped run replaced.

    Order is load-bearing. URLs go first so a DSN's password is masked as a URL
    credential rather than half-caught by the email rule; digests are parked
    before the long-blob sweep and restored after it; the verification pass runs
    last, over the bytes that are actually about to be printed.
    """
    if _PRIVATE_KEY_HEADER.search(line):
        # The header names the key type and nothing else useful, and the body
        # is caught by the blob rule. A PEM in a CI log is never a diagnostic.
        return "[private key material withheld by CI]"

    out = _URL_CREDENTIALS.sub(r"\g<scheme>[credentials redacted]@", line)
    out = _JWT.sub("[jwt redacted]", out)
    out = _BEARER.sub(r"\1 [redacted]", out)
    out = _SESSION_COOKIE.sub(r"\1=[redacted]", out)
    out = _SECRET_KEY.sub(r"\1\2[redacted]", out)
    out = _EMAIL.sub(_email_slot, out)

    digests: list[str] = []

    def _park(match: re.Match[str]) -> str:
        digests.append(match.group(0))
        return _DIGEST_SLOT.format(len(digests) - 1)

    out = _DIGEST.sub(_park, out)
    out = _LONG_BLOB.sub(_blob_mask, out)
    for index, digest in enumerate(digests):
        out = out.replace(_DIGEST_SLOT.format(index), digest)

    out = _IPV4.sub("[ip redacted]", out)

    if len(out) > max_line:
        out = out[:max_line] + f" …[truncated at {max_line} chars by CI]"

    # The result, not the input: anything credential-shaped that survived means
    # a rule has a hole. Digests stripped first so an exempt one cannot trip it.
    if _EMAIL.search(out) or _JWT.search(out) or _PRIVATE_KEY_HEADER.search(out) or _blob_survivor(out):
        return LINE_MARKER
    return out


def _stream(lines: Iterable[str], *, max_line: int) -> Iterator[str]:
    for line in lines:
        yield _clean(line.rstrip("\n"), max_line=max_line)


def main(argv: list[str]) -> int:
    args = argv[1:]
    max_line = DEFAULT_MAX_LINE
    if args[:1] == ["--max-line"]:
        if len(args) < 2 or not args[1].isdigit() or int(args[1]) < 80:
            print("--max-line needs a whole number of at least 80", file=sys.stderr)  # noqa: T201
            return 2
        max_line = int(args[1])
        args = args[2:]

    try:
        if not args:
            for cleaned in _stream(sys.stdin, max_line=max_line):
                print(cleaned, flush=True)  # noqa: T201
            return 0
        for path in args:
            with Path(path).open(encoding="utf-8", errors="replace") as handle:
                for cleaned in _stream(handle, max_line=max_line):
                    print(cleaned, flush=True)  # noqa: T201
    except BrokenPipeError:
        # The consumer went away (a `head`, or a cancelled step). Not a
        # redaction failure, and not worth a red X.
        return 0
    except Exception as exc:  # noqa: BLE001 — any failure means "stop printing"
        # Whatever was emitted before this point was cleaned; what follows was
        # not, so nothing follows.
        print(f"::error::redaction aborted, output truncated: {type(exc).__name__}: {exc}", file=sys.stderr)  # noqa: T201
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
