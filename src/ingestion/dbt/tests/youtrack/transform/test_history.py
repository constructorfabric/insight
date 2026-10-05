import json

from conftest import SOURCE, TENANT, Warehouse


def test_project_move_preserves_relationships_and_keys(case: Warehouse) -> None:
    link = {'linkType': {'id': 'depends'}, 'direction': 'OUTWARD',
            'issues': [{'id': 'issue-2', 'idReadable': 'EX-2'}]}
    case.issue([], links=[link])
    case.event('event-before-move', [], ['a'])
    case.insert('youtrack_activities', {
        'id': 'project-move', 'unique_key': TENANT + '-' + SOURCE + '-project-move',
        '_type': 'ProjectActivityItem', 'timestamp': '1768089600000', 'author_id': 'user-1',
        'field_json': '{}',
        'target_json': '{"id":"issue-1","idReadable":"NEW-7","$type":"Issue"}',
        'removed_json': '{"id":"key-old","project":{"id":"project-1","name":"Example One"}}',
        'added_json': '{"id":"key-new","project":{"id":"project-2","name":"Example Two"}}',
        'activity_json': '{"targetMember":"project"}',
    }, '2026-01-11T00:00:00')
    case.insert('youtrack_comments', {'unique_key': 'synthetic-comment', 'id': 'comment-1',
        'issue_id': 'issue-1', 'issue_id_readable': 'EX-1', 'created': 1767312000000,
        'deleted': False, 'author_id': 'user-1', 'text': 'Synthetic comment'})
    case.insert('youtrack_work_items', {'unique_key': 'synthetic-work', 'id': 'work-1',
        'issue_id': 'issue-1', 'author_id': 'user-1', 'date': 1767312000000,
        'work_item_json': '{"duration":{"minutes":15}}'})
    classes = 'class_task_field_history class_task_comments class_task_worklogs class_task_links'
    case.build()
    case.build(classes)
    link_before = list(case.rows("SELECT unique_key,issue_id,target_id,valid_from FROM silver.class_task_links FINAL WHERE data_source='youtrack'"))
    history_before = list(case.rows("SELECT unique_key FROM silver.class_task_field_history FINAL WHERE event_id='event-before-move'"))
    link['issues'][0]['idReadable'] = 'OTHER-8'
    case.issue([], links=[link], observed='2026-01-11T00:00:00',
               id_readable='NEW-7', project_id='project-2')
    case.build()
    case.build(classes)
    assert list(case.rows("SELECT unique_key,issue_id,target_id,valid_from FROM silver.class_task_links FINAL WHERE data_source='youtrack'")) == link_before
    assert list(case.rows("SELECT unique_key FROM silver.class_task_field_history FINAL WHERE event_id='event-before-move'")) == history_before
    assert list(case.rows("SELECT value_ids FROM silver.class_task_field_history FINAL WHERE event_id='project-move'")) == [
        {'value_ids': ['project-2']}]
    assert list(case.rows("SELECT id_readable,target_readable,valid_to FROM silver.class_task_links FINAL WHERE data_source='youtrack'")) == [
        {'id_readable': 'NEW-7', 'target_readable': 'OTHER-8', 'valid_to': None}]
    for name in ['comments', 'worklogs']:
        rows = list(case.rows(f"""SELECT w.issue_id AS issue_id,i.id_readable AS id_readable
            FROM silver.class_task_{name} AS w FINAL
            INNER JOIN staging.youtrack__issues AS i
                ON i.insight_source_id=w.insight_source_id AND i.issue_id=w.issue_id
            WHERE w.data_source='youtrack'"""))
        assert rows == [{'issue_id': 'issue-1', 'id_readable': 'NEW-7'}]
    case.build(classes)
    assert list(case.rows("SELECT unique_key,issue_id,target_id,valid_from FROM silver.class_task_links FINAL WHERE data_source='youtrack'")) == link_before


def value(key: str) -> dict[str, str]:
    return {'id': key, 'name': key, '$type': 'EnumBundleElement'}


