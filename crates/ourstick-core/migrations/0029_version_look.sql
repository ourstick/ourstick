-- ADR 0033: what the app keeps of its looks for a new version, and the
-- highest serial taken per key that signed a notice. At most one row of
-- `version_look`; no row means the defaults (looking, nothing known). A
-- device set up before this version is told once that the app now looks
-- (looking_told 0); a fresh one is not (it learns it with everything else).
-- See docs/schema.md.

CREATE TABLE version_look (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    looking INTEGER NOT NULL DEFAULT 1 CHECK (looking IN (0, 1)),
    looking_told INTEGER NOT NULL DEFAULT 0 CHECK (looking_told IN (0, 1)),
    next_look_ms INTEGER NOT NULL DEFAULT 0 CHECK (next_look_ms >= 0),
    last_good_ms INTEGER CHECK (last_good_ms >= 0),
    failing_since_ms INTEGER CHECK (failing_since_ms >= 0),
    failure TEXT CHECK (failure IN ('no-connection', 'refused', 'not-the-file')),
    newest TEXT,
    security TEXT,
    new_release_key TEXT,
    security_told TEXT
);

CREATE TABLE notice_serials (
    key_id TEXT PRIMARY KEY,
    serial INTEGER NOT NULL CHECK (serial >= 0)
);

INSERT INTO version_look (id, looking_told)
    SELECT 1, 1 WHERE NOT EXISTS (SELECT 1 FROM device);
