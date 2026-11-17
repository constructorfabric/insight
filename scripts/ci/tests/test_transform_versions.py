from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import transform_versions as guard


def model(sql: str = "SELECT 1", *, version: guard.Json = 1, **config: guard.Json) -> dict[str, guard.Json]:
    return {
        "resource_type": "model",
        "raw_code": sql,
        "config": {
            "meta": {"transform_version": version},
            "materialized": "table",
            "contract": {"enforced": False},
            **config,
        },
        "schema": "example",
        "alias": "example",
        "database": "",
        "description": "",
        "columns": {},
        "tags": config.get("tags", []),
        "depends_on": {"macros": [], "nodes": []},
    }


def macro(sql: str, *dependencies: str) -> dict[str, guard.Json]:
    return {"macro_sql": sql, "depends_on": {"macros": list(dependencies)}}


def manifest(
    nodes: dict[str, guard.Json], macros: dict[str, guard.Json], variables: dict[str, guard.Json] | None = None
) -> guard.Manifest:
    return guard.read_manifest({"nodes": nodes, "macros": macros}, {} if variables is None else variables)


def judge(before: dict[str, guard.Json], after: dict[str, guard.Json]) -> list[guard.Finding]:
    return guard.compare(manifest({"model.example.one": before}, {}), manifest({"model.example.one": after}, {}))


def union_manifest(contributing: bool, *, hint: bool, version: int = 1) -> guard.Manifest:
    tags: list[guard.Json] = ["silver:example"] if contributing else []
    producer = model(f"{{{{ config(tags={tags!r}) }}}} SELECT 1", tags=tags)
    consumer = model("SELECT * FROM {{ union_by_tag('silver:example') }}", version=version)
    guard.object_value(consumer["depends_on"])["macros"] = ["macro.example.union_by_tag"]
    if hint:
        consumer["raw_code"] = "-- depends_on: {{ ref('producer') }}\n" + str(consumer["raw_code"])
        guard.object_value(consumer["depends_on"])["nodes"] = ["model.example.producer"]
    nodes = {"model.example.consumer": consumer, "model.example.producer": producer}
    return manifest(nodes, {"macro.example.union_by_tag": macro("SELECT 1")})


@pytest.mark.parametrize("macro_sql", [False, True], ids=["model", "macro"])
@pytest.mark.parametrize(
    ("before", "after"),
    [
        pytest.param("SELECT (1)", "SELECT ( 1 )", id="parentheses"),
        pytest.param("SELECT a,b", "SELECT a, b", id="comma"),
        pytest.param("SELECT 1+2", "SELECT 1/* note */+2", id="inline-comment"),
        pytest.param("SELECT a,\nb", "SELECT a,b", id="line-join"),
        pytest.param("SELECT 1+2", "SELECT 1{# note #}+2", id="jinja-comment"),
        pytest.param("SELECT 1+2", "SELECT 1-- note\n+2", id="line-comment"),
        pytest.param("SELECT a+b", "SELECT a + b", id="operator-spacing"),
        pytest.param("SELECT a+-1", "SELECT a + -1", id="unary-spacing"),
        pytest.param("SELECT a.b", "SELECT a . b", id="qualified-name"),
        pytest.param("{{ leaf(1,2) }}", "{{ leaf( 1, 2 ) }}", id="jinja-expression"),
        pytest.param("SELECT a b", "SELECT a/* note */b", id="word-comment"),
        pytest.param("SELECT a - -1", "SELECT a -/* note */-1", id="operator-comment"),
    ],
)
def test_harmless_spacing_does_not_require_a_bump(before: str, after: str, macro_sql: bool) -> None:
    assert text_findings(before, after, macro_sql=macro_sql) == [], (before, after)


