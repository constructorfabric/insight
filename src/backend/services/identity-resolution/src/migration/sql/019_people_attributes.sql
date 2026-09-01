ALTER TABLE people
    ADD COLUMN IF NOT EXISTS attributes JSON NULL AFTER last_name;

UPDATE people
SET attributes = JSON_OBJECT()
WHERE attributes IS NULL;

ALTER TABLE people
    MODIFY COLUMN attributes JSON NOT NULL;
