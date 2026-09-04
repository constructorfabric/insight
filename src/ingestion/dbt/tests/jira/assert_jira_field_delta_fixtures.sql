-- Unit test for `jira_delta_sides`: the before/after state a single changelog
-- item describes, per field_kind.
--
-- Every case is a shape observed in a real changelog. The ones that matter most
-- are the shapes the current pipeline gets wrong:
--   * a labels-type item, whose id sides are both NULL and whose whole list sits
--     space-separated in the display sides — currently discarded entirely;
--   * a bracketed-id item, currently collapsed into one element whose id is the
--     literal string `[a, b]`;
--   * a label that itself contains a comma, which must stay one element;
--   * the four shapes Sprint mixes, all of which reduce to a full-list snapshot.
--
-- An empty string in a fixture models a SQL NULL and is converted below.
-- Touches no table.

WITH fixtures AS (
    SELECT
        t.1 AS label,
        t.2 AS kind,
        nullIf(t.3, '') AS value_from,
        nullIf(t.4, '') AS value_from_string,
        nullIf(t.5, '') AS value_to,
        nullIf(t.6, '') AS value_to_string,
        t.7 AS exp_before_ids,
        t.8 AS exp_before_displays,
        t.9 AS exp_after_ids,
        t.10 AS exp_after_displays
    FROM (
        SELECT arrayJoin([
            -- scalar, id side present and equal to the display (durations)
            ('scalar duration',   'scalar', '3600','3600','7200','7200',
                 ['3600'], ['3600'], ['7200'], ['7200']),
            -- scalar date: id side is the machine value, display side is rendered.
            -- The id side is the one that matches the issue JSON.
            ('scalar date',       'scalar', '','','2024-07-07','2024-07-07 00:00:00.0',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['2024-07-07'], ['2024-07-07']),
            -- scalar with NO id side at all (summary, description, story points)
            ('scalar no id side', 'scalar', '','Old title','','New title',
                 ['Old title'], ['Old title'], ['New title'], ['New title']),
            ('scalar cleared',    'scalar', '','5','','',
                 ['5'], ['5'], CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- duration: `null -> 0` is what logging work on an unestimated issue
            -- emits, and it means the estimate is still nothing
            ('duration to zero',  'duration', '','','0','0',
                 CAST([] AS Array(String)), CAST([] AS Array(String)),
                 CAST([] AS Array(String)), CAST([] AS Array(String))),
            ('duration consumed', 'duration', '28800','28800','0','0',
                 ['28800'], ['28800'], CAST([] AS Array(String)), CAST([] AS Array(String))),
            ('duration set',      'duration', '','','28800','28800',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['28800'], ['28800']),

            -- datetime: both sides reduced to the instant, so the value the
            -- changelog writes and the one the issue resource holds are one
            ('datetime set',      'datetime', '2026-01-05T07:00:00+0000','05/Jan/26 7:00 AM','2026-02-09T09:30:00+0000','09/Feb/26 9:30 AM',
                 ['2026-01-05 07:00:00.000'], ['2026-01-05 07:00:00.000'],
                 ['2026-02-09 09:30:00.000'], ['2026-02-09 09:30:00.000']),
            ('datetime cleared',  'datetime', '2026-01-05T07:00:00+0000','05/Jan/26 7:00 AM','','',
                 ['2026-01-05 07:00:00.000'], ['2026-01-05 07:00:00.000'],
                 CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- single objects: id and display are separate and both present
            ('obj status',        'obj',    '1','To Do','3','Done',
                 ['1'], ['To Do'], ['3'], ['Done']),
            ('option first set',  'option', '','','1001','Manual Testing',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['1001'], ['Manual Testing']),
            ('user cleared',      'user',   'acct-1','Alice Alpha','','',
                 ['acct-1'], ['Alice Alpha'], CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- issue reference: the key is both value and identifier; the numeric
            -- id the changelog carries has no counterpart in the issue JSON, and
            -- an empty id array would break the parallel-arrays invariant
            ('issue_ref',         'issue_ref', '','','996455','PROJ-1',
                 CAST([] AS Array(String)), CAST([] AS Array(String)),
                 ['PROJ-1'], ['PROJ-1']),

            -- labels-type: id sides NULL, full list space-separated in displays
            ('string_array add',  'string_array', '','build-a build-b','','build-a build-b build-c',
                 ['build-a','build-b'], ['build-a','build-b'],
                 ['build-a','build-b','build-c'], ['build-a','build-b','build-c']),
            ('string_array clear','string_array', '','build-a','','',
                 ['build-a'], ['build-a'], CAST([] AS Array(String)), CAST([] AS Array(String))),
            -- a comma inside a label is content, not a separator
            ('string_array comma','string_array', '','a,b','','a,b c',
                 ['a,b'], ['a,b'], ['a,b','c'], ['a,b','c']),

            -- Sprint: four shapes, all reducing to a full-list snapshot
            ('sprint add-shaped', 'legacy_list', '','','2156','Sprint B',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['2156'], ['Sprint B']),
            ('sprint list',       'legacy_list', '2149, 2156','Sprint A, Sprint B','2156','Sprint B',
                 ['2149','2156'], ['Sprint A','Sprint B'], ['2156'], ['Sprint B']),
            ('sprint moved',      'legacy_list', '2158','Sprint C','1699','Sprint D',
                 ['2158'], ['Sprint C'], ['1699'], ['Sprint D']),
            ('sprint removed',    'legacy_list', '2126','Sprint E','','',
                 ['2126'], ['Sprint E'], CAST([] AS Array(String)), CAST([] AS Array(String))),

            -- bracketed id list, displays joined by a bare comma
            ('option_array one',  'option_array', '','','[12070]','VAP',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['12070'], ['VAP']),
            ('option_array many', 'option_array', '','','[13027, 13028]','Alpha,Beta',
                 CAST([] AS Array(String)), CAST([] AS Array(String)),
                 ['13027','13028'], ['Alpha','Beta']),
            ('option_array grew', 'option_array', '[13027]','Alpha','[13027, 13028]','Alpha,Beta',
                 ['13027'], ['Alpha'], ['13027','13028'], ['Alpha','Beta']),
            -- a display containing a comma desynchronises the two sides; ids win
            ('option_array skew', 'option_array', '','','[1, 2]','a,b,c',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['1','2'], ['1','2']),
            -- an app field of this kind can emit DISPLAY-ONLY items with no id
            -- side at all; the display then serves as both value and identifier,
            -- as it does for a labels-type field. Falling back to the (empty) id
            -- side here would lose the event outright.
            ('option_array no id','option_array', '','','','GOAL-1',
                 CAST([] AS Array(String)), CAST([] AS Array(String)), ['GOAL-1'], ['GOAL-1']),
            ('option_array id gone','option_array', '','PREV-1','','GOAL-2',
                 ['PREV-1'], ['PREV-1'], ['GOAL-2'], ['GOAL-2'])
        ]) AS t
    )
)

SELECT
    label, kind,
    exp_before_ids, sides.1 AS actual_before_ids,
    exp_before_displays, sides.2 AS actual_before_displays,
    exp_after_ids, sides.3 AS actual_after_ids,
    exp_after_displays, sides.4 AS actual_after_displays
FROM (
    SELECT
        *,
        {{ jira_delta_sides('kind', 'value_from', 'value_from_string',
                            'value_to', 'value_to_string') }} AS sides
    FROM fixtures
)
WHERE sides.1 != exp_before_ids
   OR sides.2 != exp_before_displays
   OR sides.3 != exp_after_ids
   OR sides.4 != exp_after_displays