def test_cardinality_change_does_not_reinterpret_history(case: Warehouse) -> None:
    case.issue([value('a'), value('b')])
    case.event('opaque-z', [value('old')], [value('a')])
    case.event('opaque-a', [], [value('b')], at=1767398400000)
    case.build()
    rows = list(case.rows("SELECT value_ids, field_cardinality FROM staging.youtrack__task_field_history WHERE event_kind='changelog' ORDER BY event_order"))
    assert rows == [{'value_ids': ['a'], 'field_cardinality': 'unknown'}, {'value_ids': ['a', 'b'], 'field_cardinality': 'unknown'}]
    case.issue(value('b'), 'SingleEnumIssueCustomField', '2026-01-11T00:00:00')
    case.event('opaque-next', [value('a')], [], at=1768089600000, observed='2026-01-11T00:00:00')
    case.build()
    rows = list(case.rows("SELECT value_ids, field_cardinality FROM staging.youtrack__task_field_history WHERE field_id='field-1' ORDER BY event_order"))
    assert rows[-1]['value_ids'] == ['b']
    assert rows[0] == {'value_ids': ['a'], 'field_cardinality': 'unknown'}
    observed = list(case.rows("SELECT field_cardinality FROM staging.youtrack__issue_field_snapshot WHERE field_id='field-1' ORDER BY observed_at"))
    assert [r['field_cardinality'] for r in observed] == ['multi', 'single']


def test_opaque_ids_follow_same_instant_value_chain(case: Warehouse) -> None:
    case.issue([value('c')])
    case.event('z-first', [value('a')], [value('b')])
    case.event('a-last', [value('b')], [value('c')])
    case.build()
    rows = list(case.rows("SELECT event_id,value_ids,event_order FROM staging.youtrack__task_field_history WHERE event_kind='changelog' ORDER BY event_order"))
    assert [r['event_id'] for r in rows] == ['z-first', 'a-last']
    assert [r['value_ids'] for r in rows] == [['b'], ['c']]
    assert rows[1]['event_order'] - rows[0]['event_order'] == 1


def test_partial_history_keeps_snapshot_at_observation_time(case: Warehouse) -> None:
    case.issue([value('a')])
    case.build()
    initial = list(case.rows("SELECT field_id FROM staging.youtrack__task_field_history WHERE event_kind='synthetic_initial'"))
    assert initial == [{'field_id': 'created'}]
    observed = list(case.rows("SELECT event_at,value_ids FROM staging.youtrack__task_field_history WHERE field_id='field-1'"))
    assert len(observed) == 1 and str(observed[0]['event_at']).startswith('2026-01-10')
    case.issue([value('a')], observed='2026-01-11T00:00:00')
    case.build()
    assert len(list(case.rows("SELECT * FROM staging.youtrack__task_field_history WHERE field_id='field-1'"))) == 1


def test_null_empty_and_comma_values_are_not_split(case: Warehouse) -> None:
    case.issue([])
    case.event('e1', None, ['a,b', 'c'])
    case.event('e2', ['a,b'], [], at=1767398400000)
    case.event('e3', ['c'], None, at=1767484800000)
    case.build()
    rows = list(case.rows("SELECT value_ids FROM staging.youtrack__task_field_history WHERE event_kind='changelog' ORDER BY event_order"))
    assert [r['value_ids'] for r in rows] == [['a,b', 'c'], ['c'], []]


def test_repeated_build_and_late_event_replace_silver(case: Warehouse) -> None:
    case.issue([value('c')])
    case.event('e2', [value('b')], [value('c')], at=1767398400000)
    case.build()
    case.build('class_task_field_history')
    case.event('e1', [value('a')], [value('b')], observed='2026-01-11T00:00:00')
    case.build()
    case.build('class_task_field_history')
    case.build('class_task_field_history')
    rows = list(case.rows("SELECT event_id,value_ids FROM silver.class_task_field_history FINAL WHERE data_source='youtrack' AND event_kind='changelog' ORDER BY event_order"))
    assert rows == [{'event_id': 'e1', 'value_ids': ['b']}, {'event_id': 'e2', 'value_ids': ['c']}]
    case.client.command("DELETE FROM bronze_youtrack.youtrack_activities WHERE id='e2' SETTINGS mutations_sync=2")
    case.build()
    case.build('class_task_field_history')
    assert list(case.rows("SELECT event_id FROM silver.class_task_field_history FINAL WHERE data_source='youtrack' AND event_kind='changelog'")) == [{'event_id': 'e1'}]


