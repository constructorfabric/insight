"""Runs actual dbt models against an explicitly configured disposable warehouse."""
from __future__ import annotations

import json
import os
import re
from collections.abc import Iterator, Mapping, Sequence
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

import clickhouse_connect
import pytest
import yaml
from clickhouse_connect.driver.client import Client
from dbt.cli.main import dbtRunner

INGESTION = Path(__file__).resolve().parents[4]
DBT = INGESTION / 'dbt'
SOURCE = 'synthetic-youtrack'
TENANT = 'synthetic-tenant'


class Warehouse:
    def __init__(self, client: Client, profile: Path) -> None:
        self.client = client
        self.profile = profile
        self.tables = set(re.findall(r'CREATE TABLE IF NOT EXISTS bronze_youtrack\.(\w+)',
                                     (INGESTION / 'scripts/connectors-ddl/youtrack.sql').read_text()))

    def build(self, selector: str = 'tag:youtrack', full_refresh: bool = False) -> None:
        messages = []
        result = dbtRunner(callbacks=[lambda e: messages.append(e.info.msg)]).invoke([
            'run', '--project-dir', str(DBT), '--profiles-dir', str(self.profile),
            '--select', selector, '--quiet', *(['--full-refresh'] if full_refresh else []),
        ])
        assert result.success, '\n'.join(messages)[-10000:]

    def insert(self, table: str, row: Mapping[str, object], observed: str = '2026-01-10T00:00:00') -> None:
        assert table in self.tables
        values = {
            '_airbyte_raw_id': str(uuid4()),
            '_airbyte_extracted_at': datetime.fromisoformat(observed).replace(tzinfo=timezone.utc),
            '_airbyte_meta': '{}', '_airbyte_generation_id': 0,
            'tenant_id': TENANT, 'source_id': SOURCE,
            **row,
        }
        self.client.insert('bronze_youtrack.' + table, [list(values.values())], column_names=list(values))

    def issue(self, value: object, field_type: str = 'MultiEnumIssueCustomField', observed: str = '2026-01-10T00:00:00',
              links: Sequence[Mapping[str, object]] = (), extra_fields: Sequence[Mapping[str, object]] = (),
              id_readable: str = 'EX-1', project_id: str = 'project-1') -> None:
        cf = {'id': 'issue-field', 'name': 'Synthetic field', '$type': field_type,
              'projectCustomField': {'id': 'project-field', 'field': {'id': 'field-1'}}, 'value': value}
        self.insert('youtrack_issues', {
            'id': 'issue-1', 'idReadable': id_readable, 'created': 1767225600000,
            'unique_key': TENANT + '-' + SOURCE + '-issue-1', 'reporter_id': 'user-1',
            'project_id': project_id, 'custom_fields_json': json.dumps([cf, *extra_fields]),
            'issue_json': json.dumps({'summary': 'Synthetic issue', 'description': '', 'links': list(links)}),
        }, observed)

    def event(self, event_id: str, before: object, after: object, at: int = 1767312000000,
              observed: str = '2026-01-10T00:00:00') -> None:
        self.insert('youtrack_activities', {
            'id': event_id, 'unique_key': TENANT + '-' + SOURCE + '-' + event_id,
            '_type': 'CustomFieldActivityItem', 'timestamp': str(at), 'author_id': 'user-1',
            'field_json': json.dumps({'id': 'filter-1', 'customField': {'id': 'field-1'}, 'name': 'Synthetic field'}),
            'target_json': json.dumps({'id': 'issue-1', 'idReadable': 'EX-1', '$type': 'Issue'}),
            'added_json': json.dumps(after), 'removed_json': json.dumps(before), 'activity_json': '{}',
        }, observed)

    def rows(self, sql: str) -> Iterator[dict[str, object]]:
        return self.client.query(sql).named_results()


@pytest.fixture(scope='session')
def warehouse(tmp_path_factory: pytest.TempPathFactory) -> Iterator[Warehouse]:
    if os.environ['YOUTRACK_TEST_DISPOSABLE'] != 'yes':
        raise RuntimeError('Use only a disposable ClickHouse: this suite resets its YouTrack tables')
    output = {'type': 'clickhouse', 'host': os.environ['CLICKHOUSE_HOST'],
              'port': int(os.environ['CLICKHOUSE_HTTP_PORT']), 'user': os.environ['CLICKHOUSE_USER'],
              'password': os.environ['CLICKHOUSE_PASSWORD'], 'schema': 'default', 'threads': 2,
              'engine': 'ReplacingMergeTree(_version)', 'settings': {'allow_nullable_key': 1}}
    profile = tmp_path_factory.mktemp('profile')
    (profile / 'profiles.yml').write_text(yaml.safe_dump({'ingestion': {'target': 'test', 'outputs': {'test': output}}}))
    client = clickhouse_connect.get_client(host=output['host'], port=output['port'], username=output['user'], password=output['password'])
    for stmt in (INGESTION / 'scripts/connectors-ddl/youtrack.sql').read_text().split(';'):
        if stmt.strip():
            client.command(stmt)
    client.command('CREATE DATABASE IF NOT EXISTS staging')
    silver_ddl = (INGESTION / 'scripts/connectors-ddl/silver.sql').read_text()
    for stmt in silver_ddl.split(';'):
        match = re.search(r'CREATE TABLE IF NOT EXISTS silver\.(class_task_\w+)', stmt)
        if not match:
            continue
        name = match[1]
        producer = ('jira__field_history_derived' if name == 'class_task_field_history'
                    else 'github__task_links' if name == 'class_task_links'
                    else 'jira__' + name[6:])
        client.command(stmt.replace('silver.' + name, 'staging.' + producer))
    yield Warehouse(client, profile)
    client.close()


@pytest.fixture
def case(warehouse: Warehouse) -> Warehouse:
    for table in sorted(warehouse.tables):
        warehouse.client.command('TRUNCATE TABLE bronze_youtrack.' + table)
    for name in ['youtrack__field_observations', 'youtrack__issue_observations', 'youtrack__users_snapshot', 'youtrack__comment_observations', 'youtrack__work_item_observations']:
        warehouse.client.command('TRUNCATE TABLE IF EXISTS staging.' + name)
    for table in ['task_field_roles', 'task_value_map', 'field_value_map', 'field_value_defaults']:
        warehouse.client.command('TRUNCATE TABLE IF EXISTS config.' + table)
    return warehouse
