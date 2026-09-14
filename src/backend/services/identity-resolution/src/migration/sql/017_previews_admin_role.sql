-- Seed the `previews-admin` role, same stable-id pattern as `admin` in 007.
-- Consumers gate on the NAME (the JWT roles claim carries names).
INSERT INTO roles (role_id, name)
VALUES (UNHEX('a4d11000000040008000000000000002'), 'previews-admin')
ON DUPLICATE KEY UPDATE name = name;
