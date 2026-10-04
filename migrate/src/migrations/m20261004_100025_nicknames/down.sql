ALTER TABLE report DROP COLUMN nickname;
DELETE FROM report_case WHERE kind = 'nickname';
DROP INDEX report_case_unresolved_nickname;
ALTER TABLE report_case
    DROP CONSTRAINT report_case_nickname_kind,
    DROP COLUMN community,
    DROP CONSTRAINT report_case_kind_check,
    ADD CONSTRAINT report_case_kind_check CHECK (kind IN ('message', 'profile'));

UPDATE community_role SET permissions = permissions & ~((1::BIGINT << 14) | (1::BIGINT << 15));

ALTER TABLE community_user DROP COLUMN nickname;
