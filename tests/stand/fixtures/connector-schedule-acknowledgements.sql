INSERT INTO connector_schedules (connector, source_id, crons, revision, updated_by, updated_at)
VALUES
    ('stand-scratch-connector-schedules', 'applied', '["0 6 * * *"]', 1, UNHEX('00000000000040008000000000000001'), '2026-01-15 12:00:00.000'),
    ('stand-scratch-connector-schedules', 'error', '["0 6 * * *"]', 1, UNHEX('00000000000040008000000000000001'), '2026-01-15 12:00:00.000')
ON DUPLICATE KEY UPDATE crons = VALUES(crons), revision = VALUES(revision), updated_by = VALUES(updated_by), updated_at = VALUES(updated_at);

INSERT INTO connector_schedule_acknowledgements (
    connector, source_id, applied_revision, applied_crons, applied_timezone, applied_origin,
    suspended, deployment_crons, deployment_origin, observed_at, attempted_revision, attempted_at, last_error
)
VALUES
    ('stand-scratch-connector-schedules', 'applied', 1, '["0 6 * * *"]', 'UTC', 'api', false, '["0 3 * * *"]', 'default', '2026-01-15 12:01:00.000', 1, '2026-01-15 12:01:00.000', NULL),
    ('stand-scratch-connector-schedules', 'error', 0, '["0 3 * * *"]', 'UTC', 'default', false, '["0 3 * * *"]', 'default', '2026-01-15 12:01:00.000', 1, '2026-01-15 12:02:00.000', 'synthetic readback failure')
ON DUPLICATE KEY UPDATE
    applied_revision = VALUES(applied_revision), applied_crons = VALUES(applied_crons),
    applied_timezone = VALUES(applied_timezone), applied_origin = VALUES(applied_origin), suspended = VALUES(suspended),
    deployment_crons = VALUES(deployment_crons), deployment_origin = VALUES(deployment_origin), observed_at = VALUES(observed_at),
    attempted_revision = VALUES(attempted_revision), attempted_at = VALUES(attempted_at), last_error = VALUES(last_error);
