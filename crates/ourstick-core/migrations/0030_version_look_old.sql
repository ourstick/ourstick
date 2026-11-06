-- ADR 0033 rule 5: a notice older than one already taken is no good look,
-- and is kept as the failure 'old', which 0029's CHECK did not allow (the
-- look then could not be kept at all). SQLite cannot change a CHECK
-- constraint, so the table is made again with its row. See docs/schema.md.

CREATE TABLE version_look_new (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    looking INTEGER NOT NULL DEFAULT 1 CHECK (looking IN (0, 1)),
    looking_told INTEGER NOT NULL DEFAULT 0 CHECK (looking_told IN (0, 1)),
    next_look_ms INTEGER NOT NULL DEFAULT 0 CHECK (next_look_ms >= 0),
    last_good_ms INTEGER CHECK (last_good_ms >= 0),
    failing_since_ms INTEGER CHECK (failing_since_ms >= 0),
    failure TEXT CHECK (failure IN ('no-connection', 'refused', 'not-the-file', 'old')),
    newest TEXT,
    security TEXT,
    new_release_key TEXT,
    security_told TEXT
);

INSERT INTO version_look_new (id, looking, looking_told, next_look_ms, last_good_ms,
                              failing_since_ms, failure, newest, security,
                              new_release_key, security_told)
    SELECT id, looking, looking_told, next_look_ms, last_good_ms,
           failing_since_ms, failure, newest, security,
           new_release_key, security_told
    FROM version_look;
DROP TABLE version_look;
ALTER TABLE version_look_new RENAME TO version_look;
