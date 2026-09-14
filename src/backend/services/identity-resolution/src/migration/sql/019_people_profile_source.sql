ALTER TABLE people
    ADD COLUMN IF NOT EXISTS profile_source_type VARCHAR(30) NULL,
    ADD COLUMN IF NOT EXISTS profile_source_id BINARY(16) NULL,
    ADD COLUMN IF NOT EXISTS profile_account_id VARCHAR(320) NULL,
    ADD CONSTRAINT IF NOT EXISTS ck_people_profile_source CHECK (
        (profile_source_type IS NULL AND profile_source_id IS NULL AND profile_account_id IS NULL)
        OR (profile_source_type IS NOT NULL AND profile_source_id IS NOT NULL AND profile_account_id IS NOT NULL)
    );

ALTER TABLE org_chart ADD COLUMN IF NOT EXISTS parent_reference JSON NULL;