def test_link_removal_and_readdition_are_separate_intervals(case: Warehouse) -> None:
    link = {'linkType': {'id': 'depends'}, 'direction': 'OUTWARD', 'issues': [{'id': 'issue-2', 'idReadable': 'EX-2'}]}
    case.issue([], links=[link])
    case.build()
    case.issue([], observed='2026-01-11T00:00:00', links=[])
    case.build()
    case.issue([], observed='2026-01-12T00:00:00', links=[link])
    case.build()
    rows = list(case.rows("SELECT valid_from,valid_to,valid_from_known,evidence FROM staging.youtrack__task_links ORDER BY valid_from"))
    assert len(rows) == 2
    assert str(rows[0]['valid_to']).startswith('2026-01-11')
    assert rows[1]['valid_to'] is None
    assert all(r['valid_from_known'] == 0 and r['evidence'] == 'observation' for r in rows)


def test_link_preview_alongside_the_full_set_still_yields_intervals(case: Warehouse) -> None:
    target = {'id': 'issue-2', 'idReadable': 'EX-2'}
    case.issue([], links=[{'linkType': {'id': 'depends'}, 'direction': 'OUTWARD', 'issues': [target], 'trimmedIssues': [target]}])
    case.build()
    assert list(case.rows("SELECT target_id FROM staging.youtrack__task_links")) == [{'target_id': 'issue-2'}]


def test_trimmed_link_set_does_not_close_an_interval(case: Warehouse) -> None:
    target = {'id': 'issue-2', 'idReadable': 'EX-2'}
    case.issue([], links=[{'linkType': {'id': 'depends'}, 'direction': 'OUTWARD', 'issues': [target]}])
    case.build()
    trimmed = {'linkType': {'id': 'depends'}, 'direction': 'OUTWARD', 'issues': [], 'trimmedIssues': [target]}
    case.issue([], observed='2026-01-11T00:00:00', links=[trimmed])
    case.build()
    assert list(case.rows("SELECT valid_to FROM staging.youtrack__task_links")) == [{'valid_to': None}]


def test_all_task_classes_match_shared_contract_and_units(case: Warehouse) -> None:
    case.issue([])
    case.insert('youtrack_users', {'unique_key': 'synthetic-user', 'id': 'user-1', 'email': 'user@example.com', 'fullName': 'Example User', 'login': 'example.user', 'banned': False})
    case.insert('youtrack_comments', {'unique_key': 'synthetic-comment', 'id': 'comment-1', 'issue_id': 'issue-1', 'issue_id_readable': 'EX-1', 'created': 1767312000000, 'deleted': True, 'author_id': 'user-1', 'text': 'Synthetic comment'})
    case.insert('youtrack_work_items', {'unique_key': 'synthetic-work', 'id': 'work-1', 'issue_id': 'issue-1', 'author_id': 'user-1', 'date': 1767312000000, 'created': 1767312000000, 'work_item_json': '{"duration":{"minutes":15},"text":"Synthetic work"}'})
    case.build()
    classes = ' '.join('class_task_' + name for name in ['field_history', 'field_metadata', 'statuses', 'issuetypes', 'users', 'projects', 'sprints', 'comments', 'worklogs', 'links'])
    case.build(classes)
    assert list(case.rows("SELECT duration_seconds,is_deleted FROM silver.class_task_worklogs FINAL WHERE data_source='youtrack'")) == [{'duration_seconds': 900.0, 'is_deleted': None}]
    assert list(case.rows("SELECT is_deleted FROM silver.class_task_comments FINAL WHERE data_source='youtrack'")) == [{'is_deleted': 1}]
    assert list(case.rows("SELECT value FROM staging.youtrack__identity_inputs WHERE value_type='email'"))[0]['value'] == 'user@example.com'
    assert list(case.rows("SELECT event_kind FROM silver.class_task_field_history FINAL WHERE data_source='youtrack' AND field_id='comment'")) == [{'event_kind': 'lifecycle'}]


