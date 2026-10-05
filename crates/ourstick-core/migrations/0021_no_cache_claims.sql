-- ADR 0017: the owner keeps no encrypted copies, so it claims no room for a
-- cache any more. Claims an older version left are removed; the `cache`
-- kind stays allowed by the table so a database is never refused, and is
-- never written again. See docs/schema.md.

DELETE FROM space_claims WHERE kind = 'cache';
