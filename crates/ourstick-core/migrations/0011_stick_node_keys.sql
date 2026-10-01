-- ADR 0009: a node key per stick made from now on, and which program lists a
-- stick. See docs/schema.md.

-- The node id of the stick's own node key; NULL for a stick made before this,
-- which keeps the device's node key (its links carry it).
ALTER TABLE owned_sticks ADD COLUMN node_id BLOB
    CHECK (node_id IS NULL OR length(node_id) = 32);
CREATE UNIQUE INDEX owned_sticks_node_id ON owned_sticks (node_id)
    WHERE node_id IS NOT NULL;

-- 'app' for a stick the app made (a drive: it has a size), 'cli' for one the
-- command line shares. Code always sets it.
ALTER TABLE owned_sticks ADD COLUMN kind TEXT NOT NULL DEFAULT 'cli'
    CHECK (kind IN ('app', 'cli'));
UPDATE owned_sticks SET kind = 'app' WHERE drive_size IS NOT NULL;

-- A device made from now on has no node key of its own: device.node_id may be
-- NULL. An existing device keeps its node id and key (CLAUDE.md, rule 9).
-- SQLite cannot drop NOT NULL, so the table is made again with the same rows.
CREATE TABLE device_new (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    node_id         BLOB    CHECK (node_id IS NULL OR length(node_id) = 32),
    -- Where this device's secrets live: the system keyring ('secret-service'
    -- on Linux, 'keychain' on macOS, 'credential-manager' on Windows) or a
    -- key file.
    secret_backend  TEXT    NOT NULL
        CHECK (secret_backend IN
            ('secret-service', 'keychain', 'credential-manager', 'key-file')),
    -- Path of the key file, only for the 'key-file' backend.
    key_file_path   TEXT    CHECK ((secret_backend = 'key-file') = (key_file_path IS NOT NULL)),
    created_at_ms   INTEGER NOT NULL CHECK (created_at_ms >= 0)
) STRICT;

INSERT INTO device_new (id, node_id, secret_backend, key_file_path, created_at_ms)
    SELECT id, node_id, secret_backend, key_file_path, created_at_ms FROM device;
DROP TABLE device;
ALTER TABLE device_new RENAME TO device;
