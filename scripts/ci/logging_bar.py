#!/usr/bin/env python3
"""The per-service logging bar (insight#2488 AC-6).

One check that reports, for every Rust service a default install runs, whether
it meets the logging bar this release agreed:

- shape        - the service's configmap renders ``console_format`` from the
                 install-wide ``global.observability.logs.format`` knob, so its
                 lines follow the one agreed JSON shape (AC-1).
- level        - ``console_level`` renders from ``global.observability.logs.level``,
                 the one place an operator sets the level (AC-2).
- fields       - the service mounts ``LogContextLayer`` (correlation id, tenant,
                 service, version on request lines) and carries the capture test
                 pinning the field set (AC-3).
- leaks        - the service carries seeded-leak tests proving no token, session
                 credential or personal data reaches a line (AC-4).

Services are discovered, not listed: every ``src/backend/services/*/Cargo.toml``
is held to the bar, so a new service that misses it is reported, not skipped.
Pure file inspection - no cargo, no helm, stdlib only. Prints a markdown verdict
table; exits non-zero when any service misses the bar.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent.absolute()
SERVICES_DIR = Path("src/backend/services")

FORMAT_KNOB = re.compile(r"console_format:.*observability\)\.logs\)\.format")
LEVEL_KNOB = re.compile(r"console_level:.*observability\)\.logs\)\.level")

CRITERIA = ("shape", "level", "fields", "leaks")


@dataclass(frozen=True)
class Verdict:
    service: str
    shape: bool
    level: bool
    fields: bool
    leaks: bool

    @property
    def meets_the_bar(self) -> bool:
        return self.shape and self.level and self.fields and self.leaks


def rust_services(root: Path) -> list[Path]:
    services = root / SERVICES_DIR
    if not services.is_dir():
        return []
    return sorted(entry for entry in services.iterdir() if entry.is_dir() and (entry / "Cargo.toml").is_file())


def read_or_empty(path: Path) -> str:
    return path.read_text(encoding="utf-8") if path.is_file() else ""


def has_capture_test(path: Path, capture_symbols: tuple[str, ...]) -> bool:
    """A declared test module must actually exercise the capture path: at least
    one #[test] plus a call to a line-capture helper. Presence alone would let
    a no-op module report compliance."""
    content = read_or_empty(path)
    return "#[test]" in content and any(f"{symbol}(" in content for symbol in capture_symbols)


def judge(service: Path) -> Verdict:
    configmap = read_or_empty(service / "helm" / "templates" / "configmap.yaml")
    api_mod = read_or_empty(service / "src" / "api" / "mod.rs")

    context_test = service / "src" / "api" / "log_context_tests.rs"
    leak_test = service / "src" / "api" / "log_leak_tests.rs"

    return Verdict(
        service=service.name,
        shape=bool(FORMAT_KNOB.search(configmap)),
        level=bool(LEVEL_KNOB.search(configmap)),
        fields=(
            "LogContextLayer" in api_mod
            and "mod log_context_tests;" in api_mod
            and has_capture_test(context_test, ("capture_probe_line",))
        ),
        leaks=(
            "mod log_leak_tests;" in api_mod and has_capture_test(leak_test, ("capture_output", "capture_probe_output"))
        ),
    )


def render_markdown(verdicts: list[Verdict]) -> str:
    def mark(ok: bool) -> str:
        return "yes" if ok else "**MISSING**"

    lines = [
        "## Logging bar - per-service verdict",
        "",
        "| service | shape | level | fields | leaks | verdict |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for v in verdicts:
        verdict = "meets the bar" if v.meets_the_bar else "**below the bar**"
        lines.append(
            f"| {v.service} | {mark(v.shape)} | {mark(v.level)} | {mark(v.fields)} | {mark(v.leaks)} | {verdict} |"
        )

    misses = [v for v in verdicts if not v.meets_the_bar]
    if misses:
        lines += ["", "### Below the bar (blocking)", ""]
        for v in misses:
            gaps = ", ".join(c for c in CRITERIA if not getattr(v, c))
            lines.append(f"- `{v.service}`: missing {gaps}")
    return "\n".join(lines)


def main(root: Path = ROOT) -> int:
    verdicts = [judge(service) for service in rust_services(root)]
    if not verdicts:
        print("no Rust services found under", SERVICES_DIR, file=sys.stderr)  # noqa: T201
        return 1

    print(render_markdown(verdicts))  # noqa: T201
    return 0 if all(v.meets_the_bar for v in verdicts) else 1


if __name__ == "__main__":
    sys.exit(main())
