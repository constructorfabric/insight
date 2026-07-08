{# Source-agnostic file classification for git metrics. Lives in gold-side
   macros deliberately: silver class tables are incremental, so a taxonomy
   baked at staging time would only apply to newly synced rows; computed in
   a gold view it applies retroactively on every read.

   Precedence: test -> docs -> config -> code. A `.yaml` under `tests/` is
   test; a `.md` under `docs/` is docs. Labels are static product
   vocabulary, not source data. #}

{% macro git_file_category(path_expr) %}
multiIf(
    match({{ path_expr }}, '(?i)(\\.spec\\.|\\.test\\.|__tests__/|(^|/)tests?/)'), 'test',
    match({{ path_expr }}, '(?i)(\\.(md|rst|adoc)$|(^|/)docs/)'), 'docs',
    match({{ path_expr }}, '(?i)(\\.lock$|package-lock\\.json|yarn\\.lock|poetry\\.lock|\\.ya?ml$|\\.toml$|\\.cfg$|\\.ini$)'), 'config',
    'code'
)
{% endmacro %}

{% macro git_file_category_label(category_expr) %}
multiIf(
    {{ category_expr }} = 'code', 'Code',
    {{ category_expr }} = 'test', 'Tests',
    {{ category_expr }} = 'config', 'Configuration',
    {{ category_expr }} = 'docs', 'Documentation',
    {{ category_expr }}
)
{% endmacro %}
