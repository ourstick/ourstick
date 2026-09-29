-- The seq floor of ADR 0004 moves out of joined_sticks into its own table, so
-- `ls` can raise it without touching any folder state. See docs/schema.md.

-- Highest verified record per stick: the `last_seen` of ADR 0004. Only ever
-- increases. Written by every command that verifies a signed record.
CREATE TABLE record_floors (
    stick_id       BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    highest_seq    INTEGER NOT NULL CHECK (highest_seq >= 0),
    manifest_hash  BLOB    NOT NULL CHECK (length(manifest_hash) = 32)
) STRICT;

INSERT INTO record_floors (stick_id, highest_seq, manifest_hash)
    SELECT stick_id, highest_seq, current_manifest_hash
    FROM joined_sticks
    WHERE highest_seq IS NOT NULL;

-- joined_sticks now says only what its folder holds. The old columns were set
-- before the files arrived, so they say nothing about the folder: completed
-- starts out NULL for every stick.
CREATE TABLE joined_sticks_new (
    stick_id                 BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    -- The folder of the latest `get`. Another folder replaces it.
    output_dir               TEXT    NOT NULL,
    -- The record whose files are all in output_dir; NULL until a `get` finishes.
    completed_seq            INTEGER CHECK (completed_seq IS NULL OR completed_seq >= 0),
    completed_manifest_hash  BLOB    CHECK (completed_manifest_hash IS NULL OR length(completed_manifest_hash) = 32),
    joined_at_ms             INTEGER NOT NULL CHECK (joined_at_ms >= 0),
    CHECK ((completed_seq IS NULL) = (completed_manifest_hash IS NULL))
) STRICT;

INSERT INTO joined_sticks_new (stick_id, output_dir, joined_at_ms)
    SELECT stick_id, output_dir, joined_at_ms FROM joined_sticks;

DROP TABLE joined_sticks;
ALTER TABLE joined_sticks_new RENAME TO joined_sticks;
