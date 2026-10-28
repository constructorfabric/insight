import json

from conftest import SOURCE, TENANT, Warehouse
from test_history import state_value


def test_an_issue_closed_without_being_picked_up_has_no_pickup_time(case: Warehouse) -> None:
    case.client.command(f"""INSERT INTO config.task_field_roles
        (tenant_id, insight_source_id, data_source, field_id, valid_from, recorded_at, role)
        VALUES ('{TENANT}', '{SOURCE}', 'youtrack', 'field-1', toDateTime64(0, 3), now64(3), 'assignee'),
               ('{TENANT}', '{SOURCE}', 'youtrack', 'field-state', toDateTime64(0, 3), now64(3), 'status')""")
    state_value(case, 'state-fixed', 'Fixed', True)
    state_value(case, 'state-open', 'Open', False)
    fixed = {'id': 'state-fixed', 'name': 'Fixed', 'isResolved': True, '$type': 'StateBundleElement'}
    state = {'id': 'issue-state', 'name': 'State', '$type': 'StateIssueCustomField',
             'projectCustomField': {'field': {'id': 'field-state'}}, 'value': fixed}
    case.issue({'id': 'user-1', 'fullName': 'Example User', '$type': 'User'}, 'SingleUserIssueCustomField', extra_fields=[state])
    case.insert('youtrack_users', {'unique_key': 'synthetic-user', 'id': 'user-1', 'email': 'user@example.com',
                                   'fullName': 'Example User', 'banned': False, 'isAnonymized': False})
    case.insert('youtrack_activities', {
        'id': 'close', 'unique_key': TENANT + '-' + SOURCE + '-close',
        '_type': 'CustomFieldActivityItem', 'timestamp': '1767312000000', 'author_id': 'user-1',
        'field_json': json.dumps({'id': 'filter-state', 'customField': {'id': 'field-state'}, 'name': 'State'}),
        'target_json': '{"id":"issue-1","idReadable":"EX-1","$type":"Issue"}',
        'removed_json': json.dumps([{'id': 'state-open', 'name': 'Open'}]), 'added_json': json.dumps([fixed]),
        'activity_json': '{}',
    })
    case.build()
    case.build('class_task_field_history class_task_users class_task_statuses class_task_issuetypes class_task_worklogs')
    case.build('task_issue_state task_status_spans task_worklog_flow task_metric_evidence')
    measures = {r['measure_key'] for r in case.rows("SELECT measure_key FROM insight.task_metric_evidence WHERE record_kind='issue'")}
    assert 'tasks_closed' in measures
    assert 'pickup_days' not in measures, f"a never-picked-up issue must not report a pickup: {measures}"
