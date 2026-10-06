-- A digest cannot be turned back into its token, so going down ends every sign-in.
DELETE FROM push_subscription;
DELETE FROM session;
DELETE FROM refresh_token;
