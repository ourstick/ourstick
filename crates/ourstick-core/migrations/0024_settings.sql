-- ADR 0024: the app's choices on this device, held before the device is set
-- up (a fresh install has no `device` row until its first key). At most one
-- row; no row means the defaults: start at login, not yet told. See
-- docs/schema.md.

CREATE TABLE settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    start_at_login INTEGER NOT NULL DEFAULT 1 CHECK (start_at_login IN (0, 1)),
    start_at_login_told INTEGER NOT NULL DEFAULT 0 CHECK (start_at_login_told IN (0, 1))
);
