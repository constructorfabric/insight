import os
import sys
from dataclasses import dataclass
from pathlib import Path

MAX_FAILURES = 10
MAX_NODE_LENGTH = 240
MAX_MESSAGE_LENGTH = 200


@dataclass(frozen=True)
class Lane:
    suite_stage: str
    stages: tuple[tuple[str, str], ...]
    artifacts: tuple[str, ...]


LANES = {
    "api-smoke": Lane(
        suite_stage="API_TESTS",
        stages=(
            ("STAND", "Bring the stand up"),
            ("API_TESTS", "Run the API suite"),
            ("API_COVERAGE", "API coverage gate"),
            ("API_COVERAGE_UPLOAD", "Upload the coverage ledger"),
            ("TEARDOWN", "Tear the stand down"),
        ),
        artifacts=("api-smoke-compose-logs", "api-smoke-coverage"),
    ),
    "ui-journeys": Lane(
        suite_stage="UI_TESTS",
        stages=(
            ("STAND", "Bring the stand up"),
            ("BROWSER_INSTALL", "Install the suite's browser"),
            ("UI_TESTS", "Run the browser journeys"),
            ("REDACTION", "Strip credentials from traces"),
            ("UI_TRACES_UPLOAD", "Upload traces and screenshots"),
            ("TEARDOWN", "Tear the stand down"),
        ),
        artifacts=("ui-journeys-traces", "ui-journeys-video", "ui-journeys-compose-logs"),
    ),
}


def clipped(value: str, limit: int) -> str:
    first_line = value.splitlines()[0] if value.splitlines() else ""
    printable = "".join(char for char in first_line if char.isprintable())
    return printable if len(printable) <= limit else printable[: limit - 1] + "…"


def node_id(attributes: dict[str, str]) -> str:
    file = attributes.get("file", "")
    classname = attributes.get("classname", "")
    name = attributes.get("name", "<unknown test>")
    if not file:
        return "::".join(part for part in (classname, name) if part)

    module = str(Path(file).with_suffix("")).replace("/", ".")
    classes = classname.removeprefix(module).lstrip(".")
    if file.startswith("stand/"):
        file = f"tests/{file}"

    return "::".join(part for part in (file, classes.replace(".", "::"), name) if part)


def failing_tests(path: Path) -> tuple[list[tuple[str, str]], int] | None:
    try:
        from defusedxml.ElementTree import ParseError, parse
    except ImportError:
        return None

    try:
        root = parse(path).getroot()
        if root.tag not in {"testsuites", "testsuite"}:
            return None

        failures: list[tuple[str, str]] = []
        count = 0
        for case in root.iter("testcase"):
            for result in case:
                if result.tag not in {"failure", "error"}:
                    continue
                count += 1
                if len(failures) < MAX_FAILURES:
                    message = result.get("message", f"{result.tag}: no message attribute")
                    failures.append((node_id(case.attrib), message))
        return failures, count
    except (OSError, ParseError, ValueError, LookupError):
        return None


def report(lane_name: str, path: Path, job_url: str) -> str:
    lane = LANES[lane_name]
    unsuccessful: list[str] = []
    for variable, display_name in lane.stages:
        outcome = os.environ.get(variable, "unknown")
        if outcome not in {"success", "failure", "skipped", "cancelled"}:
            outcome = "unknown"
        if outcome != "success":
            unsuccessful.append(f"{display_name} ({outcome})")

    status = "stages not successful: " + "; ".join(unsuccessful) if unsuccessful else "all stages succeeded"
    lines = [f"{lane_name}: {status}", "", f"[Stand runner job log]({job_url})", ""]
    tests = failing_tests(path)
    if os.environ.get(lane.suite_stage) == "failure" and (tests is None or tests[1] == 0):
        suite_name = dict(lane.stages)[lane.suite_stage]
        lines.append(
            f"{suite_name} failed before or outside any test (collection, setup or a crash) — read that step's log."
        )
    elif tests is None:
        lines.append("The JUnit report could not be read.")
    else:
        failures, count = tests
        lines.append(f"Failing tests ({count}):" if count else "No failing tests in the JUnit report.")
        for node, message in failures:
            node = clipped(node, MAX_NODE_LENGTH).replace("`", "'")
            message = clipped(message, MAX_MESSAGE_LENGTH).replace("`", "'")
            lines.append(f"- `{node}` — `{message}`")
        if count > len(failures):
            lines.append(f"- and {count - len(failures)} more")

    artifacts = ", ".join(f"`{name}`" for name in lane.artifacts)
    lines.extend(["", f"Artifacts (if present): {artifacts}", ""])
    return "\n".join(lines)


def main(argv: list[str]) -> None:
    try:
        lane_name, junit_path, job_url = argv[1:]
        sys.stdout.write(report(lane_name, Path(junit_path), job_url) + "\n")
    except (ValueError, KeyError, OSError):
        sys.stdout.write("Stand lane report unavailable; the suite left no readable JUnit report.\n")


if __name__ == "__main__":
    main(sys.argv)
