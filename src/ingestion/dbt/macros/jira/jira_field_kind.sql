{#-
  Classifies a Jira field into the closed `field_kind` enum that drives value
  normalization and delta application — see
  `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §3.

  Returns a SQL expression, so it is callable from a model and from a singular
  test over literal fixtures.

  INVARIANT: no `customfield_` literal may appear here. Field ids are
  instance-specific; Jira type constants are not. `assert_jira_field_kind_is_id_independent`
  enforces this.

  Branch order is load-bearing:
    1. bundled-app keys, which override structure;
    2. `schema_type = 'any'`, where Jira declares an app owns the type and
       structure carries no information — anything unrecognised here must reach
       UNKNOWN rather than fall through to a structural bucket;
    3. structural buckets.
-#}

{% macro jira_field_kind(field_id, schema_type, schema_items, schema_custom) %}
{%- set fid = "COALESCE(" ~ field_id ~ ", '')" -%}
{%- set styp = "COALESCE(" ~ schema_type ~ ", '')" -%}
{%- set sitm = "COALESCE(" ~ schema_items ~ ", '')" -%}
{%- set scus = "COALESCE(" ~ schema_custom ~ ", '')" -%}
multiIf(
    endsWith({{ scus }}, ':gh-sprint'),                                      'legacy_list',

    endsWith({{ scus }}, ':gh-epic-link')
        OR endsWith({{ scus }}, ':jpo-custom-field-parent'),                 'issue_ref',

    endsWith({{ scus }}, ':gh-lexo-rank')
        OR endsWith({{ scus }}, ':timeinstatus')
        OR endsWith({{ scus }}, ':devsummarycf')
        OR endsWith({{ scus }}, ':vulnerabilitycf'),                         'ignored',

    {{ styp }} = 'any',                                                      'UNKNOWN',

    {{ styp }} IN ('progress', 'watches', 'votes', 'statusCategory',
                   'securitylevel', 'timetracking', 'issuerestriction',
                   'comments-page'),                                         'ignored',
    {{ sitm }} = 'worklog',                                                  'ignored',
    {{ fid }} IN ('issuekey', 'thumbnail'),                                  'ignored',
    {{ styp }} IN ('option-with-child', 'option2', 'team', 'atlas-project',
                   'sd-request-lang', 'sd-approvals', 'sd-feedback',
                   'sd-customerrequesttype'),                                'ignored',

    {{ styp }} = 'array' AND {{ sitm }} = 'string',                          'string_array',
    {#- Issue links are element-wise like a component, but the two sides of the
        pipeline identify an element differently: the changelog names the LINKED
        ISSUE by key while the issue resource names the LINK OBJECT by its own
        id, with the key nested inside. Same asymmetry as `issue_ref`, one level
        up, so it needs its own kind rather than a shared normalizer. -#}
    {{ styp }} = 'array' AND {{ sitm }} = 'issuelinks'
        AND {{ scus }} = '',                                                 'link_array',
    {{ styp }} = 'array'
        AND {{ sitm }} IN ('component', 'version', 'attachment')
        AND {{ scus }} = '',                                                 'obj_array',
    {{ styp }} = 'array',                                                    'option_array',

    {{ fid }} = 'parent',                                                    'obj',
    {{ styp }} IN ('status', 'priority', 'resolution', 'issuetype',
                   'project'),                                               'obj',
    {{ styp }} = 'option',                                                   'option',
    {{ styp }} = 'user',                                                     'user',
    {{ fid }} IN ('description', 'environment')
        OR endsWith({{ scus }}, ':textarea'),                                'long_text',

    {#- `date` deliberately stays `scalar`. A date-only field spells itself the
        same way on both sides (`2026-01-05`), so canonicalizing it would turn a
        value that already reconciles into a datetime and break it. -#}
    {{ styp }} = 'datetime',                                                 'datetime',

    {#- The time-tracking estimates, where `0` and "absent" are the same state.
        Structure cannot separate them from a story-point estimate — all three
        are plain numbers — so they are named, which is what naming a Jira
        SYSTEM field is for. The equivalence was measured, not assumed: see
        FIELD-HISTORY-IN-DBT.md §3.5. -#}
    {{ fid }} IN ('timeestimate', 'timeoriginalestimate'),                   'duration',

    {{ styp }} IN ('string', 'number', 'date'),                              'scalar',

    'UNKNOWN'
)
{% endmacro %}
