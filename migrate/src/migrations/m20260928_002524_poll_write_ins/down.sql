DROP INDEX poll_option_one_write_in;
DELETE FROM poll_option WHERE write_in;
ALTER TABLE poll_option
    DROP COLUMN removed_at,
    DROP COLUMN written_by,
    DROP COLUMN write_in;
ALTER TABLE poll DROP COLUMN allow_write_ins;
