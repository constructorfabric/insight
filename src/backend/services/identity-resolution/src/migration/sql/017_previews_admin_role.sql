-- Seed the `previews-admin` role (#2374): grants preview-experiment
-- management (create/delete via the previews service) without full `admin`.
-- Same stable-id pattern as the `admin` seed in 007_roles.sql. Consumers gate
-- on the NAME (the JWT roles claim carries names): the previews service's
-- MANAGE_SCOPES and the frontend's previews gate mirror 'previews-admin'.
INSERT INTO roles (role_id, name)
VALUES (UNHEX('a4d11000000040008000000000000002'), 'previews-admin')
ON DUPLICATE KEY UPDATE name = name;