def test_retired_field_is_observed_not_an_invented_change(case: Warehouse) -> None:
    import json
    case.issue([value('a')])
    case.build()
    case.insert('youtrack_issues', {'id': 'issue-1', 'idReadable': 'EX-1', 'created': 1767225600000,
        'unique_key': TENANT + '-' + SOURCE + '-issue-1', 'reporter_id': 'user-1',
        'project_id': 'project-1', 'custom_fields_json': '[]',
        'issue_json': json.dumps({'summary': 'Synthetic issue', 'description': '', 'links': []})}, '2026-01-11T00:00:00')
    case.build()
    rows = list(case.rows("SELECT event_kind,value_ids,field_cardinality FROM staging.youtrack__task_field_history WHERE field_id='field-1' ORDER BY event_order"))
    assert rows == [{'event_kind': 'snapshot_diff', 'value_ids': ['a'], 'field_cardinality': 'multi'},
                    {'event_kind': 'retired_field', 'value_ids': [], 'field_cardinality': 'multi'}]


def test_gold_uses_bindings_and_does_not_choose_first_of_many_assignees(case: Warehouse) -> None:
    deadline = {'id': 'deadline', 'name': 'Deadline', '$type': 'SimpleIssueCustomField',
                'projectCustomField': {'field': {'id': 'due-1'}}, 'value': 1768089600000}
    case.issue({'id': 'user-1', 'fullName': 'Example User', '$type': 'User'}, 'SingleUserIssueCustomField', extra_fields=[deadline])
    case.insert('youtrack_users', {'unique_key': 'synthetic-user', 'id': 'user-1', 'email': 'user@example.com', 'fullName': 'Example User', 'banned': False, 'isAnonymized': False})
    case.build()
    case.client.command("INSERT INTO config.task_field_roles (tenant_id, insight_source_id, data_source, field_id, valid_from, recorded_at, role) VALUES ('synthetic-tenant','synthetic-youtrack','youtrack','field-1','2026-01-01','2026-01-01','assignee'), ('synthetic-tenant','synthetic-youtrack','youtrack','due-1','2026-01-01','2026-01-01','duedate')")
    case.build('class_task_field_history class_task_users class_task_statuses class_task_issuetypes')
    case.build('task_issue_state task_status_spans')
    rows = list(case.rows('SELECT issue_id,entity_id,toString(due_date) AS due_date FROM insight.task_issue_state'))
    assert rows == [{'issue_id': 'issue-1', 'entity_id': 'user@example.com', 'due_date': '2026-01-11'}]
    case.issue([{'id': 'user-1', '$type': 'User'}, {'id': 'user-2', '$type': 'User'}], 'MultiUserIssueCustomField', '2026-01-11T00:00:00')
    case.build()
    case.build('class_task_field_history')
    case.build('task_issue_state task_status_spans')
    assert list(case.rows('SELECT issue_id FROM insight.task_issue_state')) == []


def test_metadata_observations_do_not_claim_exact_change_time(case: Warehouse) -> None:
    case.issue([])
    case.insert('youtrack_custom_fields', {'id': 'field-1', 'unique_key': 'synthetic-field', 'name': 'Synthetic field', 'field_type_id': 'enum[1]'})
    case.build()
    case.insert('youtrack_custom_fields', {'id': 'field-1', 'unique_key': 'synthetic-field', 'name': 'Synthetic field', 'field_type_id': 'enum[*]'}, '2026-01-12T00:00:00')
    case.build()
    rows = list(case.rows('SELECT observed_cardinality,previously_observed_type,previously_observed_at FROM staging.youtrack__field_type_history ORDER BY observed_at'))
    assert rows[0]['observed_cardinality'] == 'single' and rows[0]['previously_observed_at'] is None
    assert rows[1]['observed_cardinality'] == 'multi' and rows[1]['previously_observed_type'] == 'enum[1]'
    assert str(rows[1]['previously_observed_at']).startswith('2026-01-10')
    case.build(full_refresh=True)
    assert len(list(case.rows('SELECT * FROM staging.youtrack__field_type_history'))) == 2


def test_comment_lifecycle_survives_bronze_replacement(case: Warehouse) -> None:
    case.issue([])
    row = {'unique_key': 'synthetic-comment', 'id': 'comment-1', 'issue_id': 'issue-1',
           'issue_id_readable': 'EX-1', 'created': 1767312000000, 'deleted': False,
           'author_id': 'user-1', 'text': 'Synthetic comment'}
    case.insert('youtrack_comments', row)
    case.build()
    case.insert('youtrack_comments', {**row, 'deleted': True}, '2026-01-11T00:00:00')
    case.build()
    rows = list(case.rows("SELECT delta_action FROM staging.youtrack__task_field_history WHERE field_id='comment' ORDER BY event_order"))
    assert [r['delta_action'] for r in rows] == ['set', 'remove']


