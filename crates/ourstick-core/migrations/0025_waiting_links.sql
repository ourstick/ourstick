-- ADR 0025: sharing links the app waits to receive, pasted while their
-- sender was away. A stick is either waiting here or received
-- (joined_sticks), never both: adding it as received ends its wait in the
-- same step. Its ticket is in the secret store, never here. See
-- docs/schema.md.

CREATE TABLE waiting_links (
    stick_id    BLOB    PRIMARY KEY CHECK (length(stick_id) = 32),
    added_at_ms INTEGER NOT NULL CHECK (added_at_ms >= 0)
);
