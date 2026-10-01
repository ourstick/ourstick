-- Many sticks in the app (ADR 0009, step 7): where a macOS stick's disk
-- image is, whether the user wants a stick shared, a stick being removed,
-- and the ids retired by "Give it a new link". See docs/schema.md.

-- macOS: the disk image of an app stick, recorded once and never derived
-- from its name or id. NULL elsewhere.
ALTER TABLE owned_sticks ADD COLUMN image_path TEXT CHECK (image_path IS NULL OR length(image_path) > 0);
-- 0 after the user's "Stop sharing", so a restart keeps it stopped.
ALTER TABLE owned_sticks ADD COLUMN sharing_wanted INTEGER NOT NULL DEFAULT 1 CHECK (sharing_wanted IN (0, 1));
-- 1 while the stick is being removed: hidden from every read but the one
-- that finishes the removal.
ALTER TABLE owned_sticks ADD COLUMN removing INTEGER NOT NULL DEFAULT 0 CHECK (removing IN (0, 1));

-- Ids replaced by a new link: only their secrets and cache are left to
-- delete. No folder, so nothing of the user's is ever touched for them.
CREATE TABLE retired_sticks (
    stick_id      BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    retired_at_ms INTEGER NOT NULL CHECK (retired_at_ms >= 0)
) STRICT, WITHOUT ROWID;
