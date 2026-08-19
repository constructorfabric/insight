#!/usr/bin/env python3
"""Summarise a TruffleHog JSONL scan and decide whether it blocks.

A finding is identified by sha256(detector | path | raw), which survives rebases
because it carries no commit id. Fingerprints listed in the allowlist are known
false positives and never block; anything else does, in `block` mode.

Raw values are never written to the summary, the log or an artifact: this
repository is public, so a redacted table is all that leaves the runner.

    trufflehog_gate.py FINDINGS.jsonl SUMMARY.md ALLOWLIST [block|report] TITLE [NEW.json]

With NEW.json the findings that are not in the allowlist are also written there,
without their values, for a follow-up step to act on.
"""

import collections
import hashlib
import json
import sys
from pathlib import Path


def fingerprint(detector: str, path: str, raw: str) -> str:
    return hashlib.sha256(f"{detector}|{path}|{raw}".encode()).hexdigest()[:16]


def read_allowlist(path: str) -> dict[str, str]:
    """{fingerprint: reason}. Lines are `<fp>  <detector>  <path>  # reason`."""
    entries = {}
    try:
        fh = Path(path).open(encoding="utf-8")
    except FileNotFoundError:
        return entries
    with fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            fp = line.split()[0]
            reason = line.split("#", 1)[1].strip() if "#" in line else ""
            entries[fp] = reason
    return entries


def read_findings(path: str):
    rows, unparsable, skipped = [], 0, 0
    with Path(path).open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                # A truncated or non-JSON line must not take the whole summary down
                # with it; it is counted and reported instead.
                unparsable += 1
                continue
            if "DetectorName" not in d:
                # Only findings carry a detector. Anything else on stdout is not a
                # finding and must not inflate the count.
                skipped += 1
                continue
            meta = (d.get("SourceMetadata") or {}).get("Data", {})
            # The GitHub source reports under Github for commits and comments alike,
            # and a comment hit carries a link where a commit hit carries a path.
            loc = meta.get("Git") or meta.get("Github") or {}
            detector = d.get("DetectorName", "?")
            path_ = loc.get("file") or loc.get("link") or "?"
            rows.append(
                {
                    "detector": detector,
                    "verified": bool(d.get("Verified")),
                    "commit": loc.get("commit") or "",
                    "file": path_,
                    "line": loc.get("line") or "",
                    "fp": fingerprint(detector, path_, d.get("Raw") or ""),
                }
            )
    return rows, unparsable, skipped


def main() -> int:
    findings, summary_path, allowlist_path, mode, title = sys.argv[1:6]
    new_json = sys.argv[6] if len(sys.argv) > 6 else None
    rows, unparsable, skipped = read_findings(findings)
    allowed = read_allowlist(allowlist_path)

    known = [r for r in rows if r["fp"] in allowed]
    new = [r for r in rows if r["fp"] not in allowed]

    if new_json:
        with Path(new_json).open("w", encoding="utf-8") as fh:
            json.dump(new, fh, indent=2)

    with Path(summary_path).open("a", encoding="utf-8") as out:
        w = out.write
        w(f"## TruffleHog — {title}\n\n")
        if unparsable or skipped:
            w(f"_{unparsable} unparsable line(s), {skipped} non-finding record(s) skipped._\n\n")
        if not new:
            if mode == "block" and unparsable:
                # A truncated stream drops every finding after the break, so an empty
                # result cannot be read as a pass.
                w(
                    f"Scan output was incomplete: {unparsable} unparsable line(s). Findings"
                    " after the break are missing, so this is not a pass.\n"
                )
                return 1
            w(f"No new secrets. {len(known)} known finding(s) matched the allowlist.\n")
            return 0
        ver = sum(1 for r in new if r["verified"])
        w(
            f"**{len(new)}** new finding(s) — {ver} verified, {len(new) - ver} unverified"
            f" ({len(known)} known, allowlisted). Values are redacted here on purpose;"
            " read them from the source commit.\n\n"
        )
        w("| Detector | Verified | Commit | Path | Fingerprint |\n|---|---|---|---|---|\n")
        for r in sorted(new, key=lambda r: (not r["verified"], r["detector"])):
            loc = f":{r['line']}" if r["line"] else ""
            w(
                f"| `{r['detector']}` | {'yes' if r['verified'] else 'no'} | `{r['commit'][:8]}` |"
                f" `{r['file']}`{loc} | `{r['fp']}` |\n"
            )
        by_det = collections.Counter(r["detector"] for r in new)
        w(f"\nBy detector: {', '.join(f'{k} x{v}' for k, v in by_det.most_common())}\n")
        w(
            "\n**Revoke and rotate every credential listed above.** Removing the commit is not"
            " remediation — assume the value is compromised the moment it was pushed. If a finding"
            " is a detector false positive on synthetic data, add its fingerprint to"
            f" `{allowlist_path}` with a reason, in this pull request, so the claim is reviewed.\n"
        )
    return 1 if mode == "block" else 0


if __name__ == "__main__":
    sys.exit(main())
