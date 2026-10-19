-- ADR 0028: a stick made from now on has no folder of its own, only what
-- is sent from where it is (linked entries); every stick made before keeps
-- its folder and its drive (1). And the window said once, at its first
-- close, that OurStick is still open. See docs/schema.md.

ALTER TABLE owned_sticks
    ADD COLUMN has_own_folder INTEGER NOT NULL DEFAULT 1 CHECK (has_own_folder IN (0, 1));

ALTER TABLE settings
    ADD COLUMN still_open_told INTEGER NOT NULL DEFAULT 0 CHECK (still_open_told IN (0, 1));
