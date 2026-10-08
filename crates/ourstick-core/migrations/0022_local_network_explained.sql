-- ADR 0020: whether the app showed, on macOS, the step that explains the
-- Local Network permission before macOS asks. Only that the step was
-- shown, never the permission's state (macOS keeps it). See docs/schema.md.

ALTER TABLE device ADD COLUMN local_network_explained INTEGER NOT NULL DEFAULT 0
    CHECK (local_network_explained IN (0, 1));
