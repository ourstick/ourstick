-- A stick has no fixed size any more (ADR 0008): what can still be copied
-- onto it is what its disk has room for (the space ledger). The app's stick
-- is told by `kind` (version 11). See docs/schema.md.

ALTER TABLE owned_sticks DROP COLUMN drive_size;
