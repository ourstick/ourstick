-- What `get` wrote into a joined stick's folder, per file, so the next `get`
-- skips files that are already there and never overwrites a file changed
-- locally. See docs/schema.md.

CREATE TABLE downloaded_files (
    stick_id   BLOB    NOT NULL REFERENCES joined_sticks (stick_id) CHECK (length(stick_id) = 32),
    -- Path relative to the folder, as in the manifest.
    path       TEXT    NOT NULL,
    -- The file on disk right after `get` wrote it.
    size       INTEGER NOT NULL CHECK (size >= 0),
    mtime_ns   INTEGER NOT NULL CHECK (mtime_ns >= 0),
    ctime_ns   INTEGER NOT NULL CHECK (ctime_ns >= 0),
    -- The blob it was decrypted from.
    blob_hash  BLOB    NOT NULL CHECK (length(blob_hash) = 32),
    PRIMARY KEY (stick_id, path)
) STRICT, WITHOUT ROWID;
