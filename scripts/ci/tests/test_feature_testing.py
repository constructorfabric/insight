from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import feature_testing as ft

ROOT = Path(__file__).resolve().parents[3]

REGISTRY = """
[[systems]]
name = "Demo"
slug = "demo"
kit = "sdlc"

[[systems.codebase]]
name = "Tests"
path = "tests"
extensions = [".py", ".ts"]

[[systems.artifacts]]
path = "docs/PRD.md"
kind = "PRD"
traceability = "DOCS-ONLY"

[[systems.artifacts]]
path = "docs/DESIGN.md"
kind = "DESIGN"
traceability = "DOCS-ONLY"

[[systems.artifacts]]
path = "docs/FEATURE.md"
kind = "FEATURE"
traceability = "DOCS-ONLY"

[[systems]]
name = "Legacy"
slug = "legacy"
kit = "sdlc"
codebase = ["ignored-string-entry"]
"""

PRD = """# PRD

## 5. Functional Requirements

#### Operator bind

- [ ] `p1` - **ID**: `cpt-demo-fr-bind`

The system **MUST** bind an unseen account.

## 6. Non-Functional Requirements

### 6.1 Quality Vector Analysis

| Quality Vector | Show-Stopper Requirement | Rationale |
|---|---|---|
| Efficiency | None — not material because the feature adds no storage | small |
| Performance | Binding MUST answer within budget, `cpt-demo-nfr-latency` | operators wait |

### 6.2 NFR Inclusions

#### Bind latency

- [ ] `p1` - **ID**: `cpt-demo-nfr-latency`

The system **MUST** answer a bind quickly.

**Threshold**: p95 under 500 ms at the reference organization.

#### Audit retention

- [ ] `p1` - **ID**: `cpt-demo-nfr-audit`

**Vector**: Security

The system **MUST** keep the audit row.

**Threshold**: every bind leaves one audit row.
"""

DESIGN = """# DESIGN

#### NFR Allocation

| NFR ID | Allocated To | Design Response | Verification Approach |
|--------|--------------|-----------------|-----------------------|
| `cpt-demo-nfr-latency` | API | index on account | Insight · API endpoints, p95 panel, 7d, insight-dev |
"""

DESIGN_GATEWAY = """# Gateway DESIGN

#### NFR Allocation

| NFR ID | Allocated To | Design Response | Verification Approach |
|---|---|---|---|
| `cpt-demo-nfr-latency` | Edge cache | shared-memory lookup | Load test measured at the gateway |
"""

FEATURE_BULLETS = """# Feature: Demo

## 1. Feature Context

### 1.2 Purpose

Why.

**Requirements**:

- `cpt-demo-fr-bind`
- `cpt-demo-nfr-latency`
- `cpt-demo-nfr-audit`
- `cpt-demo-fr-bind`

**Principles**:

- `cpt-demo-principle-x`

## 6. Acceptance Criteria

- [ ] it works
"""

FEATURE_INLINE = """# Feature: Demo

### 1.2 Purpose

**Requirements**: `cpt-demo-fr-bind`, `cpt-demo-nfr-latency`

**Principles**: `cpt-demo-principle-x`
"""


def make_project(tmp: Path) -> Path:
    (tmp / ".cf-studio" / "config").mkdir(parents=True)
    (tmp / ".cf-studio" / "config" / "artifacts.toml").write_text(REGISTRY)
    (tmp / "docs").mkdir()
    (tmp / "docs" / "PRD.md").write_text(PRD)
    (tmp / "docs" / "DESIGN.md").write_text(DESIGN)
    (tmp / "docs" / "FEATURE.md").write_text(FEATURE_BULLETS)
    (tmp / "tests" / "stand" / "api").mkdir(parents=True)
    (tmp / "tests" / "unit").mkdir(parents=True)
    (tmp / "tests" / "stand" / "api" / "test_bind.py").write_text(
        "import pytest\n\n"
        "# @cpt-test:cpt-demo-fr-bind:p1\n"
        "@pytest.mark.reliability\n"
        "def test_bind_unseen_account():\n    assert True\n\n"
        "# @cpt-test:cpt-demo-nfr-latency:p1\n"
        "# @cpt-test:cpt-demo-fr-bind:p1\n"
        "async def test_bind_is_fast():\n    assert True\n"
    )
    (tmp / "tests" / "unit" / "bind.test.ts").write_text(
        "// @cpt-test:cpt-demo-fr-bind:p1\nit('binds in the browser', () => {});\n"
    )
    return tmp


