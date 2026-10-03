-- Who has it (ADR 0015). The owner keeps the latest report of each reader
-- of its sticks; the reader keeps the counter of its last report, and this
-- device the name its receiver chose to show senders. See docs/schema.md.

CREATE TABLE reader_reports (
    stick_id    BLOB    NOT NULL REFERENCES owned_sticks (stick_id) CHECK (length(stick_id) = 32),
    reader_id   BLOB    NOT NULL CHECK (length(reader_id) = 32),
    counter     INTEGER NOT NULL CHECK (counter >= 0),
    seq         INTEGER NOT NULL CHECK (seq >= 0),
    state       TEXT    NOT NULL CHECK (state IN ('downloading', 'up_to_date', 'waiting', 'paused', 'forgotten')),
    -- Only while downloading.
    percent     INTEGER CHECK ((state = 'downloading') = (percent IS NOT NULL) AND percent BETWEEN 0 AND 99),
    -- Only while waiting.
    reason      TEXT    CHECK ((state = 'waiting') = (reason IS NOT NULL)
                               AND reason IN ('sender_away', 'no_room', 'folder_missing', 'other')),
    everything  INTEGER NOT NULL CHECK (everything IN (0, 1)),
    kind        TEXT    NOT NULL CHECK (kind IN ('macos', 'windows', 'linux', 'other')),
    name        TEXT    CHECK (length(name) > 0),
    first_ms    INTEGER NOT NULL CHECK (first_ms >= 0),
    received_ms INTEGER NOT NULL CHECK (received_ms >= first_ms),
    -- Off the sender's list (removed, cleared, or forgotten by the reader);
    -- kept for its counter, so an old report sent again changes nothing.
    hidden      INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    PRIMARY KEY (stick_id, reader_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX reader_reports_by_time ON reader_reports (stick_id, received_ms);

ALTER TABLE device ADD COLUMN receiver_name TEXT CHECK (length(receiver_name) > 0);

ALTER TABLE joined_sticks ADD COLUMN report_counter INTEGER NOT NULL DEFAULT 0 CHECK (report_counter >= 0);
