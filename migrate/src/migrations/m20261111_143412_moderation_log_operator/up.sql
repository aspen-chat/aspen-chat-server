-- Whether an entry was written by an operator command from the terminal
-- (`app::moderation_log::log_operator_moderation`), which no account takes. Its `actor` is
-- null, as is that of an entry whose actor's row has since been deleted (`ON DELETE SET
-- NULL`), so the column is what tells the two apart. The default is for a server of the
-- version before this column, running beside this one during an upgrade.
ALTER TABLE moderation_log ADD COLUMN by_operator BOOLEAN NOT NULL DEFAULT FALSE;

-- Until this column, purging evidence was the only action logged from the terminal, and
-- nothing else wrote it.
UPDATE moderation_log SET by_operator = TRUE WHERE actor IS NULL AND action = 'purgeAttachment';

ALTER TABLE moderation_log
    ADD CONSTRAINT moderation_log_operator_has_no_actor CHECK (NOT by_operator OR actor IS NULL);
