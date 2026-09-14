from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import logging_bar

CONFIGMAP_ON_THE_KNOBS = """
    logging:
      default:
        console_level: {{ ((((.Values.global).observability).logs).level)  | default "info" }}
        console_format: {{ ((((.Values.global).observability).logs).format) | default "json" }}
"""

API_MOD_ON_THE_BAR = """
mod handlers;

#[cfg(test)]
mod log_context_tests;
#[cfg(test)]
mod log_leak_tests;

pub fn register(router: Router) -> Router {
    router.layer(insight_log_context::LogContextLayer::new())
}
"""


CONTEXT_TEST_ON_THE_BAR = """
#[test]
fn request_lines_carry_the_identity() {
    let (_, log_ctx) = capture_probe_line(LogContextLayer::new(), &[], None)?;
}
"""

LEAK_TEST_ON_THE_BAR = """
#[test]
fn a_seeded_secret_never_renders() {
    let output = capture_output(|| tracing::error!(record = ?record, "probe"));
}
"""

TEST_CONTENT_ON_THE_BAR = {"log_context_tests.rs": CONTEXT_TEST_ON_THE_BAR, "log_leak_tests.rs": LEAK_TEST_ON_THE_BAR}


def write_service(root: Path, name: str, *, configmap: str, api_mod: str, tests: dict[str, str]) -> None:
    service = root / "src" / "backend" / "services" / name
    (service / "helm" / "templates").mkdir(parents=True)
    (service / "src" / "api").mkdir(parents=True)
    (service / "Cargo.toml").write_text("[package]\n", encoding="utf-8")
    (service / "helm" / "templates" / "configmap.yaml").write_text(configmap, encoding="utf-8")
    (service / "src" / "api" / "mod.rs").write_text(api_mod, encoding="utf-8")
    for test, content in tests.items():
        (service / "src" / "api" / test).write_text(content, encoding="utf-8")


class LoggingBarTests(unittest.TestCase):
    def judge(self, root: Path, name: str) -> logging_bar.Verdict:
        return logging_bar.judge(root / "src" / "backend" / "services" / name)

    def main_verdict(self, root: Path) -> int:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return logging_bar.main(root)

    def test_a_service_on_the_shared_knobs_with_both_test_suites_meets_the_bar(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(
                root,
                "exemplar",
                configmap=CONFIGMAP_ON_THE_KNOBS,
                api_mod=API_MOD_ON_THE_BAR,
                tests=TEST_CONTENT_ON_THE_BAR,
            )

            verdict = self.judge(root, "exemplar")

            self.assertTrue(verdict.meets_the_bar, verdict)

    def test_a_service_on_a_knob_of_its_own_is_below_the_bar(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(
                root,
                "loner",
                configmap='logging:\n  default:\n    console_level: "debug"\n    console_format: "text"\n',
                api_mod=API_MOD_ON_THE_BAR,
                tests=TEST_CONTENT_ON_THE_BAR,
            )

            verdict = self.judge(root, "loner")

            self.assertFalse(verdict.shape, verdict)
            self.assertFalse(verdict.level, verdict)
            self.assertFalse(verdict.meets_the_bar, verdict)

    def test_a_service_without_leak_tests_is_reported_not_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(
                root,
                "leaky",
                configmap=CONFIGMAP_ON_THE_KNOBS,
                api_mod=API_MOD_ON_THE_BAR.replace("#[cfg(test)]\nmod log_leak_tests;\n", ""),
                tests={"log_context_tests.rs": CONTEXT_TEST_ON_THE_BAR},
            )

            verdict = self.judge(root, "leaky")
            report = logging_bar.render_markdown([verdict])

            self.assertFalse(verdict.leaks, verdict)
            self.assertIn("`leaky`: missing leaks", report)
            self.assertIn("below the bar", report)

    def test_a_new_service_is_discovered_and_judged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(root, "newcomer", configmap="", api_mod="", tests={})
            (root / "src" / "backend" / "services" / "nginx-only").mkdir(parents=True)

            services = [s.name for s in logging_bar.rust_services(root)]

            self.assertEqual(services, ["newcomer"])
            self.assertFalse(self.judge(root, "newcomer").meets_the_bar)

    def test_noop_test_modules_do_not_pass_the_bar(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(
                root,
                "hollow",
                configmap=CONFIGMAP_ON_THE_KNOBS,
                api_mod=API_MOD_ON_THE_BAR,
                tests={"log_context_tests.rs": "#[test]\n", "log_leak_tests.rs": "#[test]\n"},
            )

            verdict = self.judge(root, "hollow")

            self.assertFalse(verdict.fields, verdict)
            self.assertFalse(verdict.leaks, verdict)

    def test_main_fails_when_a_service_is_below_the_bar(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(root, "below", configmap="", api_mod="", tests={})

            self.assertEqual(self.main_verdict(root), 1)

    def test_main_fails_when_no_services_exist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.main_verdict(Path(tmp)), 1)

    def test_main_passes_a_tree_on_the_bar(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_service(
                root,
                "exemplar",
                configmap=CONFIGMAP_ON_THE_KNOBS,
                api_mod=API_MOD_ON_THE_BAR,
                tests=TEST_CONTENT_ON_THE_BAR,
            )

            self.assertEqual(self.main_verdict(root), 0)

    def test_the_live_tree_meets_the_bar(self) -> None:
        verdicts = [logging_bar.judge(s) for s in logging_bar.rust_services(logging_bar.ROOT)]

        self.assertTrue(verdicts, "no services discovered in the live tree")
        for verdict in verdicts:
            self.assertTrue(verdict.meets_the_bar, verdict)


if __name__ == "__main__":
    unittest.main()
