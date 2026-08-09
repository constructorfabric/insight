#!/usr/bin/env python3
"""Mask credentials out of deployed-stand output before it reaches a public run log.

This repository is PUBLIC, and so is every line a workflow prints. The test-stand
deploy talks to a real cluster with a real credential, seeds real personas and
then signs them in — three activities whose natural output carries session
cookies, bearer tokens, connection strings and people's email addresses. Nothing
in that chain was written with a public audience in mind: `seed-stand.sh` prints
the seed manifest so a discarded pod's work is not lost, and `kubectl logs`
prints whatever the container felt like printing.

So the workflow never pipes cluster or seeder output straight to the console.
Everything goes through this filter, and the filter is the only thing standing
between a container that decided to log its DSN and a permanent public record of
it.

**Fail closed, per line.** A line this script cannot prove it has cleaned is
replaced wholesale with a marker rather than passed through. The verification
pass at the end of `_clean` re-scans the *result*: redaction that claims to have
worked is not the same as redaction that worked, and the cheap way to tell them
apart is to look at the bytes about to be printed. An exception anywhere aborts
the stream — a truncated diagnostic is recoverable, a published secret is not.

What is masked, and why each rule exists:

* **URLs carrying credentials** (`scheme://user:pass@host`) — the shape a DSN
  takes in a connection error. Run first, because the `pass@host.example` tail
  also matches the email rule and the URL form is the more informative mask.
* **Email addresses** — replaced by a short, stable digest rather than a flat
  marker. Two lines about the same person still visibly concern the same person,
  which is most of what an address is worth in a diagnostic, and the digest is
  one-way. Seeded personas live on a synthetic domain, but the `--email` the
  seeder is pointed at is a real one.
* **`Bearer` values, JWTs and `__Host-sid` cookies** — the credentials the smoke
  stage handles. A JWT is recognised by its `eyJ` header prefix, so an
  unsigned or truncated one is caught too.
* **`key: value` pairs whose KEY names a secret** — `password:`, `token:`,
  `client-key-data:` and friends. This is what a kubeconfig, a Secret dump or a
  helm values render looks like, and the key name is the only reliable signal.
* **Long unbroken base64/hex runs** — the catch-all for a secret whose shape
  nobody anticipated. Container image digests are exempted (they are
  diagnostics, not credentials, and losing them makes a pull failure
  unreadable); everything else of 40+ characters is assumed to be material.
* **IPv4 literals** — not credentials, but infrastructure detail this repo does
  not publish. Chart versions and other three-part numbers are untouched: the
  rule needs four octets.
* **Over-long lines** — truncated. A 40 KB line is either a heap dump or a blob,
  and neither belongs in a run log; truncating it also bounds what an unknown
  secret shape can leak past the rules above.

What this does NOT do, stated plainly: it does not understand structure. A
secret spread across two lines, or one that looks like an ordinary English word,
survives. The rule this implements is "credential-shaped text does not reach the
console"; the workflow's own discipline — no `kubectl describe`, no environment
dumps, no artifact uploads — is what covers the rest.

Usage:
    <producer> | redact-stand-log.py            # stdin -> stdout, line at a time
    redact-stand-log.py FILE [FILE ...]         # named files -> stdout
    redact-stand-log.py --max-line 400 FILE     # tighter truncation

Exit status is 0 when the whole stream was cleaned and emitted, 1 when the
filter aborted. A caller that pipes into this script should run under
`set -o pipefail` so an abort is not mistaken for a clean run.
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

#: Keys whose value is material wherever they appear — kubeconfig, Secret dumps,
#: rendered values, connection errors. Matched case-insensitively against the
#: token immediately left of a `:` or `=`.
#:
#: The leading boundary is `(?<![A-Za-z0-9])`, NOT `\b`, and an optional prefix
#: is allowed to precede the keyword. This is the difference between redacting
#: and not redacting, because `\b` does not match between `_` and a letter —
#: underscore is a word character. With `\b` the pattern matched a bare
#: `password:` but sailed straight past every underscore-joined name, which is
#: the dominant convention in this stack:
#:
#:     MARIADB_PASSWORD=…                                     leaked
#:     kubectl_token=…                                        leaked
#:     APP__gears__authenticator__config__idp__client_secret: leaked
#:
#: On a public repository those lines are published forever, so the boundary is
#: load-bearing rather than stylistic. The prefix group is bounded to characters
#: that appear in env-var and YAML key names so it cannot run away across a line.
_SECRET_KEY = re.compile(
    r"(?i)(?<![A-Za-z0-9])([A-Za-z0-9_.\-]*(?:"
    r"password|passwd|pwd|secret|secret[_\-]?key|client[_\-]?secret|"
    r"token|access[_\-]?token|refresh[_\-]?token|id[_\-]?token|bearer[_\-]?token|"
    r"api[_\-]?key|private[_\-]?key|signing[_\-]?key|"
    r"client-key-data|client-certificate-data|certificate-authority-data"
    r"))([\"']?\s*[:=]\s*)(?!\s*$)\S+"
)

#: An image digest: kept, because "which image failed to pull" is the answer to
#: half the seed failures there are.
_DIGEST = re.compile(r"\bsha(?:256|512):[0-9a-fA-F]{32,128}\b")

#: The catch-all. 40 characters is above every Kubernetes object name and image
#: tag in this stack (all of which carry `-`, `.` or `/`) and below every key,
#: token and certificate body.
_LONG_BLOB = re.compile(r"(?<![A-Za-z0-9+/=])[A-Za-z0-9+/]{40,}={0,2}(?![A-Za-z0-9+/=])")

_IPV4 = re.compile(r"(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])")


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
        # The header line names the key type and nothing else useful, and the
        # body that follows is caught by the blob rule. Drop the whole thing —
        # a PEM in a CI log is never a diagnostic.
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
    out = _LONG_BLOB.sub("[blob redacted]", out)
    for index, digest in enumerate(digests):
        out = out.replace(_DIGEST_SLOT.format(index), digest)

    out = _IPV4.sub("[ip redacted]", out)

    if len(out) > max_line:
        out = out[:max_line] + f" …[truncated at {max_line} chars by CI]"

    # The result, not the input, is what gets published — so the check is on the
    # result. Anything credential-shaped that survived every rule above means a
    # rule has a hole, and the safe reading of a hole is "do not print this".
    if _EMAIL.search(out) or _JWT.search(out) or _PRIVATE_KEY_HEADER.search(out):
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
