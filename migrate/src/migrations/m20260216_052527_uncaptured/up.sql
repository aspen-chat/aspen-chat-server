-- Your SQL goes here


ALTER TABLE "community" DROP COLUMN "icon_mime_type";
ALTER TABLE "community" DROP COLUMN "icon";
ALTER TABLE "community" ADD COLUMN "icon" UUID;




ALTER TABLE "user" ADD COLUMN "icon" UUID;

CREATE TABLE "other_server_auth_token"(
	"token" TEXT NOT NULL PRIMARY KEY,
	"expires" TIMESTAMP NOT NULL,
	"user" UUID NOT NULL,
	"domain" TEXT NOT NULL,
	FOREIGN KEY ("user") REFERENCES "user"("id")
);

CREATE TABLE "refresh_token"(
	"token" TEXT NOT NULL PRIMARY KEY,
	"expires" TIMESTAMP NOT NULL,
	"user" UUID NOT NULL,
	FOREIGN KEY ("user") REFERENCES "user"("id")
);

CREATE TABLE "icon"(
	"id" UUID NOT NULL PRIMARY KEY,
	"data" BYTEA NOT NULL,
	"icon_mime_type" TEXT NOT NULL
);

CREATE TABLE "session"(
	"token" TEXT NOT NULL PRIMARY KEY,
	"expires" TIMESTAMP NOT NULL,
	"refresh_token" TEXT NOT NULL,
	FOREIGN KEY ("refresh_token") REFERENCES "refresh_token"("token")
);