@pytest.mark.parametrize("macro_sql", [False, True], ids=["model", "macro"])
@pytest.mark.parametrize(
    ("before", "after"),
    [
        pytest.param("SELECT 1+2", "SELECT 1-2", id="operator-change"),
        pytest.param("SELECT 'a b'", "SELECT 'a  b'", id="string-space"),
        pytest.param('SELECT "a b"', 'SELECT "a  b"', id="quoted-identifier-space"),
        pytest.param("SELECT `a b`", "SELECT `a  b`", id="backtick-space"),
        pytest.param("SELECT $q$a b$q$", "SELECT $q$a  b$q$", id="heredoc-space"),
        pytest.param("SELECT $q$a--x\nb$q$", "SELECT $q$a--y\nb$q$", id="heredoc-comment-text"),
        pytest.param("SELECT ab", "SELECT a b", id="identifier-split"),
        pytest.param("{{ a\u0301 }}", "{{ a \u0301 }}", id="unicode-identifier-split"),
        pytest.param("SELECT a - -1", "SELECT a --1", id="line-comment-opener"),
        pytest.param("SELECT a / * b */ 1", "SELECT a /* b */ 1", id="block-comment-opener"),
        pytest.param("SELECT a < = b", "SELECT a <= b", id="comparison"),
        pytest.param("SELECT a ! = b", "SELECT a != b", id="inequality"),
        pytest.param("SELECT a | | b", "SELECT a || b", id="concatenation"),
        pytest.param("SELECT a : : Int64", "SELECT a :: Int64", id="cast"),
        pytest.param("SELECT a - > b", "SELECT a -> b", id="lambda"),
        pytest.param("{{ 2 * * 3 }}", "{{ 2 ** 3 }}", id="power"),
        pytest.param("{{ 2 / / 3 }}", "{{ 2 // 3 }}", id="floor-division"),
        pytest.param("{ { leaf() }}", "{{ leaf() }}", id="jinja-expression-opener"),
        pytest.param("{{ leaf() } }", "{{ leaf() }}", id="jinja-expression-closer"),
        pytest.param("{ % set a = 1 %}", "{% set a = 1 %}", id="jinja-statement-opener"),
        pytest.param("{% set a = 1 % }", "{% set a = 1 %}", id="jinja-statement-closer"),
        pytest.param("{ # note #}", "{# note #}", id="jinja-comment-opener"),
        pytest.param("{{ -1 }}", "{{-1 }}", id="jinja-trim-opener"),
        pytest.param("{{ 1 - }}", "{{ 1 -}}", id="jinja-trim-closer"),
        pytest.param("{% + set a = 1 %}", "{%+ set a = 1 %}", id="jinja-keep-opener"),
        pytest.param("{% set a = 1 + %}", "{% set a = 1 +%}", id="jinja-keep-closer"),
        pytest.param("SELECT x '61'", "SELECT x'61'", id="hex-literal"),
        pytest.param("SELECT b '01'", "SELECT b'01'", id="binary-literal"),
        pytest.param("SELECT 'a' 'b'", "SELECT 'a''b'", id="adjacent-quotes"),
        pytest.param("SELECT 1 .2", "SELECT 1.2", id="decimal-point-left"),
        pytest.param("SELECT 1. 2", "SELECT 1.2", id="decimal-point-right"),
        pytest.param("SELECT 1e +2", "SELECT 1e+2", id="exponent-sign"),
        pytest.param("SELECT 1e+ 2", "SELECT 1e+2", id="exponent-digits"),
        pytest.param("SELECT .1e -2", "SELECT .1e-2", id="fraction-exponent-sign"),
        pytest.param("SELECT .1e- 2", "SELECT .1e-2", id="fraction-exponent-digits"),
        pytest.param("SELECT 1. e2", "SELECT 1.e2", id="decimal-exponent"),
        pytest.param("SELECT 0x1. fp2", "SELECT 0x1.fp2", id="hex-fraction"),
        pytest.param("SELECT 0x1.fp +2", "SELECT 0x1.fp+2", id="hex-exponent-sign"),
        pytest.param("SELECT 0x1.fp+ 2", "SELECT 0x1.fp+2", id="hex-exponent-digits"),
    ],
)
def test_meaningful_text_changes_require_a_bump(before: str, after: str, macro_sql: bool) -> None:
    changes = ("macro macro.example.leaf",) if macro_sql else ("sql",)
    expected = [guard.Finding("model.example.one", 1, 1, changes)]
    assert text_findings(before, after, macro_sql=macro_sql) == expected, (before, after)


