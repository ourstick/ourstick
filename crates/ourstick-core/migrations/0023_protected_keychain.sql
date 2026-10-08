-- ADR 0021: on macOS the app keeps its secrets in the data protection
-- keychain: secret_backend may now be 'protected-keychain'. SQLite cannot
-- change a CHECK constraint, so the table is made again with the same rows;
-- old_keys_left says the login keychain still holds the copies moved from it,
-- deleted at a later start. See docs/schema.md.

CREATE TABLE device_new (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    node_id         BLOB    CHECK (node_id IS NULL OR length(node_id) = 32),
    -- Where this device's secrets live: the system keyring ('secret-service'
    -- on Linux, 'keychain' or 'protected-keychain' on macOS,
    -- 'credential-manager' on Windows) or a key file.
    secret_backend  TEXT    NOT NULL
        CHECK (secret_backend IN
            ('secret-service', 'keychain', 'protected-keychain', 'credential-manager', 'key-file')),
    -- Path of the key file, only for the 'key-file' backend.
    key_file_path   TEXT    CHECK ((secret_backend = 'key-file') = (key_file_path IS NOT NULL)),
    created_at_ms   INTEGER NOT NULL CHECK (created_at_ms >= 0),
    receiver_name   TEXT    CHECK (length(receiver_name) > 0),
    local_network_explained INTEGER NOT NULL DEFAULT 0
        CHECK (local_network_explained IN (0, 1)),
    old_keys_left   INTEGER NOT NULL DEFAULT 0
        CHECK (old_keys_left IN (0, 1) AND (old_keys_left = 0 OR secret_backend = 'protected-keychain'))
) STRICT;

INSERT INTO device_new (id, node_id, secret_backend, key_file_path, created_at_ms,
                        receiver_name, local_network_explained)
    SELECT id, node_id, secret_backend, key_file_path, created_at_ms,
           receiver_name, local_network_explained
    FROM device;
DROP TABLE device;
ALTER TABLE device_new RENAME TO device;
