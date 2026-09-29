-- macOS keeps a device's secrets in its keychain: secret_backend may now be
-- 'keychain'. SQLite cannot change a CHECK constraint, so the table is made
-- again with the same rows. See docs/schema.md.

CREATE TABLE device_new (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    node_id         BLOB    NOT NULL CHECK (length(node_id) = 32),
    -- Where this device's secrets live: the system keyring ('secret-service'
    -- on Linux, 'keychain' on macOS) or a key file.
    secret_backend  TEXT    NOT NULL
        CHECK (secret_backend IN ('secret-service', 'keychain', 'key-file')),
    -- Path of the key file, only for the 'key-file' backend.
    key_file_path   TEXT    CHECK ((secret_backend = 'key-file') = (key_file_path IS NOT NULL)),
    created_at_ms   INTEGER NOT NULL CHECK (created_at_ms >= 0)
) STRICT;

INSERT INTO device_new SELECT * FROM device;
DROP TABLE device;
ALTER TABLE device_new RENAME TO device;