def text_findings(before: str, after: str, *, macro_sql: bool) -> list[guard.Finding]:
    if not macro_sql:
        return judge(model(before), model(after))

    node = model()
    node["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
    old = manifest({"model.example.one": node}, {"macro.example.leaf": macro(before)})
    new = manifest({"model.example.one": node}, {"macro.example.leaf": macro(after)})
    return guard.compare(old, new)


@pytest.mark.parametrize(
    ("before", "after"),
    [
        pytest.param("SELECT(1)", "SELECT{{ config(tags=['example']) }}(1)", id="parentheses"),
        pytest.param("SELECT a,b", "SELECT a,{{ config(tags=['example']) }}b", id="comma"),
    ],
)
def test_inline_config_does_not_introduce_significant_spacing(before: str, after: str) -> None:
    assert judge(model(before), model(after)) == [], (before, after)


class TransformVersionTests(unittest.TestCase):
    def test_sql_changes_require_a_higher_version(self) -> None:
        findings = judge(model(), model("SELECT 2"))
        self.assertEqual(findings, [guard.Finding("model.example.one", 1, 1, ("sql",))])
        self.assertIn("transform_version 1 -> 1: sql", str(findings[0]))

    def test_sql_changes_with_a_higher_version_pass(self) -> None:
        self.assertEqual(judge(model(), model("SELECT 2", version=2)), [])

    def test_comments_and_whitespace_do_not_change_a_definition(self) -> None:
        for sql in (
            "SELECT   1\n",
            "-- leading\nSELECT 1 -- trailing",
            "SELECT /* block\n */ 1",
            "SELECT {# note #} 1",
        ):
            with self.subTest(sql=sql):
                self.assertEqual(judge(model(), model(sql)), [], sql)

    def test_comment_markers_and_whitespace_in_literals_are_significant(self) -> None:
        for literal in ("'--a'", "'/*a*/'", "'{#a#}'", "'a  b'", '"a  b"', "`a  b`"):
            with self.subTest(literal=literal):
                self.assertEqual(guard.normalize(f"SELECT {literal}"), f"SELECT {literal}")
                self.assertTrue(judge(model("SELECT ''"), model(f"SELECT {literal}")), literal)

    def test_build_config_changes_name_the_key(self) -> None:
        for key, value in {
            "order_by": "id",
            "unique_key": "id",
            "materialized": "incremental",
            "engine": "MergeTree()",
            "partition_by": "day",
            "full_refresh": True,
            "on_schema_change": "append_new_columns",
            "incremental_strategy": "append",
            "query_settings": {"max_threads": 2},
        }.items():
            with self.subTest(key=key):
                self.assertEqual(judge(model(), model(**{key: value}))[0].changes, (f"config {key}",))

    def test_relation_names_are_effective_build_config(self) -> None:
        for key in ("schema", "alias", "database"):
            after = model()
            after[key] = "other"
            with self.subTest(key=key):
                self.assertEqual(judge(model(), after)[0].changes, (f"config {key}",))

    def test_documentation_config_does_not_change_a_definition(self) -> None:
        for key, value in {
            "tags": ["example"],
            "description": "Example description",
            "docs": {"show": False},
            "persist_docs": {"relation": True},
        }.items():
            with self.subTest(key=key):
                self.assertEqual(judge(model(), model(**{key: value})), [], key)

    def test_model_and_column_descriptions_do_not_change_a_definition(self) -> None:
        after = model()
        after["description"] = "Example description"
        after["columns"] = {"id": {"description": "Example identifier"}}
        self.assertEqual(judge(model(), after), [])

    def test_inline_metadata_and_version_bumps_do_not_change_sql(self) -> None:
        for config in (
            "{{ config(meta={'transform_version': 2}) }}",
            "{{- config(meta={'transform_version': 2, 'example': ')}}'}, tags=['example']) -}}",
            "{{ config({'meta': {'transform_version': 2}}, order_by=('id')) }}",
        ):
            with self.subTest(config=config):
                self.assertEqual(judge(model(version=2), model(f"{config}\nSELECT 1", version=2, tags=["example"])), [])

    def test_inline_tags_are_compared_through_effective_config(self) -> None:
        before = model("{{ config(tags=['before']) }} SELECT 1", tags=["before"])
        after = model("{{ config(tags=['after']) }} SELECT 1", tags=["after"])
        self.assertEqual(judge(before, after), [])

    def test_macro_changes_reach_direct_and_transitive_users_only(self) -> None:
        direct = model()
        direct["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
        transitive = model()
        transitive["depends_on"] = {"macros": ["macro.example.wrapper"], "nodes": []}
        nodes = {"model.example.direct": direct, "model.example.transitive": transitive, "model.example.other": model()}
        old = {
            "macro.example.leaf": macro("{% macro leaf() %}1{% endmacro %}"),
            "macro.example.wrapper": macro("{% macro wrapper() %}{{ leaf() }}{% endmacro %}", "macro.example.leaf"),
        }
        new = copy.deepcopy(old)
        guard.object_value(new["macro.example.leaf"])["macro_sql"] = "{% macro leaf() %}2{% endmacro %}"

        findings = guard.compare(manifest(nodes, old), manifest(nodes, new))

        self.assertEqual([finding.model for finding in findings], ["model.example.direct", "model.example.transitive"])
        self.assertTrue(all(finding.changes == ("macro macro.example.leaf",) for finding in findings))

    def test_macro_comments_and_whitespace_do_not_require_a_bump(self) -> None:
        node = model()
        node["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
        old = manifest({"one": node}, {"macro.example.leaf": macro("SELECT 1")})
        for sql in ("SELECT   1", "SELECT /* note */ 1", "SELECT 1 -- note", "SELECT {# note #} 1"):
            with self.subTest(sql=sql):
                new = manifest({"one": node}, {"macro.example.leaf": macro(sql)})
                self.assertEqual(guard.compare(old, new), [], sql)

    def test_dispatch_dependencies_and_cycles_are_followed(self) -> None:
        node = model()
        node["depends_on"] = {"macros": ["macro.example.wrapper"], "nodes": []}
        old = {
            "macro.example.wrapper": macro("{{ adapter.dispatch('leaf')() }}", "macro.example.clickhouse__leaf"),
            "macro.example.clickhouse__leaf": macro("SELECT 1", "macro.example.wrapper"),
        }
        new = copy.deepcopy(old)
        guard.object_value(new["macro.example.clickhouse__leaf"])["macro_sql"] = "SELECT 2"
        findings = guard.compare(manifest({"one": node}, old), manifest({"one": node}, new))
        self.assertEqual(findings[0].changes, ("macro macro.example.clickhouse__leaf",))

    def test_removed_macro_dependencies_change_the_definition(self) -> None:
        before = model()
        before["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
        old = manifest({"one": before}, {"macro.example.leaf": macro("SELECT 1")})
        new = manifest({"one": model()}, {})
        self.assertEqual(guard.compare(old, new)[0].changes, ("macro macro.example.leaf",))

    def test_a_version_decrease_always_fails(self) -> None:
        for sql in ("SELECT 1", "SELECT 2"):
            with self.subTest(sql=sql):
                findings = judge(model(version=2), model(sql, version=1))
                self.assertEqual((findings[0].before, findings[0].after), (2, 1))
                self.assertIn("version decreased", findings[0].changes)

    def test_new_and_deleted_models_pass(self) -> None:
        empty = manifest({}, {})
        populated = manifest({"new": model()}, {})
        self.assertEqual(guard.compare(empty, populated), [])
        self.assertEqual(guard.compare(populated, empty), [])

    def test_versions_must_be_positive_integers(self) -> None:
        for version in (None, 0, -1, True, 1.5, "2"):
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "positive integer"):
                manifest({"one": model(version=version)}, {})

    def test_missing_versions_fail_even_for_new_models(self) -> None:
        unversioned = model()
        guard.object_value(unversioned["config"])["meta"] = {}
        findings = guard.compare(manifest({}, {}), manifest({"new": unversioned}, {}))
        self.assertEqual(findings[0].changes, ("missing transform_version",))

    def test_introducing_versions_establishes_the_baseline(self) -> None:
        before = model()
        guard.object_value(before["config"])["meta"] = {}
        self.assertEqual(judge(before, model()), [])

    def test_unversioned_base_models_still_require_a_bump_for_sql_changes(self) -> None:
        before = model()
        guard.object_value(before["config"])["meta"] = {}
        self.assertEqual(judge(before, model("SELECT 2"))[0].changes, ("sql",))
        self.assertEqual(judge(before, model("SELECT 2", version=2)), [])

    def test_enforced_column_contracts_are_build_config(self) -> None:
        before = model(contract={"enforced": True})
        before["columns"] = {"id": {"data_type": "UInt32", "constraints": [], "quote": None}}
        before["constraints"] = []
        after = copy.deepcopy(before)
        column = guard.object_value(guard.object_value(after["columns"])["id"])
        column["data_type"] = "UInt64"
        self.assertEqual(judge(before, after)[0].changes, ("config columns",))

    def test_metadata_inside_a_contract_is_documentation_only(self) -> None:
        before = model(contract={"enforced": True})
        before["columns"] = {"id": {"data_type": "UInt32", "constraints": [], "quote": None}}
        before["constraints"] = []
        after = copy.deepcopy(before)
        column = guard.object_value(guard.object_value(after["columns"])["id"])
        column["description"] = "Example identifier"
        self.assertEqual(judge(before, after), [])

    def test_missing_macro_definitions_fail_closed(self) -> None:
        node = model()
        node["depends_on"] = {"macros": ["macro.example.missing"], "nodes": []}
        broken = manifest({"one": node}, {})
        with self.assertRaises(KeyError):
            guard.compare(broken, broken)

    def test_macro_changes_with_higher_model_versions_pass(self) -> None:
        before = model()
        before["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
        after = copy.deepcopy(before)
        guard.object_value(guard.object_value(after["config"])["meta"])["transform_version"] = 2
        old = manifest({"one": before}, {"macro.example.leaf": macro("SELECT 1")})
        new = manifest({"one": after}, {"macro.example.leaf": macro("SELECT 2")})
        self.assertEqual(guard.compare(old, new), [])

    def test_disabled_models_keep_version_enforcement(self) -> None:
        old = manifest({"one": model(enabled=True)}, {})
        new = guard.read_manifest({"nodes": {}, "disabled": {"one": [model(enabled=False)]}, "macros": {}}, {})
        self.assertEqual(guard.compare(old, new)[0].changes, ("config enabled",))

    def test_history_before_versions_passes(self) -> None:
        before = model()
        after = model("SELECT 2")
        guard.object_value(before["config"])["meta"] = {}
        guard.object_value(after["config"])["meta"] = {}
        self.assertEqual(judge(before, after), [])

    def test_removing_versions_after_the_baseline_fails(self) -> None:
        after = model()
        guard.object_value(after["config"])["meta"] = {}
        self.assertEqual(judge(model(), after)[0].changes, ("missing transform_version",))

    def test_contributor_changes_require_only_the_consumer_to_bump(self) -> None:
        for before, after in ((False, True), (True, False)):
            with self.subTest(before=before, after=after):
                base = union_manifest(before, hint=before)
                head = union_manifest(after, hint=after)
                expected = guard.Finding("model.example.consumer", 1, 1, ("dependencies", "contributors"))
                self.assertEqual(guard.compare(base, head), [expected])
                bumped = union_manifest(after, hint=after, version=2)
                self.assertEqual(guard.compare(base, bumped), [])

    def test_contributor_tags_are_tracked_without_hints(self) -> None:
        for before, after in ((False, True), (True, False)):
            with self.subTest(before=before, after=after):
                base = union_manifest(before, hint=False)
                head = union_manifest(after, hint=False)
                self.assertEqual(
                    guard.compare(base, head), [guard.Finding("model.example.consumer", 1, 1, ("contributors",))]
                )

    def test_ref_and_source_changes_require_a_bump(self) -> None:
        for dependency in ("model.example.input", "source.example.bronze.input"):
            before = model()
            after = model()
            guard.object_value(after["depends_on"])["nodes"] = [dependency]
            with self.subTest(dependency=dependency):
                self.assertEqual(judge(before, after)[0].changes, ("dependencies",))
                self.assertEqual(judge(after, before)[0].changes, ("dependencies",))

    def test_dependency_order_is_irrelevant(self) -> None:
        before = model()
        after = model()
        guard.object_value(before["depends_on"])["nodes"] = ["model.example.a", "source.example.b"]
        guard.object_value(after["depends_on"])["nodes"] = ["source.example.b", "model.example.a", "source.example.b"]
        self.assertEqual(judge(before, after), [])

    def test_project_variable_changes_reach_only_sql_and_macro_readers(self) -> None:
        direct = model("SELECT {{ var('example_patterns') }}")
        direct_macro = model("SELECT {{ leaf() }}")
        direct_macro["depends_on"] = {"macros": ["macro.example.leaf"], "nodes": []}
        macro_reader = model("SELECT {{ wrapper() }}")
        macro_reader["depends_on"] = {"macros": ["macro.example.wrapper"], "nodes": []}
        macros = {
            "macro.example.wrapper": macro("{{ leaf() }}", "macro.example.leaf"),
            "macro.example.leaf": macro('{{ var("example_patterns", []) }}'),
        }
        nodes = {
            "model.example.direct_macro": direct_macro,
            "model.example.sql": direct,
            "model.example.transitive": macro_reader,
            "model.example.other": model(),
        }
        before = manifest(nodes, macros, {"example_patterns": ["^a"]})
        after = manifest(nodes, macros, {"example_patterns": ["^b"]})
        findings = guard.compare(before, after)
        self.assertEqual(
            findings,
            [
                guard.Finding("model.example.direct_macro", 1, 1, ("var example_patterns",)),
                guard.Finding("model.example.sql", 1, 1, ("var example_patterns",)),
                guard.Finding("model.example.transitive", 1, 1, ("var example_patterns",)),
            ],
        )

    def test_unrelated_project_variable_changes_pass(self) -> None:
        nodes = {"one": model("SELECT {{ var('used') }}")}
        before = manifest(nodes, {}, {"used": 1, "unused": 1})
        after = manifest(nodes, {}, {"used": 1, "unused": 2})
        self.assertEqual(guard.compare(before, after), [])

    def test_absent_project_variables_differ_from_null_values(self) -> None:
        nodes = {"one": model("SELECT {{ var('example', 'fallback') }}")}
        before = manifest(nodes, {}, {})
        after = manifest(nodes, {}, {"example": None})
        self.assertEqual(guard.compare(before, after), [guard.Finding("one", 1, 1, ("var example",))])
        self.assertEqual(guard.compare(after, before), [guard.Finding("one", 1, 1, ("var example",))])

    def test_literal_variable_names_with_or_without_defaults_are_tracked(self) -> None:
        for expression in ("var('example')", 'var("example")', "var('example', 0)", 'var("example", 0)'):
            with self.subTest(expression=expression):
                nodes = {"one": model("SELECT {{ " + expression + " }}")}
                before = manifest(nodes, {}, {"example": 1})
                after = manifest(nodes, {}, {"example": 2})
                self.assertEqual(guard.compare(before, after), [guard.Finding("one", 1, 1, ("var example",))])

    def test_dynamic_and_documentary_variable_calls_are_ignored(self) -> None:
        for sql in (
            "SELECT {{ var(name) }}",
            "SELECT {{ var('prefix' ~ suffix, 0) }}",
            "SELECT 1 -- var('example')",
            "SELECT /* var('example') */ 1",
            "SELECT {# var('example') #} 1",
            "SELECT 'var(\"example\")'",
        ):
            with self.subTest(sql=sql):
                nodes = {"one": model(sql)}
                before = manifest(nodes, {}, {"example": 1})
                after = manifest(nodes, {}, {"example": 2})
                self.assertEqual(guard.compare(before, after), [])
