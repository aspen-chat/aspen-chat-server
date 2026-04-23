-- Your SQL goes here
CREATE TABLE "attachment"(
    "id" UUID NOT NULL PRIMARY KEY,
    "mime_type" TEXT NOT NULL,
    "file_name" TEXT NOT NULL
);