def activity_issue(case: Warehouse, observed: str, id_readable: str) -> None:
    # The same record shape as youtrack_issues, from the activity-feed stream.
    case.insert('youtrack_activity_issues', {
        'id': 'issue-1', 'idReadable': id_readable, 'created': 1767225600000,
        'unique_key': TENANT + '-' + SOURCE + '-issue-1', 'reporter_id': 'user-1',
        'project_id': 'project-1', 'custom_fields_json': json.dumps([{
            'id': 'issue-field', 'name': 'Synthetic field', '$type': 'MultiEnumIssueCustomField',
            'projectCustomField': {'id': 'project-field', 'field': {'id': 'field-1'}}, 'value': [value('a')]}]),
        'issue_json': json.dumps({'summary': 'Synthetic issue', 'description': '', 'links': []}),
    }, observed)


def test_activity_issue_snapshot_backs_history_the_search_missed(case: Warehouse) -> None:
    # A change that left `updated` untouched: the search never returned the
    # issue, only the activity-feed stream did.
    activity_issue(case, '2026-01-10T00:00:00', 'EX-1')
    case.event('silent-change', [], [value('a')])
    case.build()
    assert list(case.rows("SELECT id_readable, value_ids FROM staging.youtrack__task_field_history WHERE event_id='silent-change'")) == [
        {'id_readable': 'EX-1', 'value_ids': ['a']}]
    # Both streams hold the issue: the later observation is the snapshot.
    case.issue([value('a')], observed='2026-01-11T00:00:00', id_readable='EX-9')
    activity_issue(case, '2026-01-12T00:00:00', 'EX-12')
    case.build()
    assert list(case.rows("SELECT id_readable FROM staging.youtrack__issues")) == [{'id_readable': 'EX-12'}]


def test_search_snapshot_wins_a_tie_with_the_activity_stream(case: Warehouse) -> None:
    case.issue([], id_readable='EX-SEARCH')
    activity_issue(case, '2026-01-10T00:00:00', 'EX-ACTIVITY')
    case.build()
    assert list(case.rows("SELECT id_readable FROM staging.youtrack__issues")) == [{'id_readable': 'EX-SEARCH'}]


def test_issue_observation_watermark_is_scoped_to_the_source(case: Warehouse) -> None:
    case.issue([], observed='2026-01-12T00:00:00')
    case.build()
    # A second source whose sync ran earlier must not fall below the first one's watermark.
    case.insert('youtrack_issues', {
        'id': 'issue-1', 'idReadable': 'OT-1', 'created': 1767225600000,
        'unique_key': TENANT + '-other-source-issue-1', 'source_id': 'other-source',
        'project_id': 'project-1', 'custom_fields_json': '[]', 'issue_json': '{}',
    }, '2026-01-11T00:00:00')
    case.issue([], observed='2026-01-13T00:00:00')
    case.build()
    rows = case.rows("SELECT insight_source_id, toString(observed_at) AS observed_at FROM staging.youtrack__issue_observations FINAL"
                     " ORDER BY insight_source_id, observed_at")
    assert list(rows) == [
        {'insight_source_id': 'other-source', 'observed_at': '2026-01-11 00:00:00.000'},
        {'insight_source_id': SOURCE, 'observed_at': '2026-01-12 00:00:00.000'},
        {'insight_source_id': SOURCE, 'observed_at': '2026-01-13 00:00:00.000'},
    ]


def test_single_value_field_is_replaced_not_merged(case: Warehouse) -> None:
    # The replaced text differs from the snapshot's by a trailing space; an
    # id-merge keeps both and the summary would hold two values.
    case.issue([])
    case.insert('youtrack_activities', {
        'id': 'retitle', 'unique_key': TENANT + '-' + SOURCE + '-retitle',
        '_type': 'TextMarkupActivityItem', 'timestamp': '1767312000000', 'author_id': 'user-1',
        'field_json': '{}', 'target_json': '{"id":"issue-1","idReadable":"EX-1","$type":"Issue"}',
        'removed_json': json.dumps('Draft title'), 'added_json': json.dumps('Synthetic issue '),
        'activity_json': '{"targetMember":"summary"}',
    })
    case.build()
    rows = list(case.rows("SELECT field_name, field_cardinality, value_ids FROM staging.youtrack__task_field_history WHERE event_id='retitle'"))
    assert rows == [{'field_name': 'Summary', 'field_cardinality': 'single', 'value_ids': ['Synthetic issue ']}]


