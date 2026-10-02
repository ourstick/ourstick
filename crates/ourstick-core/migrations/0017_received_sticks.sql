-- A received (joined) stick belongs to one program (step 7b): `cli` for
-- `ourstick get` (every row before this version), `app` for the receiver
-- window, which `get` never moves to another folder. Its name as the last
-- verified manifest gave it, so lists show it without the network. Whether
-- the user paused it, and whether it is being forgotten (hidden; finished at
-- the next start: its ticket, cache and rows go, its files stay, its record
-- floor stays). See docs/schema.md.

ALTER TABLE joined_sticks ADD COLUMN kind TEXT NOT NULL DEFAULT 'cli'
    CHECK (kind IN ('cli', 'app'));
ALTER TABLE joined_sticks ADD COLUMN name TEXT;
ALTER TABLE joined_sticks ADD COLUMN paused INTEGER NOT NULL DEFAULT 0
    CHECK (paused IN (0, 1));
ALTER TABLE joined_sticks ADD COLUMN forgetting INTEGER NOT NULL DEFAULT 0
    CHECK (forgetting IN (0, 1));
