-- ADR 0007: the owner's catalogue records the plaintext hash of each file too
-- (the nonce prefix of ADR 0003). A hint only: every blob is still made by a
-- full encryption. NULL for rows written before. See docs/schema.md.

ALTER TABLE owned_files ADD COLUMN plaintext_hash BLOB
    CHECK (plaintext_hash IS NULL OR length(plaintext_hash) = 32);