def add_artifact(root: Path, kind: str, rel: str, text: str) -> None:
    (root / rel).write_text(text)
    toml = root / ".cf-studio" / "config" / "artifacts.toml"
    toml.write_text(
        toml.read_text() + f'\n[[systems.artifacts]]\npath = "{rel}"\nkind = "{kind}"\ntraceability = "DOCS-ONLY"\n'
    )


class RegistryTests(unittest.TestCase):
    def test_registry_reads_table_entries_only(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            reg = ft.load_registry(root)
            self.assertEqual([c.path for c in reg.codebase], [Path("tests")])
            self.assertEqual(reg.codebase[0].extensions, (".py", ".ts"))
            self.assertEqual(sorted(a.kind for a in reg.artifacts), ["DESIGN", "FEATURE", "PRD"])


class CitationTests(unittest.TestCase):
    def test_scan_finds_every_citation_with_its_test_name(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            cites = ft.scan_citations(ft.load_registry(root), root)
            bind = sorted((str(c.path), c.line, c.test_name) for c in cites["cpt-demo-fr-bind"])
            self.assertEqual(
                bind,
                [
                    ("tests/stand/api/test_bind.py", 3, "test_bind_unseen_account"),
                    ("tests/stand/api/test_bind.py", 9, "test_bind_is_fast"),
                    ("tests/unit/bind.test.ts", 1, "binds in the browser"),
                ],
            )
            self.assertEqual([c.test_name for c in cites["cpt-demo-nfr-latency"]], ["test_bind_is_fast"])

    def test_scan_ignores_other_marker_kinds(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            (root / "tests" / "unit" / "test_other.py").write_text(
                "# @cpt-dod:cpt-demo-dod-x:p1\ndef test_x():\n    pass\n"
            )
            cites = ft.scan_citations(ft.load_registry(root), root)
            self.assertNotIn("cpt-demo-dod-x", cites)


class SuiteTests(unittest.TestCase):
    def test_suite_by_path(self) -> None:
        cases = {
            "tests/stand/api/x.py": ("stand-api", True),
            "tests/stand/ui/x.py": ("stand-ui", True),
            "tests/datapath/identity/x.py": ("identity-e2e", True),
            "tests/datapath/metrics/x.py": ("metric-spec", True),
            "src/ingestion/tests/x.py": ("ingestion-e2e", True),
            "src/backend/services/authenticator/tests/x.rs": ("auth-rig", False),
            "src/ingestion/connectors/git/jira/tests/x.py": ("connector-tests", False),
            "src/ingestion/dbt/tests/x.sql": ("dbt-tests", False),
            "src/frontend/src/a/b.stories.tsx": ("fe-component", False),
            "src/frontend/src/a/b.test.tsx": ("fe-unit", False),
            "src/backend/tools/routegen/tests/x.rs": ("unit", False),
        }
        for rel, want in cases.items():
            self.assertEqual(ft.suite_for(Path(rel)), want, rel)


class RequirementTests(unittest.TestCase):
    def test_bulleted_list_in_order_without_duplicates(self) -> None:
        self.assertEqual(
            ft.feature_requirements(FEATURE_BULLETS), ["cpt-demo-fr-bind", "cpt-demo-nfr-latency", "cpt-demo-nfr-audit"]
        )

    def test_inline_list(self) -> None:
        self.assertEqual(ft.feature_requirements(FEATURE_INLINE), ["cpt-demo-fr-bind", "cpt-demo-nfr-latency"])


class DefinitionTests(unittest.TestCase):
    def test_definitions_carry_heading_threshold_and_vector(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            defs = ft.load_definitions(ft.load_registry(root), root)
            lat = defs["cpt-demo-nfr-latency"]
            self.assertEqual(lat.heading, "Bind latency")
            self.assertEqual(lat.threshold, "p95 under 500 ms at the reference organization.")
            self.assertEqual(lat.vector, "Performance")
            self.assertEqual(defs["cpt-demo-nfr-audit"].vector, "Security")
            self.assertEqual(defs["cpt-demo-fr-bind"].heading, "Operator bind")
            self.assertEqual(defs["cpt-demo-fr-bind"].vector, "")

    def test_verification_cell_from_design(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            reg = ft.load_registry(root)
            self.assertEqual(
                ft.verification_for("cpt-demo-nfr-latency", reg, root),
                "Insight · API endpoints, p95 panel, 7d, insight-dev",
            )
            self.assertEqual(ft.verification_for("cpt-demo-nfr-audit", reg, root), "")

    def test_verification_joins_every_design_allocating_the_nfr(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            add_artifact(root, "DESIGN", "docs/GATEWAY-DESIGN.md", DESIGN_GATEWAY)
            self.assertEqual(
                ft.verification_for("cpt-demo-nfr-latency", ft.load_registry(root), root),
                "Insight · API endpoints, p95 panel, 7d, insight-dev; Load test measured at the gateway",
            )


class RenderTests(unittest.TestCase):
    def test_tables_and_gate(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("### 7.1 Requirement verification", block)
            self.assertIn(
                "| `cpt-demo-fr-bind` | test_bind.py::test_bind_unseen_account, "
                "test_bind.py::test_bind_is_fast, bind.test.ts::binds in the browser | stand-api, unit | yes |  |",
                block,
            )
            self.assertIn("### 7.2 Quality metrics", block)
            self.assertIn(
                "| Performance | `cpt-demo-nfr-latency` | Bind latency | p95 under 500 ms at the reference organization. "
                "|  | test_bind.py::test_bind_is_fast; Insight · API endpoints, p95 panel, 7d, insight-dev |  |",
                block,
            )
            self.assertIn(
                "| Security | `cpt-demo-nfr-audit` | Audit retention | every bind leaves one audit row. |  | none |  |",
                block,
            )
            self.assertNotIn("Reliability", block.split("### 7.2")[0])
            self.assertEqual(failures, ["cpt-demo-nfr-audit: no source and no note"])

    def test_unknown_id_is_reported_not_dropped(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            (root / "docs" / "FEATURE.md").write_text(FEATURE_BULLETS.replace("cpt-demo-nfr-audit", "cpt-demo-fr-gone"))
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("| `cpt-demo-fr-gone` | unknown id |  | no |  |", block)
            self.assertIn("cpt-demo-fr-gone: unknown id", failures)

    def test_authored_cells_survive_regeneration(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            text = (root / "docs" / "FEATURE.md").read_text()
            text = text.replace(
                "| Security | `cpt-demo-nfr-audit` | Audit retention | every bind leaves one audit row. |  | none |  |",
                "| Security | `cpt-demo-nfr-audit` | Audit retention | every bind leaves one audit row. "
                "| 1 row on 2026-10-01 | none | observed by hand |",
            )
            (root / "docs" / "FEATURE.md").write_text(text)
            self.assertFalse(ft.apply(Path("docs/FEATURE.md"), root, write=False))
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("| 1 row on 2026-10-01 | none | observed by hand |", block)
            self.assertEqual(failures, [])

    def test_write_appends_section_when_missing_and_is_idempotent(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            self.assertTrue(ft.apply(Path("docs/FEATURE.md"), root, write=True))
            text = (root / "docs" / "FEATURE.md").read_text()
            self.assertEqual(text.count("## 7. Testing"), 1)
            self.assertEqual(text.count(ft.BEGIN), 1)
            self.assertFalse(ft.apply(Path("docs/FEATURE.md"), root, write=True))

    def test_write_places_block_at_end_of_existing_section(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            (root / "docs" / "FEATURE.md").write_text(FEATURE_BULLETS + "\n## 7. Testing\n\nIntro kept.\n")
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            text = (root / "docs" / "FEATURE.md").read_text()
            self.assertLess(text.index("Intro kept."), text.index(ft.BEGIN))
            self.assertEqual(text.count("## 7. Testing"), 1)


class SkillDocTests(unittest.TestCase):
    def test_suite_table_names_only_suites_the_tool_emits(self) -> None:
        skill = (ROOT / ".claude" / "skills" / "quality-vector-tests" / "SKILL.md").read_text()
        table = skill.split("| Target component |", 1)[1].split("\n\n", 1)[0]
        named = set(re.findall(r"`([a-z][a-z0-9-]*)`", table))
        emitted = {suite for _, suite in ft.SUITE_PREFIXES} | {"fe-component", "unit"}
        self.assertEqual(named - emitted, set())


class CliTests(unittest.TestCase):
    def test_check_ignores_features_without_block_and_fails_on_stale(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            self.assertEqual(ft.main(["--check", "--root", str(root)]), 0)
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            self.assertEqual(ft.main(["--check", "--root", str(root)]), 0)
            (root / "tests" / "unit" / "bind.test.ts").unlink()
            self.assertEqual(ft.main(["--check", "--root", str(root)]), 1)

    def test_check_scans_citations_once_for_all_features(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            add_artifact(root, "FEATURE", "docs/FEATURE2.md", FEATURE_INLINE)
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            ft.apply(Path("docs/FEATURE2.md"), root, write=True)
            with mock.patch.object(ft, "scan_citations", wraps=ft.scan_citations) as scan:
                self.assertEqual(ft.main(["--check", "--root", str(root)]), 0)
            self.assertEqual(scan.call_count, 1)

    def test_gate_exit_code(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            self.assertEqual(ft.main(["docs/FEATURE.md", "--gate", "--root", str(root)]), 1)


if __name__ == "__main__":
    unittest.main()


class ReviewFixTests(unittest.TestCase):
    def test_authored_cell_with_escaped_pipe_survives(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            path = root / "docs" / "FEATURE.md"
            text = path.read_text().replace(
                "| every bind leaves one audit row. |  | none |  |",
                "| every bind leaves one audit row. | p95 1.2 s \\| cf-prod | none | by hand \\| KT |",
            )
            path.write_text(text)
            self.assertFalse(ft.apply(Path("docs/FEATURE.md"), root, write=False))
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("| p95 1.2 s \\| cf-prod | none | by hand \\| KT |", block)
            self.assertEqual(failures, [])

    def test_unknown_id_marker_is_not_read_back_as_a_note(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            feature = root / "docs" / "FEATURE.md"
            feature.write_text(FEATURE_BULLETS.replace("cpt-demo-nfr-audit", "cpt-demo-fr-gone"))
            ft.apply(Path("docs/FEATURE.md"), root, write=True)
            prd = root / "docs" / "PRD.md"
            prd.write_text(
                prd.read_text() + "\n#### Gone\n\n- [ ] `p1` - **ID**: `cpt-demo-fr-gone`\n\nThe system **MUST** go.\n"
            )
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("| `cpt-demo-fr-gone` | none |  | no |  |", block)
            self.assertIn("cpt-demo-fr-gone: no test and no note", failures)

    def test_cited_dod_of_this_feature_renders_and_uncited_dod_is_not_a_gap(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            feature = root / "docs" / "FEATURE.md"
            feature.write_text(
                FEATURE_BULLETS
                + "\n## 5. Definitions of Done\n\n### Verbs\n\n- [ ] `p1` - **ID**: `cpt-demo-dod-verbs`\n\n"
                "### Quiet\n\n- [ ] `p1` - **ID**: `cpt-demo-dod-quiet`\n"
            )
            (root / "tests" / "unit" / "test_dod.py").write_text(
                "# @cpt-test:cpt-demo-dod-verbs:p1\ndef test_verbs():\n    pass\n"
            )
            block, failures = ft.render(Path("docs/FEATURE.md"), root)
            self.assertIn("| `cpt-demo-dod-verbs` | test_dod.py::test_verbs | unit | no |  |", block)
            self.assertNotIn("cpt-demo-dod-quiet", block)
            self.assertNotIn("cpt-demo-dod-quiet: no test and no note", failures)

    def test_multiline_threshold_is_kept_whole(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = make_project(Path(d))
            prd = root / "docs" / "PRD.md"
            prd.write_text(
                prd.read_text().replace(
                    "**Threshold**: p95 under 500 ms at the reference organization.",
                    "**Threshold**: p95 under 500 ms at the reference organization,\nmeasured at the gateway.",
                )
            )
            defs = ft.load_definitions(ft.load_registry(root), root)
            self.assertEqual(
                defs["cpt-demo-nfr-latency"].threshold,
                "p95 under 500 ms at the reference organization, measured at the gateway.",
            )
