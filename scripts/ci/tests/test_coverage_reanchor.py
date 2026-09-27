"""A Cobertura report must mean the same thing on whichever runner reads it.

pytest-cov and cargo-llvm-cov both write the producer's absolute workspace into
`<source>`. The shared self-hosted pool answers to one label set from two kinds
of runner, whose workspaces differ, so the consumer's checkout is routinely not
the producer's. These cover the repair and, as importantly, its refusals: a
source that cannot be proved against a real file is left exactly as it was.

Run: python3 -m unittest discover -s scripts/ci/tests -p 'test_*.py'
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CI_DIR = ROOT / "scripts" / "ci"
sys.path.insert(0, str(CI_DIR))

import coverage  # noqa: E402

ARC_WORKSPACE = "/home/runner/_work/insight/insight"
POOL_WORKSPACE = "/srv/gha/work/insight/insight"

#: Where the real producers put a module, and what they call it there.
PY_COMPONENT = "src/ingestion/scripts"
PY_FILENAME = "reconcile_bronze_schema.py"
RUST_COMPONENT = "src/backend"
RUST_FILENAME = "services/analytics/src/lib.rs"
JS_COMPONENT = "src/frontend"
JS_FILENAME = "src/router.ts"


def report(sources: list[str], filenames: list[str], hits: int = 1, lines: int = 2) -> str:
    """A Cobertura document carrying exactly the parts the gates read."""
    source_xml = "".join(f"<source>{s}</source>" for s in sources)
    measured = "".join(f'<line number="{n}" hits="{hits}"/>' for n in range(1, lines + 1))
    classes = "".join(
        f'<class name="c{i}" filename="{f}" line-rate="1.0"><lines>{measured}</lines></class>'
        for i, f in enumerate(filenames)
    )
    return (
        '<?xml version="1.0" ?>\n'
        '<coverage version="7.0" line-rate="1.0">\n'
        f"  <sources>{source_xml}</sources>\n"
        f'  <packages><package name="p" line-rate="1.0"><classes>{classes}</classes></package></packages>\n'
        "</coverage>\n"
    )


class ReanchorTests(unittest.TestCase):
    """`reanchor_source` against a checkout that really contains the files."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def touch(self, relative: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("x = 1\n", encoding="utf-8")
        return path

    def test_a_source_under_this_checkout_is_left_alone(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")
        source = str(self.root / PY_COMPONENT)

        self.assertIsNone(coverage.reanchor_source(source, [PY_FILENAME], self.root))

    def test_a_relative_source_is_left_alone(self) -> None:
        self.touch(f"{JS_COMPONENT}/{JS_FILENAME}")

        self.assertIsNone(coverage.reanchor_source(JS_COMPONENT, [JS_FILENAME], self.root))

    def test_a_pod_workspace_is_re_anchored_onto_a_pool_checkout(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")

        anchored = coverage.reanchor_source(f"{ARC_WORKSPACE}/{PY_COMPONENT}", [PY_FILENAME], self.root)

        self.assertEqual(anchored, PY_COMPONENT)

    def test_a_pool_workspace_is_re_anchored_onto_a_pod_checkout(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")

        anchored = coverage.reanchor_source(f"{POOL_WORKSPACE}/{PY_COMPONENT}", [PY_FILENAME], self.root)

        self.assertEqual(anchored, PY_COMPONENT)

    def test_a_nested_filename_resolves_the_rust_layout(self) -> None:
        self.touch(f"{RUST_COMPONENT}/{RUST_FILENAME}")

        anchored = coverage.reanchor_source(f"{ARC_WORKSPACE}/{RUST_COMPONENT}", [RUST_FILENAME], self.root)

        self.assertEqual(anchored, RUST_COMPONENT)

    def test_the_longest_proved_suffix_wins_over_a_shorter_one(self) -> None:
        """`scripts/` and `src/ingestion/scripts/` both exist, with one filename.

        The short suffix resolves to a real file too, so only preferring the
        longer one keeps the report in the tree it was measured from.
        """
        self.touch(f"scripts/{PY_FILENAME}")
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")

        anchored = coverage.reanchor_source(f"{ARC_WORKSPACE}/{PY_COMPONENT}", [PY_FILENAME], self.root)

        self.assertEqual(anchored, PY_COMPONENT)

    def test_a_source_matching_nothing_here_is_not_invented(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")

        anchored = coverage.reanchor_source("/somewhere/else/entirely", ["no_such_module.py"], self.root)

        self.assertIsNone(anchored)

    def test_a_directory_alone_is_not_proof(self) -> None:
        (self.root / PY_COMPONENT).mkdir(parents=True)

        anchored = coverage.reanchor_source(f"{ARC_WORKSPACE}/{PY_COMPONENT}", [PY_FILENAME], self.root)

        self.assertIsNone(anchored)

    def test_a_filename_cannot_climb_out_of_the_checkout(self) -> None:
        outside = self.root.parent / "outside.py"
        outside.write_text("x = 1\n", encoding="utf-8")
        self.addCleanup(outside.unlink)
        (self.root / PY_COMPONENT).mkdir(parents=True)

        anchored = coverage.reanchor_source(
            f"{ARC_WORKSPACE}/{PY_COMPONENT}", [f"../../../../{outside.name}"], self.root
        )

        self.assertIsNone(anchored)


class SanitizeReportTests(unittest.TestCase):
    """`sanitize_report` over whole documents, in each producer's real shape."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def touch(self, relative: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("x = 1\n", encoding="utf-8")

    def write(self, name: str, body: str) -> Path:
        path = self.root / name
        path.write_text(body, encoding="utf-8")
        return path

    def test_the_python_producer_format_is_repaired(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")
        path = self.write("python.xml", report([f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME]))

        text = coverage.sanitize_report(path, self.root)

        self.assertIsNotNone(text)
        self.assertIn(f"<source>{PY_COMPONENT}</source>", text)
        self.assertNotIn(ARC_WORKSPACE, text)

    def test_the_rust_producer_format_is_repaired(self) -> None:
        self.touch(f"{RUST_COMPONENT}/{RUST_FILENAME}")
        path = self.write("rust.xml", report([f"{ARC_WORKSPACE}/{RUST_COMPONENT}"], [RUST_FILENAME]))

        text = coverage.sanitize_report(path, self.root)

        self.assertIsNotNone(text)
        self.assertIn(f"<source>{RUST_COMPONENT}</source>", text)

    def test_the_js_producer_format_is_already_portable(self) -> None:
        self.touch(f"{JS_COMPONENT}/{JS_FILENAME}")
        path = self.write("js.xml", report([JS_COMPONENT], [JS_FILENAME]))

        self.assertIsNone(coverage.sanitize_report(path, self.root))

    def test_only_the_provable_source_of_several_is_rewritten(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")
        path = self.write("multi.xml", report(["/nowhere/at/all", f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME]))

        text = coverage.sanitize_report(path, self.root)

        self.assertIn(f"<source>{PY_COMPONENT}</source>", text)
        self.assertIn("<source>/nowhere/at/all</source>", text)

    def test_a_report_that_cannot_be_proved_is_returned_unchanged(self) -> None:
        path = self.write("orphan.xml", report([f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME]))

        self.assertIsNone(coverage.sanitize_report(path, self.root))

    def test_originals_are_copied_not_rewritten(self) -> None:
        self.touch(f"{PY_COMPONENT}/{PY_FILENAME}")
        reports = self.root / "coverage"
        reports.mkdir()
        original = report([f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME])
        (reports / "python.xml").write_text(original, encoding="utf-8")
        dest = self.root / "sanitized"
        dest.mkdir()

        used = coverage.sanitized_reports_dir(reports, self.root, dest)

        self.assertEqual(used, dest)
        self.assertEqual((reports / "python.xml").read_text(encoding="utf-8"), original)
        self.assertIn(f"<source>{PY_COMPONENT}</source>", (dest / "python.xml").read_text(encoding="utf-8"))

    def test_nothing_to_repair_leaves_the_gates_on_the_originals(self) -> None:
        self.touch(f"{JS_COMPONENT}/{JS_FILENAME}")
        reports = self.root / "coverage"
        reports.mkdir()
        (reports / "js.xml").write_text(report([JS_COMPONENT], [JS_FILENAME]), encoding="utf-8")
        dest = self.root / "sanitized"
        dest.mkdir()

        self.assertEqual(coverage.sanitized_reports_dir(reports, self.root, dest), reports)


class ParseAfterSanitationTests(unittest.TestCase):
    """What the component gate ends up with once the report has been repaired."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        path = self.root / PY_COMPONENT / PY_FILENAME
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("x = 1\n", encoding="utf-8")

    def test_a_foreign_report_buckets_to_its_component_after_sanitation(self) -> None:
        reports = self.root / "coverage"
        reports.mkdir()
        (reports / "python.xml").write_text(
            report([f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME]), encoding="utf-8"
        )
        dest = self.root / "sanitized"
        dest.mkdir()

        before = coverage.load_all_reports(reports, self.root)
        after = coverage.load_all_reports(coverage.sanitized_reports_dir(reports, self.root, dest), self.root)

        self.assertEqual(list(before), [PY_FILENAME], "the unrepaired path is the regression")
        self.assertEqual(list(after), [f"{PY_COMPONENT}/{PY_FILENAME}"])

    def test_a_same_root_report_is_unaffected_by_sanitation(self) -> None:
        reports = self.root / "coverage"
        reports.mkdir()
        (reports / "python.xml").write_text(report([str(self.root / PY_COMPONENT)], [PY_FILENAME]), encoding="utf-8")
        dest = self.root / "sanitized"
        dest.mkdir()

        files = coverage.load_all_reports(coverage.sanitized_reports_dir(reports, self.root, dest), self.root)

        self.assertEqual(list(files), [f"{PY_COMPONENT}/{PY_FILENAME}"])


class DiffCoverTests(unittest.TestCase):
    """The third-party patch gate reads the XML itself, so it needs the repair too."""

    def setUp(self) -> None:
        if not coverage.which("diff-cover") or not coverage.which("git"):
            self.skipTest("the new-code gate needs diff-cover and git")
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        target = self.root / PY_COMPONENT / PY_FILENAME
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("a = 1\nb = 2\n", encoding="utf-8")
        # `-b master` so the compare branch below does not depend on whatever
        # init.defaultBranch the host happens to set.
        self.git("init", "-q", "-b", "master", ".")
        self.git("config", "user.email", "ci@example.com")
        self.git("config", "user.name", "ci")
        self.git("add", "-A")
        self.git("commit", "-qm", "base")
        self.git("checkout", "-q", "-b", "work")
        target.write_text("a = 1\nb = 2\nc = 3\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "change")

    def git(self, *args: str) -> None:
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def diff_cover(self, report_path: Path) -> str:
        proc = subprocess.run(
            ["diff-cover", str(report_path), "--compare-branch", "master", "--fail-under", "0"],
            cwd=self.root,
            capture_output=True,
            text=True,
        )
        return proc.stdout

    def test_a_foreign_report_measures_nothing_until_it_is_sanitized(self) -> None:
        reports = self.root / "coverage"
        reports.mkdir()
        # The report must declare the line the diff adds, or diff-cover has
        # nothing to say whatever the source path resolves to.
        body = report([f"{ARC_WORKSPACE}/{PY_COMPONENT}"], [PY_FILENAME], hits=1, lines=3)
        (reports / "python.xml").write_text(body, encoding="utf-8")
        dest = self.root / "sanitized"
        dest.mkdir()

        before = self.diff_cover(reports / "python.xml")
        used = coverage.sanitized_reports_dir(reports, self.root, dest)
        after = self.diff_cover(used / "python.xml")

        self.assertIn("No lines with coverage information", before)
        self.assertIn(f"{PY_COMPONENT}/{PY_FILENAME}", after)


if __name__ == "__main__":
    unittest.main()
