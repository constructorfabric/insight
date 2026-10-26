from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "candidate_inputs.py"
SPEC = importlib.util.spec_from_file_location("candidate_inputs", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
candidate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(candidate)
SHA = "a" * 40
DIGEST = "b" * 64


def write_candidate(root: Path) -> None:
    values = {
        alias: {"image": {"repository": f"example.invalid/{alias.lower()}", "tag": f"v1@sha256:{DIGEST}"}}
        for alias in candidate.COMPOSE_IMAGES
    }
    files = {
        "source-sha": SHA,
        "version": "1.2.3",
        "release.patch": "synthetic patch",
        "values.json": json.dumps(values),
    }
    for name, content in files.items():
        (root / name).write_text(content)
    write_checksums(root)


def write_checksums(root: Path) -> None:
    (root / "SHA256SUMS").write_text(
        "".join(
            f"{hashlib.sha256((root / name).read_bytes()).hexdigest()}  {name}\n" for name in sorted(candidate.FILES)
        )
    )


class CandidateInputsTests(unittest.TestCase):
    def test_all_compose_services_use_the_verified_image_set(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_candidate(root)
            images = candidate.compose_images(candidate.verify(root, SHA))
            self.assertEqual(set(images), set(candidate.COMPOSE_IMAGES.values()))
            self.assertTrue(all(image.endswith(f"@sha256:{DIGEST}") for image in images.values()))

    def test_corrupt_or_incomplete_candidate_cannot_start_tests(self) -> None:
        for case in ("wrong sha", "changed values", "missing checksum", "duplicate checksum", "path escape"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                write_candidate(root)
                checksums = root / "SHA256SUMS"
                if case == "changed values":
                    (root / "values.json").write_text("{}")
                if case == "missing checksum":
                    checksums.write_text("\n".join(checksums.read_text().splitlines()[1:]))
                if case == "duplicate checksum":
                    checksums.write_text(checksums.read_text() * 2)
                if case == "path escape":
                    checksums.write_text(f"{'0' * 64}  ../source-sha\n")
                with self.assertRaises(ValueError):
                    candidate.verify(root, "c" * 40 if case == "wrong sha" else SHA)

    def test_missing_mutable_or_injected_images_cannot_enter_the_environment(self) -> None:
        for tag in ("latest", f"v1@sha256:{DIGEST}\nOTHER=value", f"$(command)@sha256:{DIGEST}", ""):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                write_candidate(root)
                values = json.loads((root / "values.json").read_text())
                values["analytics"]["image"]["tag"] = tag
                (root / "values.json").write_text(json.dumps(values))
                write_checksums(root)
                with self.assertRaises(ValueError):
                    candidate.verify(root, SHA)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_candidate(root)
            values = json.loads((root / "values.json").read_text())
            del values["gateway"]
            (root / "values.json").write_text(json.dumps(values))
            write_checksums(root)
            with self.assertRaises(KeyError):
                candidate.verify(root, SHA)


if __name__ == "__main__":
    unittest.main()
