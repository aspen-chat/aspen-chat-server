-- Categories a user has collapsed in their channel list, for themself alone.
CREATE TABLE category_collapse (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    category UUID NOT NULL REFERENCES category (id) ON DELETE CASCADE,
    PRIMARY KEY ("user", category)
);