def test_custom_field_cardinality_is_the_type_observed_at_the_event(case: Warehouse) -> None:
    case.issue([value('a')])
    case.insert('youtrack_custom_fields', {'id': 'field-1', 'unique_key': 'synthetic-field', 'name': 'Synthetic field', 'field_type_id': 'enum[*]'})
    case.event('before-first-observation', [], [value('a')])
    case.build()
    rows = list(case.rows("SELECT field_cardinality FROM staging.youtrack__task_field_history WHERE event_id='before-first-observation'"))
    assert rows == [{'field_cardinality': 'multi'}]
    # The type changes later: the earlier event keeps what it was read as.
    case.insert('youtrack_custom_fields', {'id': 'field-1', 'unique_key': 'synthetic-field', 'name': 'Synthetic field', 'field_type_id': 'enum[1]'}, '2026-01-12T00:00:00')
    case.event('after-type-change', [value('a')], [value('b')], at=1768262400000, observed='2026-01-13T00:00:00')
    case.build()
    rows = list(case.rows("SELECT event_id, field_cardinality, value_ids FROM staging.youtrack__task_field_history WHERE event_kind='changelog' AND field_id='field-1' ORDER BY event_order"))
    assert rows == [{'event_id': 'before-first-observation', 'field_cardinality': 'multi', 'value_ids': ['a']},
                    {'event_id': 'after-type-change', 'field_cardinality': 'single', 'value_ids': ['b']}]


def board(case: Warehouse, board_id: str, project_id: str, sprint_id: str, sprint_name: str) -> None:
    case.insert('youtrack_agiles', {'id': board_id, 'name': board_id, 'unique_key': 'synthetic-' + board_id,
        'agile_json': json.dumps({'id': board_id, 'projects': [{'id': project_id}], 'sprintsSettings': {
            'isExplicit': False, 'disableSprints': False, 'sprintSyncField': {'id': 'field-1', 'name': 'Sprints'}}})})
    case.insert('youtrack_sprints', {'id': sprint_id, 'name': sprint_name, 'agile_id': board_id,
        'unique_key': 'synthetic-' + sprint_id, 'sprint_json': json.dumps({'id': sprint_id, 'name': sprint_name})})


def test_sync_field_membership_reads_as_the_sprints_property(case: Warehouse) -> None:
    # The board manages sprints through field-1, so membership changes arrive
    # only as that field's change; they restate as `sprints`, in sprint ids,
    # for the boards the issue's project is on.
    board(case, 'board-1', 'project-1', 'sprint-7', 'Sprint 7')
    board(case, 'board-2', 'project-2', 'sprint-70', 'Sprint 7')
    case.issue([])
    sprint = {'id': 'version-7', 'name': 'Sprint 7', '$type': 'VersionBundleElement'}
    case.event('into-sprint', [], [sprint])
    case.build()
    rows = list(case.rows("SELECT field_id, field_name, field_cardinality, value_ids FROM staging.youtrack__task_field_history WHERE event_id='into-sprint' ORDER BY field_id"))
    assert rows == [
        {'field_id': 'field-1', 'field_name': 'Synthetic field', 'field_cardinality': 'unknown', 'value_ids': ['version-7']},
        {'field_id': 'sprints', 'field_name': 'Sprints', 'field_cardinality': 'multi', 'value_ids': ['sprint-7']}]


def test_sprint_activity_is_named_as_the_property_not_the_board(case: Warehouse) -> None:
    case.issue([])
    case.insert('youtrack_activities', {
        'id': 'board-assignment', 'unique_key': TENANT + '-' + SOURCE + '-board-assignment',
        '_type': 'SprintActivityItem', 'timestamp': '1767312000000', 'author_id': 'user-1',
        'field_json': '{"name":"Board Example"}',
        'target_json': '{"id":"issue-1","idReadable":"EX-1","$type":"Issue"}',
        'removed_json': '[]', 'added_json': '[{"id":"sprint-7","name":"Sprint 7","$type":"Sprint"}]',
        'activity_json': '{}',
    })
    case.build()
    assert list(case.rows("SELECT field_id, field_name, value_ids FROM staging.youtrack__task_field_history WHERE event_id='board-assignment'")) == [
        {'field_id': 'sprints', 'field_name': 'Sprints', 'value_ids': ['sprint-7']}]


