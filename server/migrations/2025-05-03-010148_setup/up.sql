-- Your SQL goes here
CREATE TABLE "user"(
	"id" UUID NOT NULL PRIMARY KEY,
	"name" TEXT NOT NULL
);

CREATE TABLE "community"(
	"id" UUID NOT NULL PRIMARY KEY,
	"name" TEXT NOT NULL,
	"icon" BYTEA,
	"icon_mime_type" TEXT
);

CREATE TABLE "community_user"(
	"user" UUID NOT NULL,
	"community" UUID NOT NULL,
	PRIMARY KEY ("user", "community"),
	FOREIGN KEY ("community") REFERENCES "community"("id"),
	FOREIGN KEY ("user") REFERENCES "user"("id")
);

CREATE TABLE "category"(
	"id" UUID NOT NULL PRIMARY KEY,
	"community" UUID NOT NULL,
	"name" TEXT NOT NULL,
	FOREIGN KEY ("community") REFERENCES "community"("id")
);

CREATE TABLE "channel"(
	"id" UUID NOT NULL PRIMARY KEY,
	"community" UUID,
	"parent_category" UUID,
	"name" TEXT NOT NULL,
	"ty" INTEGER NOT NULL,
	FOREIGN KEY ("community") REFERENCES "community"("id"),
	FOREIGN KEY ("parent_category") REFERENCES "category"("id")
);

CREATE TABLE "message"(
	"id" UUID NOT NULL PRIMARY KEY,
	"author" UUID NOT NULL,
	"channel" UUID NOT NULL,
	FOREIGN KEY ("author") REFERENCES "user"("id"),
	FOREIGN KEY ("channel") REFERENCES "channel"("id")
);

CREATE TABLE "react"(
	"emoji" TEXT NOT NULL,
	"author" UUID NOT NULL,
	"message" UUID NOT NULL,
	PRIMARY KEY("emoji", "author", "message"),
	FOREIGN KEY ("author") REFERENCES "user"("id"),
	FOREIGN KEY ("message") REFERENCES "message"("id")
);

