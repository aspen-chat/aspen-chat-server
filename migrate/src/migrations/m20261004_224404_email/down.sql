UPDATE deployment_role SET permissions = permissions & ~(1::BIGINT << 14);
DROP TABLE newsletter_post;
DROP TABLE email_outbox;
DROP TABLE user_email;
ALTER TABLE "user" DROP COLUMN public_email;
ALTER TABLE deployment_settings
    DROP COLUMN email_required,
    DROP COLUMN email_verification_required,
    DROP COLUMN newsletter_enabled;
