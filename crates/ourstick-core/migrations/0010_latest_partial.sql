-- ADR 0007: whether the latest signed record is of a partial manifest (the
-- owner was still reading its folder). A restarted owner goes on publishing
-- partial versions until the first complete one. See docs/schema.md.

ALTER TABLE owned_sticks ADD COLUMN latest_partial INTEGER NOT NULL DEFAULT 0
    CHECK (latest_partial IN (0, 1));
