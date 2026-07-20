{# Source-agnostic file classification for git metrics. Lives in gold-side
   macros deliberately: silver class tables are incremental, so a taxonomy
   baked at staging time would only apply to newly synced rows; computed in
   a gold view it applies retroactively on every read.

   Precedence: vendored -> test -> docs -> config -> code. `vendored` wins
   over everything so machine-produced content (vendored deps, build output,
   generated code, minified assets) never inflates authored line counts, no
   matter its extension or folder — a `.js` under `node_modules/` is vendored,
   not code. A `.yaml` under `tests/` is test; a `.md` under `docs/` is docs.
   Labels are static product vocabulary, not source data. #}

{# Single source of truth for the vendored/generated path patterns, shared by
   every classifier call site so the exclusion is defined once and applied
   consistently. Patterns are configurable via the `git_vendored_path_patterns`
   dbt var (see dbt_project.yml) so the list evolves without a code change. #}
{% macro git_vendored_path_regex() -%}
{%- set patterns = var('git_vendored_path_patterns') -%}
{%- if patterns | length == 0 -%}
{# Empty override disables the exclusion. Emit a never-match pattern rather
   than an empty group `(?i)()`, which would match every path and flag all
   files as vendored. `[^\s\S]` is the RE2-safe never-match (RE2 has no
   lookahead), doubled-escaped for the ClickHouse string literal. #}
'[^\\s\\S]'
{%- else -%}
'(?i)(' ~ (patterns | join('|')) ~ ')'
{%- endif -%}
{%- endmacro %}

{% macro git_file_category(path_expr) %}
multiIf(
    match({{ path_expr }}, {{ git_vendored_path_regex() }}), 'vendored',
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
    {{ category_expr }} = 'vendored', 'Vendored / Generated',
    {{ category_expr }}
)
{% endmacro %}

{# Display label for the git `source` dimension. Static product vocabulary
   (same rationale as the category labels), centralized so the gold view
   carries no inline vendor mapping. #}
{% macro git_source_label(source_expr) %}
multiIf(
    {{ source_expr }} = 'github', 'GitHub',
    {{ source_expr }} = 'gitlab', 'GitLab',
    {{ source_expr }} = 'bitbucket_cloud', 'Bitbucket Cloud',
    {{ source_expr }}
)
{% endmacro %}
