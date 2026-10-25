-- A provider's addresses become accounts' verified addresses only when its administrators say so:
-- one that marks any address verified would otherwise have digests and newsletters mailed to
-- addresses its users merely typed. Providers already added keep what was chosen for them.
ALTER TABLE identity_provider ALTER COLUMN trust_email SET DEFAULT false;
