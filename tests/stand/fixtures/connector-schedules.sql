INSERT INTO ingestion_history.sync_events (
    ts,
    tick_id,
    connector,
    tenant_id,
    source_id,
    event,
    schedules,
    schedule_timezone,
    schedule_origin,
    schedule_suspended
)
WITH now64(3) AS recorded_at
SELECT
    recorded_at,
    'stand-scratch-connector-schedules',
    if(event = 'connector.configured', 'stand-scratch-connector-schedules', ''),
    if(event = 'connector.configured', 'example-tenant', ''),
    if(event = 'connector.configured', 'main', ''),
    event,
    if(event = 'connector.configured', ['0 3 * * *'], []),
    if(event = 'connector.configured', 'UTC', ''),
    if(event = 'connector.configured', 'default', ''),
    false
FROM (
    SELECT arrayJoin(['connector.configured', 'sweep.completed']) AS event
)
WHERE NOT EXISTS (
    SELECT 1
    FROM ingestion_history.sync_events
    WHERE tick_id = 'stand-scratch-connector-schedules'
);
