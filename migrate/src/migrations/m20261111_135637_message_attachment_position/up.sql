-- Where each attachment stands among its message's, in the order its author listed them
-- (`app::message::attachments_of`): reads order by it, then by the attachment's id. The order
-- the rows already here were listed in was never stored, so they take the order of their ids.
-- The default is for a server of the version before this column, running beside this one
-- during an upgrade, whose rows then read in the order of their ids too.
ALTER TABLE message_attachment ADD COLUMN position INTEGER NOT NULL DEFAULT 0;

UPDATE message_attachment held
SET position = ranked.position
FROM (
    SELECT message_id, attachment_id,
           row_number() OVER (PARTITION BY message_id ORDER BY attachment_id) - 1 AS position
    FROM message_attachment
) ranked
WHERE held.message_id = ranked.message_id
  AND held.attachment_id = ranked.attachment_id
  AND ranked.position > 0;
