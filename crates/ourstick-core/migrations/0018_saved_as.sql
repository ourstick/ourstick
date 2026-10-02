-- Where each received file was saved in its folder (ADR 0010): its path in
-- the stick, or the smallest change of it this computer can save. Equal to
-- `path` for every row before this version. No two files of a stick are
-- saved at one place. See docs/schema.md.

ALTER TABLE downloaded_files ADD COLUMN saved_as TEXT NOT NULL DEFAULT '';
UPDATE downloaded_files SET saved_as = path;
CREATE UNIQUE INDEX downloaded_files_saved_as ON downloaded_files (stick_id, saved_as);
