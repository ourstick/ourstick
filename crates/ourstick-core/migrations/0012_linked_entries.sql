-- Folders and files shared where they are, not copied onto the stick
-- (ADR 0008 §2): each is a top-level entry of its stick under `name`. Removing
-- a row stops sharing it; the user's files are never touched. See
-- docs/schema.md.

CREATE TABLE linked_entries (
    stick_id    BLOB    NOT NULL REFERENCES owned_sticks (stick_id) CHECK (length(stick_id) = 32),
    -- Its name on the stick: the first component of its manifest paths.
    name        TEXT    NOT NULL CHECK (length(name) > 0 AND instr(name, '/') = 0),
    -- Where it is on this device. Never leaves the device.
    path        TEXT    NOT NULL CHECK (length(path) > 0),
    is_dir      INTEGER NOT NULL CHECK (is_dir IN (0, 1)),
    -- The id of the device (disk) it was on when linked (`st_dev`; NULL where
    -- there is none): if it is on another one now, its disk is gone and an
    -- empty mount point is in its place.
    disk        INTEGER,
    added_at_ms INTEGER NOT NULL CHECK (added_at_ms >= 0),
    PRIMARY KEY (stick_id, name)
) STRICT, WITHOUT ROWID;
