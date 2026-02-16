-- Your SQL goes here

ALTER TABLE "category" ADD COLUMN sort_index INTEGER NOT NULL;
ALTER TABLE "channel" ADD COLUMN sort_index INTEGER NOT NULL;