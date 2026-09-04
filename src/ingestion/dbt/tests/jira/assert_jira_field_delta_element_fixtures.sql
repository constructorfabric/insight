-- Unit test for `jira_delta_action` and `jira_delta_element`: the element-wise
-- path taken by `obj_array` fields (components, fix versions, affects versions,
-- issue links, attachments).
--
-- These are the only kinds whose items do not describe both sides, so they are
-- the only ones whose state accumulates and the only ones the reconstruction
-- has to fold. Everything else reports 'set' and is handled by
-- `jira_delta_sides`.
--
-- An item with neither side carries no information and must report 'none'.
-- Clearing a field by removing its last element is an ordinary 'remove', not a
-- degenerate case — the distinction is the point of this test.
--
-- An empty string in a fixture models a SQL NULL. Touches no table.

WITH fixtures AS (
    SELECT
        t.1 AS label,
        t.2 AS kind,
        nullIf(t.3, '') AS value_from,
        nullIf(t.4, '') AS value_from_string,
        nullIf(t.5, '') AS value_to,
        nullIf(t.6, '') AS value_to_string,
        t.7 AS exp_action,
        t.8 AS exp_element_id,
        t.9 AS exp_element_display
    FROM (
        SELECT arrayJoin([
            ('component added',    'obj_array', '','','35054','Storage',
                 'add',    '35054',  'Storage'),
            ('component removed',  'obj_array', '35054','Storage','','',
                 'remove', '35054',  'Storage'),
            ('last one removed',   'obj_array', '35055','Network','','',
                 'remove', '35055',  'Network'),
            ('version added',      'obj_array', '','','39091','release-1.0',
                 'add',    '39091',  'release-1.0'),
            -- id present without a display: the id stands in as the display
            ('id without display', 'obj_array', '','','39092','',
                 'add',    '39092',  '39092'),
            -- no information at all
            ('degenerate',         'obj_array', '','','','',
                 'none',   '',       ''),

            -- every other kind is self-describing and reports 'set'
            ('scalar is set',      'scalar',      '','a','','b', 'set', '', ''),
            -- all four sides empty is degenerate for EVERY kind, not just obj_array
            ('scalar degenerate',  'scalar',      '','','','',  'none', '', ''),
            ('string_array degen', 'string_array','','','','',  'none', '', ''),
            ('legacy degenerate',  'legacy_list', '','','','',  'none', '', ''),
            ('string_array is set','string_array','','a','','a b','set', '', ''),
            ('legacy is set',      'legacy_list', '1','A','2','B', 'set', '', ''),
            ('option_array is set','option_array','','','[1]','A',  'set', '', '')
        ]) AS t
    )
)

SELECT
    label, kind,
    exp_action, actual_action,
    exp_element_id, actual_element_id,
    exp_element_display, actual_element_display
FROM (
    SELECT
        *,
        {{ jira_delta_action('kind', 'value_from', 'value_from_string',
                             'value_to', 'value_to_string') }} AS actual_action,
        {{ jira_delta_element('value_from', 'value_from_string',
                              'value_to', 'value_to_string') }}.1 AS actual_element_id,
        {{ jira_delta_element('value_from', 'value_from_string',
                              'value_to', 'value_to_string') }}.2 AS actual_element_display
    FROM fixtures
)
WHERE actual_action != exp_action
   -- the element is only meaningful on the element-wise path
   OR (kind = 'obj_array' AND exp_action != 'none'
       AND (actual_element_id != exp_element_id
            OR actual_element_display != exp_element_display))
