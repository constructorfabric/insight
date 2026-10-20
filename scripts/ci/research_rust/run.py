from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import re
import shutil
import subprocess
import time
from pathlib import Path
from statistics import median

from defusedxml import ElementTree as ET

VARIANTS = ("baseline", "clippy-flags", "compiler-cache")
PACKAGE = "insight-clickhouse"
LOG = logging.getLogger(__name__)


def environment(variant: str, output: Path) -> dict[str, str]:
    env = os.environ.copy()
    for name in ("RUSTFLAGS", "CARGO_BUILD_WARNINGS", "RUSTC_WRAPPER", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(name, None)
    env.update(
        CARGO_INCREMENTAL="0",
        CARGO_BUILD_JOBS="2",
        CARGO_TERM_COLOR="never",
        CARGO_TARGET_DIR=str(output / variant / "target"),
        SCCACHE_DIR=str(output / "compiler-cache" / "cache"),
        SCCACHE_GHA_ENABLED="false",
        SCCACHE_CACHE_SIZE="3G",
    )
    if variant == "compiler-cache":
        env["RUSTC_WRAPPER"] = "sccache"
    return env


def run(command: list[str], cwd: Path, env: dict[str, str], log: Path) -> dict[str, object]:
    started = time.monotonic()
    build_seconds = None
    outcomes = []
    with log.open("w") as stream:
        process = subprocess.Popen(
            command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
        )
        assert process.stdout is not None
        for line in process.stdout:
            stream.write(line)
            stream.flush()
            if re.search(r"Finished .*target\(s\)", line):
                build_seconds = time.monotonic() - started
            match = re.match(r"test (.+) \.\.\. (ok|ignored|FAILED)", line)
            if match:
                outcomes.append(match.groups())
        code = process.wait()
    elapsed = time.monotonic() - started
    result = {"command": command, "seconds": elapsed, "exit_code": code, "outcomes": sorted(outcomes)}
    if build_seconds is not None:
        result["build_seconds"] = build_seconds
        result["after_build_seconds"] = elapsed - build_seconds
    if code:
        raise RuntimeError(f"command failed ({code}): {command}; see {log}")
    return result


def coverage(path: Path, source: Path) -> list[tuple[str, int, int]]:
    result = []
    for item in ET.parse(path).findall(".//class"):
        name = str(Path(item.attrib["filename"]).as_posix()).replace(str(source) + "/", "")
        for line in item.findall("./lines/line"):
            result.append((name, int(line.attrib["number"]), int(line.attrib["hits"]) > 0))
    if not result:
        raise RuntimeError("coverage report has no measured lines")
    return sorted(set(result))


def fixture(contents: str, revision: int) -> str:
    return contents + (
        "\n#[cfg(test)]\nmod ci_research {\n"
        "    #[test]\n    fn synthetic_edit_keeps_the_same_contract() {\n"
        f"        assert!(std::hint::black_box({revision}_u8) > 0);\n"
        "    }\n}\n"
    )


def scenarios(repeats: int) -> list[tuple[str, int, int]]:
    return (
        [("cold", 1, 1)]
        + [("warm", n, 1) for n in range(1, repeats + 1)]
        + [("edit", n, n + 1) for n in range(1, repeats + 1)]
    )


def sample(source: Path, output: Path, variant: str, phase: str, repeat: int) -> dict[str, object]:
    folder = output / variant / f"{phase}-{repeat}"
    folder.mkdir(parents=True)
    env = environment(variant, output)
    workspace = source / "src/backend"
    if variant == "compiler-cache":
        subprocess.run(["sccache", "--zero-stats"], env=env, check=True, stdout=subprocess.DEVNULL)
    lint_env = env.copy()
    if variant == "clippy-flags":
        lint_env["CARGO_BUILD_WARNINGS"] = "deny"
    else:
        lint_env["RUSTFLAGS"] = "-D warnings"
    lint = run(
        ["cargo", "clippy", "--locked", "-p", PACKAGE, "--all-targets", "--all-features"],
        workspace,
        lint_env,
        folder / "clippy.log",
    )
    target = Path(env["CARGO_TARGET_DIR"])
    for profile in target.rglob("*.profraw"):
        profile.unlink()
    tests = run(
        ["cargo", "llvm-cov", "--locked", "--no-report", "--package", PACKAGE, "--all-features"],
        workspace,
        env,
        folder / "tests.log",
    )
    report = run(
        [
            "cargo",
            "llvm-cov",
            "report",
            "--package",
            PACKAGE,
            "--cobertura",
            "--output-path",
            str(folder / "coverage.xml"),
        ],
        workspace,
        env,
        folder / "coverage.log",
    )
    if not tests["outcomes"]:
        raise RuntimeError("test execution was not observed")
    result = {
        "variant": variant,
        "phase": phase,
        "repeat": repeat,
        "clippy": lint,
        "tests": tests,
        "report": report,
        "total_seconds": sum(float(step["seconds"]) for step in (lint, tests, report)),
        "coverage": coverage(folder / "coverage.xml", source),
    }
    if variant == "compiler-cache":
        stats = subprocess.check_output(["sccache", "--show-stats", "--stats-format=json"], env=env, text=True)
        result["cache"] = json.loads(stats)
    return result


def summary(document: dict[str, object]) -> str:
    rows = document["samples"]
    lines = [
        "# Isolated Rust CI research",
        "",
        "Real crate; one worker; two compiler jobs; one parameter changes per candidate.",
        "",
        "| Phase | Variant | Clippy s | Coverage build s | After build s | Report s | Total s |",
        "|---|---|---:|---:|---:|---:|---:|",
    ]
    for phase in ("cold", "warm", "edit", "cache-reuse"):
        for variant in VARIANTS:
            group = [r for r in rows if r["phase"] == phase and r["variant"] == variant]
            if not group:
                continue
            values = [
                median(r["clippy"]["seconds"] for r in group),
                median(r["tests"].get("build_seconds", 0) for r in group),
                median(r["tests"].get("after_build_seconds", 0) for r in group),
                median(r["report"]["seconds"] for r in group),
                median(r["total_seconds"] for r in group),
            ]
            lines.append(f"| {phase} | {variant} | " + " | ".join(f"{v:.3f}" for v in values) + " |")
    lines += [
        "",
        "Cold is one exploratory sample; the other phases use three repetitions. After-build time includes test execution and Cargo overhead.",
        "Compiler-cache uses an isolated local disk cache. Remote cache restore/save costs and whole-workflow gains are not measured.",
        "Warm retains Cargo artifacts; edit changes one synthetic test body. No application change is committed.",
        "Cache-reuse deletes Cargo build artifacts before each sample but retains the compiler cache.",
    ]
    if document.get("error"):
        lines += ["", "FAILED: " + str(document["error"])]
    return "\n".join(lines) + "\n"


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, choices=range(1, 4), required=True)
    parser.add_argument("--scenario", choices=("retained-targets", "missing-targets"), required=True)
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    if output.exists():
        raise SystemExit("output must be fresh")
    output.mkdir(parents=True)
    edited = source / "src/backend/libs/insight-clickhouse/src/lib.rs"
    original = edited.read_text()
    document = {
        "application_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip(),
        "source_sha256": hashlib.sha256(original.encode()).hexdigest(),
        "rust": subprocess.check_output(["rustc", "-Vv"], text=True),
        "coverage_tool": subprocess.check_output(["cargo", "llvm-cov", "--version"], text=True),
        "package": PACKAGE,
        "scenario": args.scenario,
        "samples": [],
    }
    try:
        env = environment("baseline", output)
        run(["cargo", "fetch", "--locked"], source / "src/backend", env, output / "fetch.log")
        cases = scenarios(args.repeats)
        variants = VARIANTS
        if args.scenario == "missing-targets":
            cases = [("cold", 1, 1)] + [("cache-reuse", n, 1) for n in range(1, args.repeats + 1)]
            variants = ("baseline", "compiler-cache")
        for phase, repeat, revision in cases:
            updated = fixture(original, revision)
            if edited.read_text() != updated:
                edited.write_text(updated)
            order = variants if repeat % 2 else tuple(reversed(variants))
            peers = []
            for variant in order:
                LOG.info("Starting %s/%s/%s", phase, repeat, variant)
                if phase == "cache-reuse":
                    shutil.rmtree(output / variant / "target")
                row = sample(source, output, variant, phase, repeat)
                peers.append(row)
                document["samples"].append(row)
                (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
            if any(row["tests"]["outcomes"] != peers[0]["tests"]["outcomes"] for row in peers):
                raise RuntimeError("test outcomes differ between variants")
            if any(row["coverage"] != peers[0]["coverage"] for row in peers):
                raise RuntimeError("covered lines differ between variants")
    except Exception as error:
        document["error"] = str(error)
        raise
    finally:
        edited.write_text(original)
        (output / "results.json").write_text(json.dumps(document, indent=2) + "\n")
        (output / "summary.txt").write_text(summary(document))
        for variant in VARIANTS:
            shutil.rmtree(output / variant / "target", ignore_errors=True)
        shutil.rmtree(output / "compiler-cache" / "cache", ignore_errors=True)


if __name__ == "__main__":
    main()
