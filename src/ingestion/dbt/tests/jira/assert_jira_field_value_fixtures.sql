-- Unit test for the `jira_norm_*` value normalizers.
--
-- The JSON *shapes* are Jira's; the content is synthetic. Every case is a shape
-- that occurs in a real field catalogue, including the ones that broke earlier
-- assumptions:
--   * a sprint element whose `id` is an unquoted number, not a string;
--   * a project object carrying both `name` and `key`, where the changelog
--     renders the name;
--   * a parent object carrying `key` and no `name`;
--   * an object array whose elements are users (`accountId`/`displayName`)
--     rather than options (`id`/`value`) — the same kind, a different element
--     spelling.
--
-- Touches no table.

WITH fixtures AS (
    SELECT
        t.1 AS label,
        t.2 AS kind,
        t.3 AS raw_json,
        t.4 AS expected_ids,
        t.5 AS expected_displays
    FROM (
        SELECT arrayJoin([
            -- scalar
            ('scalar string',      'scalar',       '"Example summary"',        ['Example summary'],   ['Example summary']),
            ('scalar number',      'scalar',       '8100',                     ['8100'],              ['8100']),
            ('scalar bool',        'scalar',       'true',                     ['true'],              ['true']),
            ('scalar null',        'scalar',       'null',                     CAST([] AS Array(String)), CAST([] AS Array(String))),
            ('scalar absent',      'scalar',       '',                         CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- duration: zero is not a value, on either side
            ('duration value',     'duration',     '28800',                    ['28800'],             ['28800']),
            ('duration zero',      'duration',     '0',                        CAST([] AS Array(String)), CAST([] AS Array(String))),
            ('duration null',      'duration',     'null',                     CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- datetime: the resource spells the instant with milliseconds and the
            -- changelog without, so both are reduced to one canonical rendering
            ('datetime resource',  'datetime',     '"2026-01-05T07:00:00.000+0000"', ['2026-01-05 07:00:00.000'], ['2026-01-05 07:00:00.000']),
            ('datetime changelog', 'datetime',     '"2026-01-05T07:00:00+0000"',     ['2026-01-05 07:00:00.000'], ['2026-01-05 07:00:00.000']),
            -- an offset folds to the instant, not to the same text
            ('datetime offset',    'datetime',     '"2026-01-05T10:00:00+0300"',     ['2026-01-05 07:00:00.000'], ['2026-01-05 07:00:00.000']),
            -- a spelling that does not parse stays visible as itself
            ('datetime unparsed',  'datetime',     '"not a timestamp"',              ['not a timestamp'],         ['not a timestamp']),
            ('datetime null',      'datetime',     'null',                     CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- link_array: the changelog names the LINKED ISSUE by key, so the key
            -- is the identifier here too — the link's own id appears nowhere in
            -- the changelog and is discarded
            ('link outward',      'link_array',   '[{"id":"157916","outwardIssue":{"key":"PROJ-2"},"type":{"name":"Duplicate"}}]',
                                                                              ['PROJ-2'],            ['PROJ-2']),
            ('link inward',       'link_array',   '[{"id":"158190","inwardIssue":{"key":"PROJ-3"},"type":{"name":"Blocks"}}]',
                                                                              ['PROJ-3'],            ['PROJ-3']),
            -- a subtask element IS the referenced issue: the key is at the top
            -- level, and there is no link object around it
            ('subtask element',   'link_array',   '[{"id":"1364577","key":"PROJ-4","fields":{"summary":"child"}}]',
                                                                              ['PROJ-4'],            ['PROJ-4']),
            ('link empty',        'link_array',   '[]',                       CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- single objects
            ('option',             'option',       '{"id":"1001","self":"https://example.invalid/o/1001","value":"Manual Testing"}',
                                                                              ['1001'],              ['Manual Testing']),
            ('user',              'user',          '{"accountId":"acct-0001","active":true,"displayName":"Alice Alpha","emailAddress":"alice@example.com"}',
                                                                              ['acct-0001'],         ['Alice Alpha']),
            ('obj resolution',    'obj',           '{"description":"","id":"8","name":"Done","self":"https://example.invalid/r/8"}',
                                                                              ['8'],                 ['Done']),
            -- project has BOTH name and key; the changelog renders the name
            ('obj project',       'obj',           '{"id":"16415","key":"PROJ","name":"Example Project"}',
                                                                              ['16415'],             ['Example Project']),
            -- parent has key and no name
            ('obj parent',        'obj',           '{"fields":{"summary":"Parent summary"},"id":"1364577","key":"PROJ-1"}',
                                                                              ['1364577'],           ['PROJ-1']),
            ('obj null',          'obj',           'null',                     CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- the referenced issue's key is both value and identifier: the
            -- changelog's numeric id has no counterpart in the issue JSON
            ('issue_ref',         'issue_ref',     '"PROJ-42"',                ['PROJ-42'],           ['PROJ-42']),

            -- string arrays
            ('string_array',      'string_array',  '["build-0001","build-0002"]',
                                                                              ['build-0001','build-0002'], ['build-0001','build-0002']),
            ('string_array empty','string_array',  '[]',                       CAST([] AS Array(String)), CAST([] AS Array(String))),
            ('string_array null', 'string_array',  'null',                     CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- object arrays: three element spellings, one normalizer
            ('obj_array version', 'obj_array',     '[{"archived":false,"id":"39091","name":"release-1.0","released":true}]',
                                                                              ['39091'],             ['release-1.0']),
            ('obj_array component','obj_array',    '[{"id":"35054","name":"Storage"},{"id":"35055","name":"Network"}]',
                                                                              ['35054','35055'],     ['Storage','Network']),
            ('option_array option','option_array', '[{"id":"23180","value":"Q2"},{"id":"23181","value":"Q3"}]',
                                                                              ['23180','23181'],     ['Q2','Q3']),
            ('option_array user', 'option_array',  '[{"accountId":"acct-0002","displayName":"Bob Beta"}]',
                                                                              ['acct-0002'],         ['Bob Beta']),
            ('option_array version','option_array','[{"id":"39092","name":"release-2.0"}]',
                                                                              ['39092'],             ['release-2.0']),
            ('obj_array empty',   'obj_array',     '[]',                       CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- sprint: element id is an UNQUOTED number
            ('legacy_list',       'legacy_list',   '[{"boardId":413,"id":2151,"name":"Sprint 1","state":"closed"}]',
                                                                              ['2151'],              ['Sprint 1']),

            -- long_text: the body is content-addressed into
            -- jira__task_field_text and the journal carries the hash plus a
            -- prefix (§8). The hash is PINNED on purpose — changing the address
            -- function orphans every row of the side table, so that must fail
            -- here rather than silently.
            ('long_text',         'long_text',     '{"type":"doc","version":1}',
                                                                              ['d1bd86477c8ad077de68ffe70c77fb52'],
                                                                              ['{"type":"doc","version":1}']),
            ('long_text empty',   'long_text',     'null',                     CAST([] AS Array(String)), CAST([] AS Array(String)))
        ]) AS t
    )
)

SELECT
    label,
    kind,
    raw_json,
    expected_ids,
    actual_ids,
    expected_displays,
    actual_displays
FROM (
    SELECT
        *,
        {{ jira_norm_value('kind', 'raw_json') }}.1 AS actual_ids,
        {{ jira_norm_value('kind', 'raw_json') }}.2 AS actual_displays
    FROM fixtures
)
WHERE actual_ids != expected_ids
   OR actual_displays != expected_displays
