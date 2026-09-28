-- Initial schema. See docs/schema.md for what each column means.
-- No secrets are stored here: keys live in the secret store, under names
-- derived from the StickId (docs/protocol.md, "Key storage").

-- This device. At most one row.
CREATE TABLE device (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    node_id         BLOB    NOT NULL CHECK (length(node_id) = 32),
    -- Where this device's secrets live: 'secret-service' or 'key-file'.
    secret_backend  TEXT    NOT NULL CHECK (secret_backend IN ('secret-service', 'key-file')),
    -- Path of the key file, only for the 'key-file' backend.
    key_file_path   TEXT    CHECK ((secret_backend = 'key-file') = (key_file_path IS NOT NULL)),
    created_at_ms   INTEGER NOT NULL CHECK (created_at_ms >= 0)
) STRICT;

-- Sticks this device owns and shares.
CREATE TABLE owned_sticks (
    stick_id         BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    name             TEXT    NOT NULL,
    root_path        TEXT    NOT NULL,
    cipher           INTEGER NOT NULL CHECK (cipher BETWEEN 0 AND 255),
    -- Highest seq ever signed for this stick. Only ever increases.
    last_signed_seq  INTEGER NOT NULL DEFAULT 0 CHECK (last_signed_seq >= 0),
    -- The latest signed manifest record (137 bytes), served to readers.
    latest_record    BLOB    CHECK (latest_record IS NULL OR length(latest_record) = 137),
    created_at_ms    INTEGER NOT NULL CHECK (created_at_ms >= 0)
) STRICT;

-- Sticks this device reads, from a ticket.
CREATE TABLE joined_sticks (
    stick_id               BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    output_dir             TEXT    NOT NULL,
    -- Highest seq accepted from a signed record; NULL until the first one.
    -- Only ever increases.
    highest_seq            INTEGER CHECK (highest_seq IS NULL OR highest_seq >= 0),
    -- Manifest hash of that record.
    current_manifest_hash  BLOB    CHECK (current_manifest_hash IS NULL OR length(current_manifest_hash) = 32),
    joined_at_ms           INTEGER NOT NULL CHECK (joined_at_ms >= 0),
    CHECK ((highest_seq IS NULL) = (current_manifest_hash IS NULL))
) STRICT;
