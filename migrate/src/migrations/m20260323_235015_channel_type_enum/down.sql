ALTER TABLE channel ALTER COLUMN ty TYPE integer USING CASE WHEN ty = 'text'::channel_type THEN 0 ELSE 1 END;
DROP TYPE channel_type;