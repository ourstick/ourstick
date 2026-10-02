-- What the user chose not to get of a received stick (ADR 0012): its
-- top-level names, byte-exact as in the stick's manifest. What is not here
-- comes, including anything the sender adds later. Only for the window's
-- sticks; `get` never reads it. See docs/schema.md.

CREATE TABLE joined_left_out (
    stick_id BLOB NOT NULL REFERENCES joined_sticks (stick_id) CHECK (length(stick_id) = 32),
    name     TEXT NOT NULL CHECK (length(name) > 0),
    PRIMARY KEY (stick_id, name)
) STRICT, WITHOUT ROWID;
