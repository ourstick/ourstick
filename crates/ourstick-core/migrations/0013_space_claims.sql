-- Who holds room on which disk (the space ledger, ADR 0008 §1 as amended):
-- a sharer's cache budget (ADR 0007), a download's bytes still to come. A
-- row counts only while its holder's lock is held (`share-<hex>.lock` for
-- a cache, `download-<hex>.lock` for a download); a row whose lock is free
-- is stale and is ignored, and replaced the next time. See docs/schema.md.

CREATE TABLE space_claims (
    stick_id BLOB    NOT NULL CHECK (length(stick_id) = 32),
    kind     TEXT    NOT NULL CHECK (kind IN ('cache', 'download')),
    -- Which disk: `dev:<st_dev>` on Unix, `vol:<volume>` on Windows.
    disk     TEXT    NOT NULL CHECK (length(disk) > 0),
    bytes    INTEGER NOT NULL CHECK (bytes >= 0),
    PRIMARY KEY (stick_id, kind)
) STRICT, WITHOUT ROWID;