def test_rereading_an_unchanged_record_adds_no_lifecycle_event(case: Warehouse) -> None:
    # Work items are re-read on every sync and a comment whenever its issue
    # changes: each state is one event, dated by its first observation.
    case.issue([])
    work = {'unique_key': 'synthetic-work', 'id': 'work-1', 'issue_id': 'issue-1', 'author_id': 'user-1',
            'date': 1767312000000, 'created': 1767312000000, 'work_item_json': '{"duration":{"minutes":15}}'}
    comment = {'unique_key': 'synthetic-comment', 'id': 'comment-1', 'issue_id': 'issue-1', 'issue_id_readable': 'EX-1',
               'created': 1767312000000, 'deleted': False, 'author_id': 'user-1', 'text': 'Synthetic comment'}
    case.insert('youtrack_work_items', work)
    case.insert('youtrack_comments', comment)
    case.build()
    case.insert('youtrack_work_items', work, '2026-01-11T00:00:00')
    case.insert('youtrack_comments', comment, '2026-01-11T00:00:00')
    case.insert('youtrack_comments', {**comment, 'deleted': True}, '2026-01-12T00:00:00')
    case.build()
    case.insert('youtrack_comments', {**comment, 'deleted': True}, '2026-01-13T00:00:00')
    case.build()
    rows = list(case.rows("SELECT field_id, delta_action, toString(event_at) AS event_at FROM staging.youtrack__task_field_history WHERE field_id IN ('comment', 'worklog') ORDER BY field_id, event_order"))
    assert rows == [
        {'field_id': 'comment', 'delta_action': 'set', 'event_at': '2026-01-02 00:00:00.000'},
        {'field_id': 'comment', 'delta_action': 'remove', 'event_at': '2026-01-12 00:00:00.000'},
        {'field_id': 'worklog', 'delta_action': 'set', 'event_at': '2026-01-02 00:00:00.000'}]


def state_value(case: Warehouse, value_id: str, name: str, resolved: bool) -> None:
    case.insert('youtrack_field_values', {
        'unique_key': 'synthetic-state-' + value_id, 'project_id': 'project-1', 'field_id': 'field-state',
        'field_type_id': 'state[1]', 'bundle_id': 'bundle-state',
        'values_json': json.dumps({'collection': 'values', 'value': {
            'id': value_id, 'name': name, 'isResolved': resolved, '$type': 'StateBundleElement'}})})


def test_status_category_is_resolved_from_youtrack_and_split_by_the_operator(case: Warehouse) -> None:
    # `done` is YouTrack's own isResolved; the open split is the operator's, and
    # an open value nobody decided stays `undefined`.
    case.client.command(f"""INSERT INTO config.task_field_roles
        (tenant_id, insight_source_id, data_source, field_id, valid_from, recorded_at, role)
        VALUES ('{TENANT}', '{SOURCE}', 'youtrack', 'field-state', toDateTime64(0, 3), now64(3), 'status')""")
    case.client.command(f"""INSERT INTO config.task_value_map
        (tenant_id, insight_source_id, data_source, field_id, value_id, valid_from, recorded_at,
         canonical_value, value_display)
        VALUES ('{TENANT}', '{SOURCE}', 'youtrack', 'field-state', 'state-progress', toDateTime64(0, 3), now64(3),
                'in_progress', 'In Progress')""")
    state_value(case, 'state-fixed', 'Fixed', True)
    state_value(case, 'state-progress', 'In Progress', False)
    state_value(case, 'state-open', 'Open', False)
    case.issue([])
    case.build()
    rows = list(case.rows("SELECT status_id, status_name, status_category FROM staging.youtrack__task_statuses ORDER BY status_id"))
    assert rows == [
        {'status_id': 'state-fixed', 'status_name': 'Fixed', 'status_category': 'done'},
        {'status_id': 'state-open', 'status_name': 'Open', 'status_category': 'undefined'},
        {'status_id': 'state-progress', 'status_name': 'In Progress', 'status_category': 'in_progress'}]
