-- This file should undo anything in `up.sql`.
-- Drop in child-first order so foreign-key dependents go before their
-- parents: community_user / category / channel all reference user and/or
-- community, so user and community come last.
DROP TABLE IF EXISTS "react";
DROP TABLE IF EXISTS "message";
DROP TABLE IF EXISTS "channel";
DROP TABLE IF EXISTS "category";
DROP TABLE IF EXISTS "community_user";
DROP TABLE IF EXISTS "user";
DROP TABLE IF EXISTS "community";
