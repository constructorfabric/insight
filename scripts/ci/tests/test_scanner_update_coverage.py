"""Dependabot refreshes the Semgrep hash lock, and no other scanner pin.

The pip job on ``/.github`` rewrites each ``==`` pin and that pin's
``--hash=sha256`` lines. Trivy and TruffleHog stay version-plus-digest
assignments in the composite installer. No ecosystem reads those.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GITHUB = ROOT / ".github"
DEPENDABOT = GITHUB / "dependabot.yml"
LOCK = GITHUB / "semgrep-requirements.txt"
ACTION = GITHUB / "actions" / "install-scanners" / "action.yml"
SECURITY = GITHUB / "workflows" / "security.yml"
PYTHON_VERSION = GITHUB / ".python-version"

# INVARIANT: Dependabot's Python file fetcher drops a requirements file above
# this size (SharedFileFetcher::MAX_FILE_SIZE). A larger lock is skipped silently.
MAX_REQUIREMENTS_BYTES = 500_000

_PIN = re.compile(
    r"(?m)^(?P<name>[A-Za-z0-9][A-Za-z0-9._-]*)==(?P<version>[^\s\\]+) \\\n"
    r"(?P<body>(?:    --hash=sha256:[0-9a-f]{64} \\\n)*"
    r"    --hash=sha256:[0-9a-f]{64})\n"
)
_DECLARED = re.compile(r"(?m)^([A-Za-z0-9][A-Za-z0-9._-]*)==")
_ENTRY = re.compile(r"(?m)^  - package-ecosystem:")


def hashed_pins(text: str) -> list[tuple[str, str, tuple[str, ...]]]:
    pins: list[tuple[str, str, tuple[str, ...]]] = []
    for match in _PIN.finditer(text):
        digests = tuple(re.findall(r"--hash=sha256:([0-9a-f]{64})", match.group("body")))
        if not digests:
            raise AssertionError(f"{match.group('name')} has no sha256 digest")
        pins.append((match.group("name"), match.group("version"), digests))

    declared = _DECLARED.findall(text)
    found = [name for name, _, _ in pins]
    if declared != found:
        raise AssertionError(f"pins without a sha256 block: {declared} vs {found}")
    return pins


def rewrite_hashed_pin(
    text: str,
    name: str,
    new_version: str,
    new_digests: tuple[str, ...],
) -> str:
    """The edit Dependabot's pip replacer makes: the ``==`` version and that pin's digests."""
    if not new_digests or any(re.fullmatch(r"[0-9a-f]{64}", digest) is None for digest in new_digests):
        raise ValueError("a hashed pin keeps at least one sha256 digest")

    match = next((found for found in _PIN.finditer(text) if found.group("name") == name), None)
    if match is None:
        raise ValueError(f"missing pin: {name}")

    lines = [f"{name}=={new_version} \\"]
    for index, digest in enumerate(new_digests):
        slash = " \\" if index < len(new_digests) - 1 else ""
        lines.append(f"    --hash=sha256:{digest}{slash}")
    replacement = "\n".join(lines) + "\n"
    return text[: match.start()] + replacement + text[match.end() :]


def dependabot_blocks(text: str) -> list[str]:
    starts = [match.start() for match in _ENTRY.finditer(text)]
    blocks: list[str] = []
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(text)
        blocks.append(text[start:end])
    return blocks


def field(block: str, key: str) -> str:
    if key == "package-ecosystem":
        match = re.match(r"  - package-ecosystem:\s*(\S+)", block)
    else:
        match = re.search(rf"(?m)^    {re.escape(key)}:[ \t]*(.*)$", block)
    if match is None:
        raise AssertionError(f"missing {key}")
    return match.group(1).strip().strip('"').strip("'")


def list_field(block: str, key: str) -> list[str]:
    inline = re.search(rf"(?m)^    {re.escape(key)}:[ \t]*(.*)$", block)
    if inline and inline.group(1).strip():
        value = inline.group(1).strip()
        if value.startswith("[") and value.endswith("]"):
            return [part.strip().strip('"').strip("'") for part in value[1:-1].split(",") if part.strip()]
        return [value.strip('"').strip("'")]

    items: list[str] = []
    seen = False
    for line in block.splitlines():
        if re.match(rf"^    {re.escape(key)}:\s*$", line):
            seen = True
            continue
        if not seen:
            continue
        item = re.match(r'^      - "([^"]+)"\s*$', line) or re.match(r"^      - (\S+)\s*$", line)
        if item:
            items.append(item.group(1))
            continue
        if line.startswith("    ") and not line.startswith("      ") and line.strip():
            if line.lstrip().startswith("#"):
                continue
            break
    return items


def entry_directories(block: str) -> list[str]:
    if re.search(r"(?m)^    directories:", block):
        return list_field(block, "directories")
    return [field(block, "directory")]


class ScannerUpdateCoverageTests(unittest.TestCase):
    def test_semgrep_hash_lock_is_the_only_manifest_the_pip_job_scans(self) -> None:
        blocks = dependabot_blocks(DEPENDABOT.read_text(encoding="utf-8"))
        pip = [
            block
            for block in blocks
            if field(block, "package-ecosystem") == "pip" and entry_directories(block) == ["/.github"]
        ]
        self.assertEqual(len(pip), 1, "the Semgrep lock needs one pip entry on /.github")
        block = pip[0]

        self.assertRegex(block, r"(?m)^      interval: weekly$")
        self.assertRegex(block, r"(?m)^      day: monday$")
        self.assertRegex(block, r"(?m)^      default-days: 7$")
        self.assertEqual(field(block, "versioning-strategy"), "increase")
        self.assertNotIn("lockfile-only", block)
        self.assertEqual(list_field(block, "exclude-paths"), ["trufflehog-allowlist.txt"])
        self.assertIn('        patterns: ["*"]', block.splitlines())

        names = sorted(
            path.relative_to(GITHUB).as_posix()
            for path in GITHUB.rglob("*")
            if path.is_file() and path.suffix in {".txt", ".in"}
        )
        self.assertEqual(
            names,
            ["semgrep-requirements.txt", "trufflehog-allowlist.txt"],
            "a new .txt or .in under .github is inside this pip scan",
        )
        self.assertEqual(list(GITHUB.rglob("*.in")), [])
        self.assertLess(LOCK.stat().st_size, MAX_REQUIREMENTS_BYTES)
        self.assertEqual(PYTHON_VERSION.read_text(encoding="utf-8").strip(), "3.12")

        security = SECURITY.read_text(encoding="utf-8")
        self.assertEqual(security.count("--require-hashes -r .github/semgrep-requirements.txt"), 2)
        self.assertEqual(set(re.findall(r"uv venv --python (\S+)", security)), {"3.12"})

        pins = hashed_pins(LOCK.read_text(encoding="utf-8"))
        semgrep = next(pin for pin in pins if pin[0] == "semgrep")
        self.assertRegex(semgrep[1], r"^\d+\.\d+\.\d+$")
        self.assertGreaterEqual(len(semgrep[2]), 1)

    def test_a_version_bump_replaces_that_pins_digests(self) -> None:
        original = LOCK.read_text(encoding="utf-8")
        before = {name: (version, digests) for name, version, digests in hashed_pins(original)}
        old_version, old_digests = before["bracex"]
        new_digests = ("ab" * 32, "cd" * 32)

        rewritten = rewrite_hashed_pin(original, "bracex", "9.9.9", new_digests)
        after = {name: (version, digests) for name, version, digests in hashed_pins(rewritten)}

        self.assertEqual(after["bracex"], ("9.9.9", new_digests))
        self.assertNotIn(old_version, after["bracex"])
        self.assertFalse(set(old_digests) & set(after["bracex"][1]))
        untouched = {name: pin for name, pin in before.items() if name != "bracex"}
        self.assertEqual({name: after[name] for name in untouched}, untouched)
        with self.assertRaises(ValueError):
            rewrite_hashed_pin(original, "bracex", "9.9.9", ())

    def test_a_pin_missing_its_digest_is_rejected(self) -> None:
        original = LOCK.read_text(encoding="utf-8")
        stripped, count = re.subn(
            r"(?m)^bracex==[^\n]* \\\n(?:    --hash=sha256:[0-9a-f]{64} ?\\\n)*    --hash=sha256:[0-9a-f]{64}\n",
            "bracex==3.0.1\n",
            original,
            count=1,
        )
        self.assertEqual(count, 1)
        with self.assertRaises(AssertionError):
            hashed_pins(stripped)

    def test_trivy_and_trufflehog_remain_checksum_pinned_binaries(self) -> None:
        text = ACTION.read_text(encoding="utf-8")
        self.assertIn("using: composite", text)
        self.assertNotIn("uses:", text)
        self.assertNotIn("setup-trivy", text)
        self.assertNotIn("trivy-action", text)
        self.assertNotIn("docker://", text)
        self.assertIn("sha256sum -c -", text)

        pins = dict(
            re.findall(
                r"(?m)^        (TRIVY_VERSION|TRIVY_SHA256|TRUFFLEHOG_VERSION|TRUFFLEHOG_SHA256): (\S+)$",
                text,
            )
        )
        self.assertEqual(set(pins), {"TRIVY_VERSION", "TRIVY_SHA256", "TRUFFLEHOG_VERSION", "TRUFFLEHOG_SHA256"})
        self.assertRegex(pins["TRIVY_VERSION"], r"^\d+\.\d+\.\d+$")
        self.assertRegex(pins["TRUFFLEHOG_VERSION"], r"^\d+\.\d+\.\d+$")
        self.assertRegex(pins["TRIVY_SHA256"], r"^[0-9a-f]{64}$")
        self.assertRegex(pins["TRUFFLEHOG_SHA256"], r"^[0-9a-f]{64}$")
        self.assertIn("v${TRIVY_VERSION}/trivy_${TRIVY_VERSION}_Linux-64bit.tar.gz", text)
        self.assertIn("v${TRUFFLEHOG_VERSION}/trufflehog_${TRUFFLEHOG_VERSION}_linux_amd64.tar.gz", text)
        self.assertIn('"$TRIVY_SHA256"', text)
        self.assertIn('"$TRUFFLEHOG_SHA256"', text)

    def test_those_digest_pairs_are_not_a_dependabot_manifest(self) -> None:
        blocks = dependabot_blocks(DEPENDABOT.read_text(encoding="utf-8"))
        ecosystems = [(field(block, "package-ecosystem"), entry_directories(block)) for block in blocks]

        actions = [directories for ecosystem, directories in ecosystems if ecosystem == "github-actions"]
        self.assertEqual(actions, [["/"]])
        docker = [directories for ecosystem, directories in ecosystems if ecosystem == "docker"]
        self.assertEqual(docker, [["/src/**", "/deploy/**"]])

        covering_github = [
            (ecosystem, directories)
            for ecosystem, directories in ecosystems
            if any(directory == "/.github" or directory.startswith("/.github/") for directory in directories)
        ]
        self.assertEqual(covering_github, [("pip", ["/.github"])])


if __name__ == "__main__":
    unittest.main()
