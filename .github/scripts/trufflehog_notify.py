#!/usr/bin/env python3
"""Turn findings the nightly scan reports into issues assigned to their author.

The nightly jobs never fail a run, so without this a finding outside the
baseline is only a line in a summary nobody opens. One issue per fingerprint:
they close independently, and an open one suppresses the next night's duplicate.

Values are never included — the issue names the detector, the path and the
commit, which is what the assignee needs to find it.

    trufflehog_notify.py NEW.json owner/name [--dry-run]
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path


def gh(*args: str) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def author_of(repo: str, sha: str, cache: dict[str, str]) -> str:
    """GitHub login for a commit, or "" when neither route resolves it.

    The commit's own author is only known to GitHub when the address in it is
    attached to an account, which a work address often is not. The pull request
    the commit arrived with names an account either way, so it is the second
    route; a commit pushed straight to a branch has neither.
    """
    if not sha:
        return ""
    if sha not in cache:
        login = gh("api", f"repos/{repo}/commits/{sha}", "--jq", ".author.login // empty")
        if not login:
            login = gh("api", f"repos/{repo}/commits/{sha}/pulls", "--jq", ".[0].user.login // empty")
        cache[sha] = login or ""
    return cache[sha]


def already_open(repo: str, fingerprint: str) -> bool:
    found = gh(
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        "open",
        "--search",
        f"{fingerprint} in:title",
        "--json",
        "number",
        "--jq",
        ".[0].number // empty",
    )
    return bool(found)


def body_for(f: dict, login: str) -> str:
    who = (
        f"[@{login}](https://github.com/{login})"
        if login
        else "an author GitHub could not attribute — open the commit to see who"
    )
    return "\n".join(
        [
            "The nightly secret scan reports a finding that is not in `.github/trufflehog-allowlist.txt`.",
            "",
            "| Detector | Verified | Commit | Path | Fingerprint |",
            "|---|---|---|---|---|",
            f"| `{f['detector']}` | {'yes' if f['verified'] else 'no'} | `{f['commit'][:8]}` |"
            f" `{f['file']}`{':' + str(f['line']) if f['line'] else ''} | `{f['fp']}` |",
            "",
            f"It arrived with [{f['commit'][:8]}](../../commit/{f['commit']}), by {who}."
            " The value is deliberately not repeated here; read it from that commit.",
            "",
            "Two ways to close this:",
            "",
            "- **It is a credential.** Revoke and rotate it. Removing the commit is not remediation —"
            " assume it was compromised the moment it was pushed.",
            "- **It is a false positive.** Add the fingerprint to `.github/trufflehog-allowlist.txt`"
            " with a reason, in a pull request, so the claim is reviewed.",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("findings", help="JSON written by trufflehog_gate.py")
    parser.add_argument("repo", help="owner/name")
    parser.add_argument("--dry-run", action="store_true", help="print what would be opened, touch nothing")
    args = parser.parse_args()

    with Path(args.findings).open(encoding="utf-8") as fh:
        findings = json.load(fh)

    cache: dict[str, str] = {}
    created = skipped = 0
    for f in findings:
        if already_open(args.repo, f["fp"]):
            skipped += 1
            continue
        login = author_of(args.repo, f.get("commit", ""), cache)
        title = f"Possible secret in {f['file']} ({f['fp']})"
        if args.dry_run:
            print(f"would open: {title}  -> assignee: {login or '(none)'}")  # noqa: T201
            created += 1
            continue
        issue_args = ["issue", "create", "--repo", args.repo, "--title", title, "--body", body_for(f, login)]
        if login:
            issue_args += ["--assignee", login]
        print(gh(*issue_args) or f"failed to open an issue for {f['fp']}")  # noqa: T201
        created += 1

    print(f"{created} issue(s) {'would be ' if args.dry_run else ''}opened, {skipped} already tracked")  # noqa: T201
    return 0


if __name__ == "__main__":
    sys.exit(main())
