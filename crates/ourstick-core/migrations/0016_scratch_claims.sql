-- A download also holds the room for its batches (`scratch`) on the data
-- folder's disk, as its own row, so that several downloads at once each hold
-- theirs (ADR 0008's notes). It counts while the stick's download lock is
-- held, like `download`. SQLite cannot change a CHECK constraint: the table
-- is made again with the same rows. See docs/schema.md.

CREATE TABLE space_claims_new (
    stick_id BLOB    NOT NULL CHECK (length(stick_id) = 32),
    kind     TEXT    NOT NULL CHECK (kind IN ('cache', 'download', 'scratch')),
    -- Which disk: `dev:<st_dev>` on Unix, `vol:<volume>` on Windows.
    disk     TEXT    NOT NULL CHECK (length(disk) > 0),
    bytes    INTEGER NOT NULL CHECK (bytes >= 0),
    PRIMARY KEY (stick_id, kind)
) STRICT, WITHOUT ROWID;

INSERT INTO space_claims_new (stick_id, kind, disk, bytes)
    SELECT stick_id, kind, disk, bytes FROM space_claims;

DROP TABLE space_claims;
ALTER TABLE space_claims_new RENAME TO space_claims;
