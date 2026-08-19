from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "scripts" / "ci" / "changed.py"


class ChangedCliTests(unittest.TestCase):
    def test_compare_ref_selects_the_diff_base(self) -> None:
        result = subprocess.run(
            ["python3", str(SCRIPT), "--compare-ref", "HEAD"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"rust": [], "python": [], "js": []})


if __name__ == "__main__":
    unittest.main()
