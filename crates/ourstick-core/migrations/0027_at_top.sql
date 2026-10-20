-- ADR 0028, amendment of 2026-10-20: a stick made from exactly one folder
-- lists that folder's contents at its top, without the folder's name in
-- front (1). At most one entry of a stick is at the top. See docs/schema.md.

ALTER TABLE linked_entries
    ADD COLUMN at_top INTEGER NOT NULL DEFAULT 0 CHECK (at_top IN (0, 1));
