-- The app's stick is a drive of a chosen size (docs/gui-spec.md). NULL for a
-- plain shared folder (the CLI). See docs/schema.md.

ALTER TABLE owned_sticks ADD COLUMN drive_size INTEGER
    CHECK (drive_size IS NULL OR drive_size > 0);
