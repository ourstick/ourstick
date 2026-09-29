-- What the owner last encrypted, per file, so an unchanged file is not
-- encrypted again on the next publish. See docs/schema.md and "Known limits"
-- in docs/protocol.md: a file counts as unchanged when its size, mtime and
-- ctime are as recorded.

CREATE TABLE owned_files (
    stick_id   BLOB    NOT NULL REFERENCES owned_sticks (stick_id) CHECK (length(stick_id) = 32),
    -- Path relative to the stick root, as in the manifest.
    path       TEXT    NOT NULL,
    size       INTEGER NOT NULL CHECK (size >= 0),
    -- Nanoseconds since the Unix epoch; 0 where the system has no such time.
    mtime_ns   INTEGER NOT NULL CHECK (mtime_ns >= 0),
    ctime_ns   INTEGER NOT NULL CHECK (ctime_ns >= 0),
    -- BLAKE3 of the encrypted blob made from the file in that state.
    blob_hash  BLOB    NOT NULL CHECK (length(blob_hash) = 32),
    PRIMARY KEY (stick_id, path)
) STRICT, WITHOUT ROWID;
