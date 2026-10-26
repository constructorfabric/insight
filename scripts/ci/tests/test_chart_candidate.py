from __future__ import annotations

import hashlib
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / ".github/workflows/scripts/verify-chart-candidate.sh"
SOURCE_SHA = "a" * 40
VERSION = "1.2.3"


class ChartCandidateTests(unittest.TestCase):
    def test_only_intact_artifacts_for_the_expected_candidate_pass(self) -> None:
        cases = ["intact", "modified archive", "wrong source", "wrong version", "missing checksum", "missing archive"]
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                files = {
                    "insight-1.2.3.tgz": b"synthetic chart bytes",
                    "source-sha": SOURCE_SHA.encode(),
                    "version": VERSION.encode(),
                    "release.patch": b"synthetic release patch",
                }
                for name, content in files.items():
                    (root / name).write_bytes(content)
                checksums = "\n".join(
                    f"{hashlib.sha256(content).hexdigest()}  {name}" for name, content in files.items()
                )
                if case == "missing checksum":
                    checksums = "\n".join(checksums.splitlines()[1:])
                (root / "SHA256SUMS").write_text(checksums + "\n")
                if case == "modified archive":
                    (root / "insight-1.2.3.tgz").write_bytes(b"different chart")
                if case == "missing archive":
                    (root / "insight-1.2.3.tgz").unlink()
                sha = "b" * 40 if case == "wrong source" else SOURCE_SHA
                version = "1.2.4" if case == "wrong version" else VERSION
                result = subprocess.run(
                    ["bash", str(SCRIPT), directory, sha, version],
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                self.assertEqual(result.returncode == 0, case == "intact", result.stderr)


if __name__ == "__main__":
    unittest.main()
