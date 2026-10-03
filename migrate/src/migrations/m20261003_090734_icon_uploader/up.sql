-- Who uploaded an icon, who alone may delete it while nothing uses it; `NULL` for one uploaded
-- before this was recorded, or by an account since deleted, which no one may delete.
ALTER TABLE icon ADD COLUMN uploaded_by UUID REFERENCES "user" (id) ON DELETE SET NULL;
