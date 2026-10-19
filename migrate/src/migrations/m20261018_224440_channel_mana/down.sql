UPDATE community_role SET permissions = permissions & ~140737488355328;
UPDATE channel_override SET allow = allow & ~140737488355328, deny = deny & ~140737488355328;
UPDATE category_override SET allow = allow & ~140737488355328, deny = deny & ~140737488355328;

DROP INDEX held_message_target;
DROP INDEX held_message_mana_queue;
DELETE FROM held_message WHERE waiting = 'mana';
ALTER TABLE held_message
    DROP CONSTRAINT held_message_waiting_kind,
    DROP CONSTRAINT held_message_kind_payload,
    DROP CONSTRAINT held_message_kind_target,
    DROP COLUMN credit,
    DROP COLUMN cost,
    DROP COLUMN payload,
    DROP COLUMN target,
    DROP COLUMN kind,
    DROP COLUMN waiting;

ALTER TABLE channel DROP COLUMN mana_max, DROP COLUMN mana_recharge;

ALTER TABLE deployment_settings
    DROP COLUMN mana_attachment_cost,
    DROP COLUMN mana_max,
    DROP COLUMN mana_max_recharge;
