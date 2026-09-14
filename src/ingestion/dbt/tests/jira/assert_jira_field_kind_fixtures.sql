-- Unit test for the `jira_field_kind` classifier: every kind, plus the pairs
-- where two fields share a structure and only one column separates them.
--
-- Fixtures are literal and synthetic — this test touches no table, so it is a
-- true unit test in the repository's singular-test idiom.

WITH fixtures AS (
    SELECT
        t.1 AS field_id,
        t.2 AS schema_type,
        t.3 AS schema_items,
        t.4 AS schema_custom,
        t.5 AS expected
    FROM (
        SELECT arrayJoin([
            -- system single-value objects
            ('status',            'status',            '',           '',                                                                'obj'),
            ('priority',          'priority',          '',           '',                                                                'obj'),
            ('resolution',        'resolution',        '',           '',                                                                'obj'),
            ('issuetype',         'issuetype',         '',           '',                                                                'obj'),
            ('project',           'project',           '',           '',                                                                'obj'),
            -- carries no schema at all, so it is matched by name
            ('parent',            '',                  '',           '',                                                                'obj'),

            ('assignee',          'user',              '',           '',                                                                'user'),
            ('customfield_000001','user',              '',           'com.atlassian.jira.plugin.system.customfieldtypes:userpicker',     'user'),
            ('customfield_000002','option',            '',           'com.atlassian.jira.plugin.system.customfieldtypes:select',         'option'),
            ('customfield_000003','option',            '',           'com.atlassian.jira.plugin.system.customfieldtypes:radiobuttons',   'option'),

            ('summary',           'string',            '',           '',                                                                'scalar'),
            ('timespent',         'number',            '',           '',                                                                'scalar'),
            -- the two estimates, where zero and absent are the same state; a
            -- story-point estimate is the same structure and is NOT one of them
            ('timeestimate',        'number',          '',           '',                                                                'duration'),
            ('timeoriginalestimate','number',          '',           '',                                                                'duration'),
            -- a date-only field stays `scalar`: both sides already spell it the
            -- same way, and canonicalizing it would turn it into an instant
            ('duedate',           'date',              '',           '',                                                                'scalar'),
            -- a datetime does not: the two sides differ in milliseconds
            ('created',           'datetime',          '',           '',                                                                'datetime'),
            ('customfield_000020','datetime',          '',           'com.atlassian.jira.plugin.system.customfieldtypes:datetime',       'datetime'),

            -- long text: no structural difference from `summary`, matched by name
            ('description',       'string',            '',           '',                                                                'long_text'),
            ('environment',       'string',            '',           '',                                                                'long_text'),
            ('customfield_000004','string',            '',           'com.atlassian.jira.plugin.system.customfieldtypes:textarea',       'long_text'),

            -- item type alone decides the labels family: system and custom agree
            ('labels',            'array',             'string',     '',                                                                'string_array'),
            ('customfield_000005','array',             'string',     'com.atlassian.jira.plugin.system.customfieldtypes:labels',         'string_array'),

            -- same structure, different shape: schema_custom is the discriminator
            ('fixVersions',       'array',             'version',    '',                                                                'obj_array'),
            ('customfield_000006','array',             'version',    'com.atlassian.jira.plugin.system.customfieldtypes:multiversion',   'option_array'),

            ('components',        'array',             'component',  '',                                                                'obj_array'),
            ('attachment',        'array',             'attachment', '',                                                                'obj_array'),
            -- element-wise like a component, but the two sides identify an
            -- element differently, so it is its own kind
            ('issuelinks',        'array',             'issuelinks', '',                                                                'link_array'),

            ('customfield_000007','array',             'option',     'com.atlassian.jira.plugin.system.customfieldtypes:multiselect',    'option_array'),
            ('customfield_000008','array',             'option',     'com.atlassian.jira.plugin.system.customfieldtypes:multicheckboxes','option_array'),
            ('customfield_000009','array',             'user',       'com.atlassian.jira.plugin.system.customfieldtypes:people',         'option_array'),
            -- an app puts its own display name in schema_items; structure still decides
            ('customfield_000010','array',             'Focus Areas','com.atlassian.jira.plugin.system.customfieldtypes:focus-areas',    'option_array'),

            ('customfield_000011','array',             'json',       'com.pyxis.greenhopper.jira:gh-sprint',                             'legacy_list'),

            -- schema_type 'any' carries no structure: resolved by app key only
            ('customfield_000012','any',               '',           'com.pyxis.greenhopper.jira:gh-epic-link',                          'issue_ref'),
            ('customfield_000013','any',               '',           'com.atlassian.jpo:jpo-custom-field-parent',                        'issue_ref'),
            ('customfield_000014','any',               '',           'com.pyxis.greenhopper.jira:gh-lexo-rank',                          'ignored'),
            ('customfield_000015','any',               '',           'com.atlassian.jira.ext.charting:timeinstatus',                     'ignored'),
            -- an unrecognised app type inside `any` must fail loudly, not vanish
            ('customfield_000016','any',               '',           'com.example.newapp:some-new-type',                                 'UNKNOWN'),

            ('worklog',           'array',             'worklog',    '',                                                                'ignored'),
            ('comment',           'comments-page',     '',           '',                                                                'ignored'),
            ('votes',             'votes',             '',           '',                                                                'ignored'),
            ('watches',           'watches',           '',           '',                                                                'ignored'),
            ('progress',          'progress',          '',           '',                                                                'ignored'),
            ('timetracking',      'timetracking',      '',           '',                                                                'ignored'),
            ('statusCategory',    'statusCategory',    '',           '',                                                                'ignored'),
            ('security',          'securitylevel',     '',           '',                                                                'ignored'),
            ('issuerestriction',  'issuerestriction',  '',           '',                                                                'ignored'),
            ('issuekey',          '',                  '',           '',                                                                'ignored'),
            ('thumbnail',         '',                  '',           '',                                                                'ignored'),
            ('customfield_000017','option-with-child', '',           'com.atlassian.jira.plugin.system.customfieldtypes:cascadingselect','ignored'),
            ('customfield_000018','sd-approvals',      '',           'com.atlassian.servicedesk.approvals-plugin:sd-approvals',          'ignored'),

            -- a structure nobody has classified
            ('customfield_000019','brand-new-type',    '',           '',                                                                'UNKNOWN')
        ]) AS t
    )
)

SELECT
    field_id,
    schema_type,
    schema_items,
    schema_custom,
    expected,
    actual
FROM (
    SELECT
        *,
        {{ jira_field_kind('field_id', 'schema_type',
                           'schema_items', 'schema_custom') }} AS actual
    FROM fixtures
)
WHERE actual != expected
