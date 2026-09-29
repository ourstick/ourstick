-- Windows shows the app's stick as a drive letter (subst). The letter it got
-- is kept, so that the stick is at the same letter after a restart. NULL
-- elsewhere and for a plain shared folder. See docs/schema.md.

ALTER TABLE owned_sticks ADD COLUMN drive_letter TEXT
    CHECK (drive_letter IS NULL
        OR (length(drive_letter) = 1 AND drive_letter BETWEEN 'A' AND 'Z'));
