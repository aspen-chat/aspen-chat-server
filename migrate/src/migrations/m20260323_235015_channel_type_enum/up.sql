CREATE TYPE channel_type AS ENUM ('voice', 'text');
ALTER TABLE channel ALTER COLUMN ty TYPE channel_type USING CASE WHEN ty = 0 THEN 'text'::channel_type WHEN ty = 1 THEN 'voice'::channel_type END;